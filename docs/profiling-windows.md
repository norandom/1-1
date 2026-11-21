# Profiling 1-1 on Windows

How to measure 1-1's CPU on Windows, so that a measurement can be repeated (spec `rdp-native-renderer`, requirement 6.1). The Linux equivalent is [Profiling on Linux](profiling-linux.md).

## CPU per second (no admin)

`scripts/windows/Measure-AppCpu.ps1` samples the running 1-1 once per second and prints one row per second plus averages and maxima, in percent of one core:

| Column | What it is |
| --- | --- |
| `main` | 1-1's main thread (window, input, WebView2 controller, IPC requests) |
| `app` | the whole 1-1 process, main thread included (network, decoding, folder share) |
| `renderer` | WebView2's renderer process (the page, the canvas) |
| `gpu` | WebView2's GPU process |
| `browser`, `other` | WebView2's browser process and its utility processes; in the CSV only |
| `webview` | all WebView2 processes |
| `whole` | `app` plus `webview` |

```powershell
pwsh -NoProfile -File scripts/windows/Measure-AppCpu.ps1 -Seconds 30 -Name drag -OutDirectory D:/BuildArtifacts/drift-rdp/profiles
```

- Exactly one 1-1 must be running. A preview build runs as `1-1.exe` from `D:/BuildArtifacts/drift-rdp/releases/...`; `-ProcessName` picks another name.
- Windows counts processor time in 15.6 ms steps, so a single second of one thread reads as 0, 1.6, 3.1 … percent. Use the 30-second averages; single seconds are noise below about 5%.
- WebView2 processes are found as 1-1's descendants and classified by their `--type=` argument.

## Call stacks and GPU (admin)

When the sampler shows where the time goes but not why, record with Windows Performance Recorder from an elevated terminal:

```powershell
wpr -start CPU -start GPU -filemode
# run the scenario for 30 s
wpr -stop D:\BuildArtifacts\drift-rdp\profiles\drag.etl
```

Open the file in Windows Performance Analyzer (`wpa.exe`, Windows Performance Toolkit): *CPU Usage (Sampled)* grouped by process, thread and stack; *GPU Utilization* for the WebView2 GPU process. Release builds carry no debug symbols (Cargo's default), so 1-1's own frames show as module offsets; build with `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` for a symbolized run, and point WPA's symbol path at the target directory.

## Scenarios

The same four as on Linux (requirement 6.2), 30 s each, with the session visible and the window focused:

1. **Idle:** remote screen static, no input.
2. **Scrolling terminal:** `Get-ChildItem C:\Windows -Recurse` on a Windows host, `find /` on xrdp.
3. **Typing with echo:** steady typing into a remote terminal or editor.
4. **Full-screen update:** dragging a window around the remote desktop continuously, or a video.

## 21 November 2026, first measurement

Machine: the maintainer's Windows 11 workstation (i9, HiDPI display), preview build of `d25ea54`, canvas path (Windows has no native layer). Idle against the xrdp host (910×736); the other three against 10.0.0.62 (1474×1046).

| Scenario (30 s) | Main thread | 1-1 process | WebView2 renderer | WebView2 browser | WebView2 GPU | Whole app, average (peak) |
| --- | --- | --- | --- | --- | --- | --- |
| Idle | 0.2% | 0.5% | 1.4% | – | 0.1% | 2.1% (9%) |
| Scrolling terminal | 0.8% | 1.4% | 3.1% | – | 0.5% | 6.4% (22%) |
| Typing with echo | 5.2% | 7.9% | 8.8% | 6.9% | 3.0% | 27.9% (73%) |
| Window drag | 14.3% | 23.0% | 20.2% | 19.0% | 3.2% | 68.4% (129%) |

Reading:

- **No endless animation:** idle stays near zero (requirement 5.1).
- **Drawing is cheap; moving frames is not.** WebView2's GPU process stays at about 3%. The browser process and 1-1's main thread together cost as much as the renderer. Frames reach the page through a Tauri `ipc::Channel`, whose large messages the page fetches back over the IPC custom protocol; on Windows every fetch is a WebView2 resource request served by the browser process and answered on 1-1's main thread. Key events take the same IPC route, one call per key down, key up or typed character.
- **Against the Linux native layer** (about 6% of one core with the remote screen changing, [validation](../.kiro/specs/rdp-native-renderer/validation.md)), typing and the drag cost well over twice as much, so requirement 5.3's rule is met for Windows. The cost sits mostly in the transport, not in drawing.
