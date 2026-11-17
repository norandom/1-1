# Coding and agentic bandwidth optimization

Research date: 2026-11-17. User priority: sharp code/terminal text, immediate input response, reliable clipboard, and long-running remote agent sessions. Native acceptance scope: Windows only. No claimed measured savings yet.

## Findings

1. The local framebuffer already exists. `session-runtime.ts` composites changed rectangles into a retained RGBA surface, and the backend retains `DecodedImage`. RDP can send changed rectangles, cached tiles and surface-copy operations; it does not need to resend the whole desktop for every keystroke. A second local framebuffer alone cannot reduce incoming traffic.
2. The previous HUD bandwidth value counted decoded IPC frame bytes. Task 11.0n replaces it with successful TCP read/write byte accounting below TLS. This includes TLS records, excludes TCP/IP headers/retransmissions, and is not NIC billing data. Report Rx/Tx rates separately from render FPS.
3. Bulk compression is already mapped to upstream MPPC8K/64K/RDP6/RDP6.1 requests. Color depth is a bitmap request; modern EGFX uses independent formats. Neither an advertised preference nor the network-type hint proves server behavior. No user-selected profile should be silently overwritten.
4. Our EGFX capabilities advertise SMALL_CACHE and AVC_DISABLED. Pinned ironrdp-egfx 0.5.0 supports a 16 MiB small bitmap cache or 100 MiB normal cache, with bounded slots. At ten sessions, the larger cache alone can consume about 1 GiB. Increasing cache is a candidate, not a guaranteed improvement: the server must use it and the workload must reuse tiles.
5. The pinned client also has `with_persistent_cache(Box<dyn PersistentCacheStore>)`, but our adapter does not use it. Start with RAM-only cache experiments. Persistent caches contain remote-screen pixels, need per-host/account isolation, explicit opt-in, storage bounds and clear-on-demand, and can block graphics if disk I/O is performed synchronously. Do not add them merely for another checkbox.
6. RemoteFX Progressive supports differential transform coefficients and refinement. Non-AVC is not synonymous with mathematically lossless. Do not promise no transient softness merely because H.264 is disabled. Compare colored syntax and small text immediately after input, not only settled screenshots. Preserve current non-H.264 policy.
7. The stored global quality slider is not wired to a server encoder in the current connector. It is not a bandwidth control; a future UI cleanup should remove or accurately label it rather than imply effectiveness.

## Idle preview design candidate

Opt-in per-host setting: after 300 seconds without local session keyboard/mouse/wheel/paste activity, retain the last complete local image and request server graphics suppression. Remote screen changes do not reset the inactivity clock: an agent may keep printing while the user is away. Focus return or any local input immediately resumes graphics before forwarding input; never discard the first keystroke. Provide manual Resume and a compact existing-status indication showing last preview age. No extra toolbar.

Every 60 seconds, open a bounded preview window: resume output and request refresh where supported, continue decoding/acknowledging, then suppress after a complete update/refinement opportunity or timeout. Target approximately one preview per minute, not exactly one wire frame; a snapshot can require multiple PDUs, rectangles, frames or refinement passes. A timeout must be bounded without blocking input. Do not suppress on the first arbitrary dirty rectangle and freeze an incomplete/progressive image.

Maintain the RDP socket, reads, TLS state, graphics decoder/cache state, protocol responses, clipboard channel and frame acknowledgments throughout. Do not implement savings by stopping reads, withholding acknowledgments or dropping frames before decoding. Do not fake activity to evade server session policy. Server idle/lock/disconnect rules still apply.

On resize/reactivation: resume, use confirmed desktop dimensions, rebuild capability-dependent state, obtain a fresh display, then restart inactivity timing. On reconnect: reset idle state and all measurement baselines. Unsupported server capability: leave normal graphics running and show unsupported; do not claim savings. Repeated suppression/refresh failure: restore normal updates and surface a recoverable error.

### Verified pinned upstream API surface

- `ironrdp-connector 0.12.0`: `ConnectionResult::{suppress_output_support,refresh_rect_support}`; activation results also expose updated capability flags. Preserve these through initial activation and reactivation.
- `ironrdp-session 0.13.0`: public `ActiveStage::encode_static` encodes upstream `ShareDataPdu`; upstream `SuppressOutputPdu { desktop_rect: None }` suppresses and `Some(InclusiveRectangle)` resumes. Use upstream encoders and current stage/share id, not custom bytes.
- `ActiveStage::request_full_redraw(width,height,refresh_supported,suppress_supported)` returns a suppress/resume pair where supported, otherwise RefreshRectangle, otherwise no frames. This is a recovery helper, not a sustained idle-mode scheduler. Do not retain encoded packets across reactivation.
- `ironrdp-egfx 0.5.0`: compositor cache limits and existing frame acknowledgments must remain intact. UI delivery-stall tracking must not mistake intentional server silence for an unconsumed local frame.

The APIs establish feasibility, not real-server support or savings. A one-minute full refresh may cost more than the small deltas of a static screen. Measure before enabling by default; retain opt-in semantics.

## Implementation and experiment order

1. Complete measured transport Rx/Tx and observed codec diagnostics (11.0n); verify partial IO, errors, idle rates, elapsed time, reconnects and session isolation.
2. Add an independently tested idle state machine and per-host opt-in setting using the above APIs, including timeout and wake races. Native test the Windows host first; expand to the other two hosts only within Windows-client testing scope.
3. Compare current 16 MiB cache against bounded 100 MiB option, measure memory and repeat-scroll traffic. Keep font smoothing and full window drag as explicit user choices.
4. Consider persistent caching only if measured reconnect traffic justifies the added privacy/storage complexity. No H.264 work in this roadmap.

Use the same resolution, host, font, content and activity trace: idle desktop, typing code, scrolling code, window dragging, active terminal output, and unattended terminal output beyond five minutes. Record actual Rx/Tx totals/rates, observed codec/path, CPU/RAM, time to first sharp text after input, and wake responsiveness. Compare original profile versus compression, then cache, then idle suppression one variable at a time. A local application frame counter is not a network benchmark. Do not simulate user input on the user's real coding sessions or persist benchmark credentials without authorization.

## Primary sources

- Microsoft graphics behavior: https://learn.microsoft.com/en-us/azure/virtual-desktop/graphics-encoding
- Graphics suppression purpose: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/c0931f7e-9e4d-4174-bf96-14931feb3477
- Suppress Output fields: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/0be71491-0b01-402c-947d-080706ccf91b
- Progressive differential/refinement behavior: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/219ffb6f-4c86-4dd9-bf13-79b29af9ae99
- Bitmap cache: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/d1c1540f-9bc7-4344-81e0-3cce6b845f86
- Upstream implementation: https://github.com/Devolutions/IronRDP ; exact-version code verified locally in configured Cargo registry: ironrdp-session-0.13.0/src/active_stage.rs, ironrdp-connector-0.12.0/src/connection{,_activation}.rs, ironrdp-egfx-0.5.0/src/{cache,client,compositor}.rs. Current repository Cargo.lock is the authority for pinned versions.