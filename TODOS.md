# TODOS

This is a personal fork (no warranty, no support), so this list is a scratchpad, not a promise.

## Done

- **IronRDP core.** Full TLS + CredSSP/NLA negotiation against real servers: Windows RDP Server (Win11, NLA mandatory), FreeRDP/xfreerdp on Linux, and IronRDP-based Rust servers on Linux. Actor-per-session pattern.
- **Auto-reconnect.** Exponential backoff of 2s, 4s, 8s, 16s, then 30s capped, max 5 attempts, with an overlay showing attempt count and a cancel button. Fires on connection loss, not on user disconnect.
- **Clipboard sync.** Bidirectional via IronRDP's cliprdr channel, with a status indicator in the layout.
- **Status bar.** Bottom bar with connection state and metrics; hide or pin it per host, right from the bar itself.
- **Bandwidth-saver pause.** Client-issued pause of display updates from the session. Hosts that ignore the pause are reported within seconds, and mouse or keyboard input keeps a fresh pause alive for five seconds.
- **Idle preview.** Per-host option (Windows preview): after five idle minutes a busy unattended desktop refreshes about once a minute; input or tab focus restores live updates.
- **Compression and codec stack.** Per-host bulk compression (off, RDP4, RDP5, RDP6, RDP6.1) and per-host color depth (15/16/24/32 bpp), negotiated at connect time and applied on reconnect. Codec policy advertises only ClearCodec, Planar, Uncompressed, and RFX Progressive; H.264/AVC is never advertised or decoded. ClearCodec is preferred over H.264 because H.264 blurs small text, which matters for agentic coding in VMs. Performance HUD carries receive/send rates and totals.
- **SSH config import.** Parses `~/.ssh/config` Host entries into connections on port 3389.
- **Auto-update plumbing.** Updater plugin, signed preview feed (`latest.json` on the `preview-channel` prerelease), CI-built NSIS/DMG/AppImage on version tags. Updates are explicit and blocked during active sessions.

## Remaining

- **Segmented redirect copies (P1).** RDP drive redirection with one granted local folder per host, then segmented copying: bounded parallel range requests on the same file and connection, verified byte for byte against the ordinary mount. Ship only if measurements show a real win over one range in flight.
- **Rsync-style delta sync (P2).** On top of redirects, move only changed blocks (rsync/Unison-style comparison over the redirected drive). Deferred until the copy measurements exist.
- **Auto-update hardening (P2).** Runtime validation currently covers Windows x64 only; extend the real-host matrix to ARM64, macOS, and Linux and keep the feed and signing workflow honest.
- **Audio redirection (P3).** rdpsnd channel, decoded and played locally.
- **Multi-monitor (P3).** Beyond the current single-surface model.
- **Session recording (P3).** Frames to MP4.
- **LAN auto-discovery (P3).** Find Windows machines on the local network.
