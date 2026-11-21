# Probe: native renderer on Linux through a Wayland subsurface

Question: can 1-1 show remote frames on Linux without the GTK 3 full-window
software repaint that every web view frame costs today
([profiling notes](../../docs/profiling-linux.md))?

The probe is a minimal Tauri app with the same stack as 1-1 (versions from
`src-tauri/Cargo.lock`, copied here as the lockfile):

- The window is transparent and the page leaves a transparent hole, with an
  HTML "toolbar" floating over it.
- `PROBE_MODE=subsurface` (default): on GTK's main thread the probe takes the
  toplevel's `wl_display` and `wl_surface` from GDK, creates its own surface on
  the same connection and makes it a desynchronized `wl_subsurface` placed
  **below** GTK's surface, with an empty input region (clicks stay with the web
  view) and buffer scale 2. The subsurface follows the hole: its position is
  parent state, so `queue_draw` lets GTK's next commit apply it. A render
  thread presents a 60 Hz pattern through wgpu (Vulkan) at the hole's size.
- `PROBE_MODE=canvas`: the page draws the same pattern into a canvas with two
  full-size `putImageData` calls per frame, which is the current 1-1 path
  without the IPC.
- `PROBE_UPLOAD=full` uploads the whole frame each time (worst case);
  `strip` uploads 64 rows (a terminal scrolling); `clear` only clears to a
  changing color.

```bash
CC=gcc CXX=g++ CARGO_TARGET_DIR=../../src-tauri/target cargo build --release
PROBE_MODE=subsurface ../../src-tauri/target/release/probe-native-subsurface
PROBE_MODE=canvas     ../../src-tauri/target/release/probe-native-subsurface
```

Results: [results-2026-11-21.md](results-2026-11-21.md).

The probe is not production code: `strip` writes straight into a fresh
swapchain image, whose other rows are undefined, so it shows black around
the strip. A real renderer keeps a persistent texture and blits it (as
`src-tauri/src/renderer/gpu.rs` does).
