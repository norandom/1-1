# 1-1 maintainer handover

**17 November 2026.** 1-1 (pronounced “zero”) is an RDP client for coding and monitoring remote agents. The maintainer reported a successful manual test of the Windows x64 preview built from `98b6081`. The priorities are legible text while typing and resizing, responsive input, and clipboard recovery during long sessions.

## Where to resume

1. **Finish the Windows-client smart resize check (11.4).** Open the native app, connect to the Windows workstation and FreeRDP targets in turn, shrink the window, then restore it. Record the remote desktop dimensions and a fresh desktop update after each change. All three lab targets passed connection, first frame and shutdown. The automated probe confirmed both resize steps on IronRDP. It timed out waiting for confirmation on Windows and FreeRDP. One earlier native Windows run confirmed a single resize.
2. **Check clipboard recovery on those sessions.** Copy text and a screenshot in both directions after resizing. Use **Resync clipboard** in the status bar, reconnect and repeat. Record which direction, format and recovery step succeeded. Keep clipboard contents and credentials out of test reports. End Computer Use after each native check.
3. **Measure active and unattended bandwidth separately.** For active coding, compare XCRUSH with bulk compression off using the same host, resolution and repeatable scrolling workload. Reconnect between runs, let the initial desktop settle, then measure the change in the Performance HUD’s received and sent totals. For unattended agent output, hold compression fixed and compare idle preview on and off. Record first-update latency and text clarity after wake and resize. [Bandwidth research and Windows measurements](specs/windows-clearcodec-royalts-e2e/bandwidth-research.md)

## Spec status

Requirements, design and tasks are approved. Groups 1–10 and tasks 11.0d–11.0p are checked off. **62 of 77 numbered subtasks are complete**. A separate unnumbered bandwidth measurement is open. The tracked task list contains acceptance criteria. The full requirements and design documents were available in the maintainer’s local checkout at this checkpoint. [Spec metadata](specs/windows-clearcodec-royalts-e2e/spec.json) · [Task list](specs/windows-clearcodec-royalts-e2e/tasks.md)

| Open tasks | Work remaining |
| --- | --- |
| 11.0a–c | Build the shared end-to-end test harness, starting with the Windows adapter. Current runtime testing scope is Windows. |
| 11.1–3 | Exercise library migration and recovery, tree and keyboard navigation, sessions and settings in the built app. |
| 11.4–6 | Complete Windows workstation, Windows Server and GNOME Remote Desktop interoperability checks. The three lab targets are Windows workstation, FreeRDP and IronRDP. |
| 11.7–12 | Check text at 100%, 125%, 150% and 200% scaling, verify performance and memory budgets, and collect final release evidence. |

## Preview and release

The published alpha.3 Windows x64 installer includes the updater, professional graphics settings and text/image clipboard controls. The newer standalone adds measured TCP receive/send totals, observed graphics codecs and opt-in per-host adaptive idle preview. The four platform builds for alpha.3 succeeded. A full updater download, install and restart still needs a later published version. [Published installer](https://github.com/norandom/1-1/releases/tag/v0.1.0-alpha.3) · [Platform build run](https://github.com/norandom/1-1/actions/runs/44143737759)

The standalone passed 365 Windows Rust tests with five skipped, and 561 frontend tests. The quiet Windows desktop received 275 bytes in one live minute and 33,494 bytes in a forced preview minute at 1280 × 720. The first graphics update after a separate wake arrived in 84 ms. This result explains why the adaptive policy leaves a quiet editor live. The manual pass supplied no comparative traffic totals for active coding.

On the development workstation, the tested executable is `D:/BuildArtifacts/drift-rdp/releases/idle-preview-20261117/1-1-idle-preview.exe`. Windows test logs are under `D:/BuildArtifacts/drift-rdp/idle-preview/`. Run native PowerShell from the repository and reuse the configured Cargo target directory. For occasional cleanup while builds are idle, the project storage script provides `-Action CleanProject`. [Baseline runner](../scripts/Invoke-Baseline.ps1) · [Storage script](../scripts/Invoke-BuildStorage.ps1)
