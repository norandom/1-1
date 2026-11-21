# Native renderer validation

## 21 November 2026: first sessions on the native path (Linux)

Machine: the maintainer's workstation (Intel UHD 630, Mesa 26.0.8, Vulkan; Hyprland, scale 1.6, fractional scaling offered), one session to a Windows host (10.0.0.56, 1628×1073). Build from master after `e158619` and `5b3fafc`, started with `ONE_ONE_RENDERER=native`. Measured with `scripts/linux/profile.sh` (CPU per second from `/proc`; perf at 299 Hz, straight to disk).

The maintainer confirmed that the picture, input and HUD work, and that quitting no longer leaves a WebKit crash report.

| Scenario (15 s) | Main thread | Web process | Whole app incl. web | Full-window repaint |
| --- | --- | --- | --- | --- |
| Moving the mouse over a static remote screen | 0–1% per second | 2–11% | – | none |
| Remote screen changing, mouse still | about 1.5% (0–1% most seconds, 8–14% at the start) | about 2% | about 6% of one core | none (`cairo_fill`/`pixman_fill` absent) |

For comparison, the canvas path on 21 November in normal use: main thread 15% average, 26% in busy seconds, peaks 66%, with `cairo_fill` → `pixman_fill` 64% of main-thread time ([profiling notes](../../../docs/profiling-linux.md)).

Requirement 3.2 (main thread at most 5% during remote updates) holds in this run. A first 50-second run of normal use (connect, tabs, typing) averaged 10% on the main thread: page changes such as opening a session repaint the window as before, which is expected and outside 3.2.

The four-scenario reference measurement (task 7.3) was skipped by the maintainer's decision on 21 November 2026; these runs are the evidence for turning the native layer on automatically on Hyprland.

## 21 November 2026: Windows, task 9.1 and the 9.2 measurement

Machine: the maintainer's Windows 11 workstation, preview build of `d25ea54`, canvas path. Method and full table: [Profiling on Windows](../../../docs/profiling-windows.md).

- **9.1 (Windows):** the native pointer in WebView2 passes (size, hotspot, I-beam, hidden pointer; maintainer). Idle session: whole app 2.1% of one core, main thread 0.2%, so no endless animation.
- **9.2 (Windows):** whole app, average of 30 s: idle 2.1%, scrolling terminal 6.4%, typing with echo 27.9%, window drag 68.4% (peak 129%). WebView2's GPU process stays near 3%; the browser process (19% in the drag) and 1-1's main thread (14%) carry the frame transport (Tauri channel data fetched over the IPC custom protocol, one WebView2 resource request per message).
- **Rule of requirement 5.3:** met for typing and the drag (more than twice the Linux native layer's about 6%). The cost is mostly the transport, not drawing. Decision pending with the maintainer.

## 21 November 2026: Windows native layer (amendment 5a, tasks 10.4 and 10.5)

Same workstation, preview build with tasks 10.1–10.3; the log shows `Remote picture: Native path (native layer available)` and rendering on the Radeon 890M through DX12. The maintainer used sessions on it and reported that it works ("from my perspective the tool worked"); a reactivation (xrdp resize) kept the session in place. Not yet checked one by one: snap layouts, a move between monitors of different scale, several sessions side by side, and the injected fallback (`ONE_ONE_RENDERER_FAIL_AFTER`).

Measured with `scripts/windows/Measure-AppCpu.ps1`, 30 s each, against the same host as the canvas run (10.0.0.62, 1474×1046):

| Scenario | Canvas, whole app | GPU layer, whole app | Main thread, canvas → layer | WebView2 browser, canvas → layer |
| --- | --- | --- | --- | --- |
| Scrolling terminal | 6.4% | 6.1% | 0.8% → 0.4% | – |
| Typing with echo | 27.9% | 15.4% | 5.2% → 1.6% | 6.9% → 4.6% |
| Window drag | 68.4% (peak 129%) | 26.7% (peak 67%) | 14.3% → 1.5% | 19.0% → 5.8% |

- Main thread at most 5% (5a.4): met in every scenario.
- Whole app at most half of the canvas (5a.4): met for the drag (39%); typing 55%, where the rest is the key-event IPC (one invoke per key event through WebView2's browser process), which the layer does not change; the scrolling terminal was too light on this host to differ.
- A probe run with `PROBE_THREAD=render` confirmed that configuring and committing from the render thread, as 1-1 does, shows the layer.
