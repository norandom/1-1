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
