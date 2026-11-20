# Spike: Linux hardware H.264 through GStreamer

Throwaway probe for the per-host H.264 option (20 November 2026). It implements
upstream's `ironrdp_egfx::decode::H264Decoder` with a GStreamer pipeline
(`appsrc ! h264parse ! <decoder> ! appsink`, NV12/I420 in system memory) and
benchmarks every installed decoder on encoded test streams. Not part of any CI
tier; the app's manifest is untouched.

## Questions and answers (Intel UHD 630 + AMD RX 5500M, GStreamer 1.28.2, Ubuntu)

1. **Does every call return its own picture?** The trait requires it, and AVC444
   pairs its two views by call order. Out of the box, no: GStreamer's H.264 base
   decoder (`vah264dec`, also `nvh264dec`) holds one picture back when the SPS
   declares a one-frame DPB (`max_dec_frame_buffering = 1`, which openh264's
   encoder writes), and `openh264dec` holds one back on main/high streams.
   `compliance=flexible` does not change it. **Fix:** `src/sps.rs` rewrites
   each SPS in flight to `max_num_reorder_frames = 0` and
   `max_dec_frame_buffering >= 2` (RDP never reorders). After that,
   `vah264dec` on both GPUs returns 120/120 pictures on time and bit-exact
   against the software reference on every stream tested.
2. **Speed (decode + copy to system memory, p50 per frame).** Intel `vah264dec`:
   about 5 ms at 1080p, 9 ms at 3184×1272, 17–19 ms at 4K. AMD
   (`varenderD129h264dec`, a dGPU behind the T2): slower, 7–12 ms at 1080p,
   25–33 ms at 4K, but uses less CPU. Software `openh264dec`: 6 ms at 1080p
   baseline, but 15–19 ms at 1080p and 58–75 ms at 4K on main/high.
3. **CPU.** Hardware decode is not free: CPU time per frame is close to the
   latency, mostly the copy out of GPU memory.
4. **Color conversion costs more than decoding.** Upstream's
   `Yuv420Frame::to_rgba` (MS-RDPEGFX transform) takes about 17 ms at 1080p and
   65–75 ms at 4K. Its AVC420 path converts the whole picture for every frame,
   then copies only the `regionRects` that changed. Converting only those
   regions (as upstream's AVC444 path already does) would remove most of it for
   terminal updates. `fast_to_rgba` here (same output byte for byte) is about
   2× faster; real SIMD would gain more.
5. **Legacy `vaapih264dec`** needs `low-latency=true` and was not bit-exact at
   1080p in these runs; use the `va` plugin (`vah264dec`) instead.

Full tables: `results-2026-11-20.md`. Test streams are a full-motion test
pattern, the worst case; remote desktops change far less per frame.

## Not answered yet

Real servers: Windows AVC444 and xrdp AVC420 streams through the full EGFX
pipeline, and NVIDIA `nvh264dec`.

## Run

The Rust bindings link through pkg-config. Without the GStreamer `-dev`
packages, point `PKG_CONFIG_PATH` at `.pc` files naming the runtime libraries.

```sh
cargo run --release -- [frames] [WxH ...]
```

Knobs: `PROBE_DEBUG=1` (per-frame notes), `PROBE_NO_SPS_REWRITE=1`,
`PROBE_DEFAULT_OPTIONS=1` (no decoder low-latency options), `PROBE_NO_PARSE=1`.
`cargo run --release --example spsdump` prints the NAL types of encoded access
units and whether the rewrite fires.
