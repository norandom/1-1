# Terminal graphics defaults design

## Cache size

`CodecPolicy::egfx_caps()` sets `SMALL_CACHE` on every capability set today. Upstream `ironrdp-egfx` 0.5 derives the cache budget from the confirmed set (`CACHE_BYTES` 100 MB / `CACHE_SLOTS` 25,600 without the flag, 16 MB / 4,096 with it) and allocates entries only when the server stores them. For the measurement, `ONE_ONE_EGFX_CACHE=large` drops the flag; the default stays small until the numbers decide. If adopted, the flag is removed from the policy and the variable deleted.

Measurement uses the existing Transport RX/TX counters and the process's resident memory, on Windows and xrdp, 1920×1080 and 3180×1271, three runs each of: 60 s typing in a shell, scrolling 5,000 lines of a log in `less`, cycling ten terminal tabs, and switching between three windows. Results go to `measurements.md`.

## QOI and QOIZ

`ironrdp-session` 0.13 decodes `CODEC_ID_QOI` and `CODEC_ID_QOIZ` surface bits behind its `qoi` and `qoiz` features (adds `qoicoubeh` and `zstd-safe`). `CodecPolicy::fastpath_bitmap()` would list both in `BitmapCodecs`. They matter only when EGFX is not negotiated, so the first check is whether hypr_rdp negotiates EGFX at all; if it does, this requirement is dropped. Windows and xrdp must connect unchanged with the extra codec entries.

## Not adopted

| Capability | Reason |
|---|---|
| Legacy RemoteFX, NSCodec | Lossy; blurs text. |
| Persistent EGFX cache | Stores screen fragments, possibly secrets, on disk; needs an explicit switch. |
| H.264 | Parked branch `spike/h264-hw-decode`; returns as a per-host switch. |
| UDP transport | IronRDP has no RDPEUDP implementation. |
| Glyph cache | IronRDP has no GDI drawing orders; ClearCodec's glyph cache covers EGFX sessions. |
