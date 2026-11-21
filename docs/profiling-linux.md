# Profiling 1-1 on Linux

Two scripts record where the running app spends its CPU time while you keep
using it. They attach to the installed or locally built `rdp-client` and its
WebKit web process; nothing in the app changes.

## One-time setup

```bash
sudo scripts/linux/profile-setup.sh
```

This installs Sysprof and makes `/usr/local/bin/perf-cap`: a copy of `perf`
with file capabilities (`cap_perfmon`, `cap_sys_ptrace`, `cap_syslog`), not
setuid root, that only your group can run. The system `perf` is unchanged.
Without it, `perf` cannot attach to a process that is already running
(`kernel.yama.ptrace_scope=1`). Undo with `sudo rm /usr/local/bin/perf-cap`.

## Recording

```bash
scripts/linux/profile.sh start --wait   # attaches when 1-1 starts (or now, if it runs)
scripts/linux/profile.sh cpu            # main-thread and web-process CPU, once a second
scripts/linux/profile.sh dump           # save the recent window when something is slow
scripts/linux/profile.sh record 15 mouse # 15 s straight to disk, for a planned scenario
scripts/linux/profile.sh report         # summary of the newest dump
scripts/linux/profile.sh stop
```

The recorder keeps only the newest samples in memory (`perf record
--overwrite`, 299 Hz, frame-pointer call graphs), so it can run for hours; each
`dump` writes the last moments. Every `start` creates
`~/.cache/1-1-profiles/run-<time>/` with the dumps, `recorder.log` and
`cpu.log` (set `PROFILE_DIR` to change the location). A restarted app needs a
new `start`.

The rolling buffer is small (128 KiB per CPU: the kernel's default
`perf_event_mlock_kb` is 4 MiB), so with call stacks a dump holds well
under a second of a busy thread. For a scenario you can plan, use `record`.

`report` resolves symbols offline. For WebKit and GTK names, allow the Ubuntu
symbol server; the first report then downloads debug info and is slow:

```bash
DEBUGINFOD_URLS=https://debuginfod.ubuntu.com scripts/linux/profile.sh report
```

Dumps are ordinary `perf.data` files: `perf-cap report -i <file>` for
interactive use, or `perf-cap script -i <file> > out.perf` and open it in the
[Firefox Profiler](https://profiler.firefox.com) (loaded and processed locally in the
browser). Release builds are stripped, so 1-1's own frames show as addresses;
a local `cargo tauri build` keeps the symbols.

## What to look for

The **main thread** (`rdp-client`) runs GTK and carries mouse and keyboard
input to the session; time spent there is felt as input lag. On Wayland,
WebKitGTK with GTK 3 repaints the whole window in software for every web view
frame. In a report this shows as `gtk_main_do_event` → `cairo_fill` →
`pixman_fill`, plus kernel page faults (`clear_highpages`, `__zap_vma_range`)
for the fresh window buffer. With fractional scaling GTK 3 draws at the next
integer scale, so at 1.6 a 1888×1137 window repaints a buffer of about 34 MB.

Findings from 20 November 2026 (Hyprland, scale 1.6, three sessions):

| State | Main thread | Cause | Fix |
| --- | --- | --- | --- |
| Idle, beta.1 | about 80% | endless CSS and framer-motion animations (pulse, spinners, blurred overlay) | removed |
| Moving the mouse | 50–75% | the remote pointer composited into the frame: every move was a frame | native pointer (`rdp/remote_pointer.rs`) |
| Idle, now | 0% | — | — |
| Remote screen changing | 20–50% | one full-window GTK repaint per update | native renderer (planned) |

A new continuous animation anywhere in the UI brings the idle cost back. On
Linux, keep looping animations out of the session and dashboard views.

A WebKit web-process crash at exit (`libgbm`/`dri_gbm.so` freeing during exit
handlers) shows up in `coredumpctl`. It predates these changes and does not
affect sessions.

## Release build settings

Measured on 21 November 2026 on the reference machine with
`cargo test --release --lib bench_ -- --ignored --nocapture`
(`src-tauri/src/bench_decode.rs`: ClearCodec decode of a terminal-like
1920×1080 frame, and writing and taking a 1920×200 band through a frame
store; median of three runs). Rebuild time is after touching the app crate.

| Release profile | Binary | Rebuild | ClearCodec 1080p | Frame-store band |
| --- | --- | --- | --- | --- |
| Cargo defaults | 47.97 MB | 51 s | 9.0 ms | 0.21 ms |
| `lto = "thin"` | 48.20 MB | 108 s | 8.9 ms | 0.20 ms |
| `lto = "thin"`, `codegen-units = 1` | 35.58 MB | 280 s | 9.0 ms | 0.21 ms |

No setting makes decoding measurably faster: the hot loops are in IronRDP's
crates and already optimized per crate. One codegen unit makes the binary 26%
smaller at five times the rebuild time, so release builds keep the defaults.
The C compilers (`gcc`/`g++`) only build a few dependencies such as `ring`,
whose hot paths are hand-written assembly. `target-cpu=native` would make
shipped builds crash on older CPUs.

