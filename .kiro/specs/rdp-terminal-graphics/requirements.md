# Terminal graphics defaults

## Introduction

1-1 is used mostly for terminals: text that must stay sharp, typing that must echo at once, and little bandwidth. This spec adopts upstream IronRDP capabilities that serve that use without new per-host switches. H.264 is parked on the `spike/h264-hw-decode` branch; when it returns, it adds a per-host codec choice on top of the same codec policy. Approved by the maintainer on 20 November 2026.

## Boundary

In scope: what 1-1 advertises by default (graphics cache size, lossless bitmap codecs for sessions without the graphics pipeline) and the measurements that decide it. Out of scope: H.264, legacy RemoteFX and NSCodec (lossy, bad for text), the persistent bitmap cache (writes screen content to disk; needs a switch first), UDP transport and the glyph cache (not implemented in IronRDP 0.x).

## Requirements

### 1. Graphics cache size

1. 1-1 shall advertise the large EGFX bitmap cache (100 MB, 25,600 entries) instead of the small one (16 MB, 4,096 entries) only if measurement shows it lowers received bytes for terminal work on the Windows and xrdp hosts.
2. The measurement shall record received bytes and resident memory per session for typing, scrolling a long log, switching between terminal tabs and window switching, with both cache sizes at the same resolution.
3. If the large cache is adopted, the cache shall not be filled beyond the advertised budget, and the session details shall not change.

### 2. Lossless codecs without the graphics pipeline

1. When a session runs without EGFX, 1-1 shall advertise QOI and QOIZ (lossless, zstd-compressed) bitmap codecs in addition to today's uncompressed and planar bitmaps, if the IronRDP-based host uses them and the Windows and xrdp hosts ignore them.
2. The status overlay shall show QOI or QOIZ as the observed codec when the server uses them.

### 3. Codec policy stays the single source

1. Every graphics capability shall still come from `CodecPolicy`, so a later per-host codec choice (H.264) changes one place.
2. Diagnostics for the measurements may use an environment variable, as `ONE_ONE_SHARE_TRACE` does; no setting is added to the UI.
