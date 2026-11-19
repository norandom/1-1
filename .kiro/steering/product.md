# Product Overview

1-1 (pronounced zero; formerly Drift) is a cross-platform remote desktop (RDP) client for Windows, macOS, and Linux, aimed at professional users. It is a local-first tool: no cloud services, no telemetry, credentials in the OS keychain. Preserve existing application identity and credential/storage paths across the visible rename.

Current implementation includes Windows support, hierarchical libraries, font-smoothing requests, fit/native display modes and server-confirmed smart resize. H.264 transport was removed. Tri-OS release journeys and the complete real-server acceptance matrix remain unfinished; implementation is not blanket verification. Current priority is compact session chrome, professional per-host experience controls and viewport-sized initial connections.

## Target Capabilities

- **RDP connectivity**: Full IronRDP protocol implementation (TLS + CredSSP/NLA), keyboard/mouse/scroll input forwarding, clipboard sync, auto-reconnect.
- **Remote display pipeline**: Decoded bitmap frames (ClearCodec/standard bitmap compression, no H.264) rendered via a GPU surface with a webview-canvas fallback.
- **Connection management**: Hierarchical connection library (Royal TS-style sidebar tree), folders, tagging, `.rdp` file and SSH config import, latency probing, quick connect.
- **Multi-session workflow**: Up to 10 concurrent sessions with tabbed documents, per-session status, performance HUD.

## Target Use Cases

- Daily drivers connecting to Windows workstations/servers over RDP from any desktop OS.
- Operators managing many saved connections organized in folders, with live status at a glance.
- Users who need reliable reconnection and responsive input on high-latency links.

## Value Proposition

Native performance (no Electron), real RDP protocol via IronRDP, modern UI with Royal TS-style document management, and a tiny single-binary footprint.

---

_Focus on patterns and purpose, not exhaustive feature lists_

## Primary workflow — user clarification 2026-11-17
The primary purpose is remote RDP coding and agentic work: sharp code/terminal text, prompt typing feedback, reliable clipboard, and efficient long-lived sessions. Preserve non-H.264 rendering; bandwidth optimization must not trade text clarity for video smearing. Investigate opt-in idle graphics suppression for unattended agent output, restoring live display immediately on local input.
