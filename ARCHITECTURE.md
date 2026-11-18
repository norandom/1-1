# Architecture

## Current architecture

One Tauri process: a React WebView for the UI on top, a Rust backend below, and a dedicated wgpu thread that owns the actual pixels.

![1-1 architecture](docs/diagrams/architecture.svg)

Editable source: [docs/diagrams/architecture.mmd](docs/diagrams/architecture.mmd). The WebView tree: App.tsx with ErrorBoundary, frameless Titlebar, collapsible Sidebar, routes (`/` ConnectionGrid, `/session/:id` SessionView with SessionCanvas fallback, SessionToolbar, SessionTabs, PerformanceHUD, `/settings`), QuickConnect (Cmd+K), ShortcutOverlay (Cmd+?), StatusBar, ClipboardStatus, DisplayPauseControl, ToastContainer; stores for connections, sessions, settings, toasts. The backend: `commands/` (CRUD, session incl. pause_display, updater), `store/` (connections.json, settings.json, OS keychain), `rdp/` (SessionManager + one SessionActor per connection, IronRDP client, input, cliprdr clipboard, pause/idle preview, network metrics), `renderer/` (wgpu GpuRenderer + SharedFrame), `utils/` (TCP probe, latency).

## Session model: one actor per session

`SessionManager` is a lightweight registry of handles. Each handle owns an mpsc command channel and a watch channel for session info; dropping the handle kills the actor task.

![Session actors](docs/diagrams/session-actors.svg)

Editable source: [docs/diagrams/session-actors.mmd](docs/diagrams/session-actors.mmd).

Why actors instead of one shared mutex:

1. No lock contention. Each session owns its state, so frame emission never blocks input forwarding on another session.
2. Clean lifecycle. Dropping the handle kills the actor task.
3. Backpressure. The mpsc channel queues commands naturally.
4. IronRDP fits. Its async session loop is a long-lived task, which maps directly onto the actor pattern.

### SessionHandle API

```rust
struct SessionHandle {
    id: String,
    cmd_tx: mpsc::Sender<SessionCommand>,
    info: watch::Receiver<SessionInfo>,
}

enum SessionCommand {
    SendKey { key_code: u32, is_down: bool },
    SendMouse { x: i32, y: i32, button: Option<String>, event_type: String },
    Resize { width: u32, height: u32 },
    SetDisplayPaused { paused: bool },
    Disconnect,
}
```

## Frame delivery

The session actor writes dirty rects into a `SharedFrame`, a zero-copy double buffer shared with the renderer thread. One mutex lock covers a whole batch of rects, and a condvar wakes the renderer only when there is work, so an idle session costs 0% CPU. The renderer uploads the frame with `queue.write_texture` and presents a fullscreen quad via wgpu (Metal on macOS, Vulkan on Linux). If no GPU is available, the frontend falls back to 2D Canvas rendering in the WebView.

![Frame pipeline](docs/diagrams/frame-pipeline.svg)

Editable source: [docs/diagrams/frame-pipeline.mmd](docs/diagrams/frame-pipeline.mmd).

## Session state machine

![Session state machine](docs/diagrams/session-state-machine.svg)

Editable source: [docs/diagrams/session-state-machine.mmd](docs/diagrams/session-state-machine.mmd).

## Fork additions on top of Drift

- **Per-host wire tuning** (`store/connections.rs`, `rdp/client.rs`): bulk compression (off / RDP4 / RDP5 / RDP6 / RDP6.1) and color depth (15/16/24/32 bpp) are per-host profile fields negotiated in the X.224 client info and applied on reconnect; multitransport stays off.
- **Codec policy** (`rdp/codec_policy.rs`, `rdp/egfx.rs`): the client advertises only ClearCodec, Planar, Uncompressed, and RFX Progressive; no H.264/AVC variant is ever advertised or decoded, and an AVC frame arriving on the graphics pipeline is an error. ClearCodec is the preference because it keeps text sharp for agentic coding in VMs, where H.264 chroma subsampling smears small glyphs.
- **Bandwidth-saver pause and idle preview** (`rdp/idle_preview.rs`, `commands/session.rs::pause_display`): the client can pause display updates per session. Hosts that ignore the pause are reported within seconds, and fresh input keeps a pause alive for five seconds. The per-host idle preview refreshes a busy unattended desktop about once a minute after five idle minutes.
- **Clipboard sync** (`rdp/clipboard.rs`, cliprdr virtual channel): bidirectional text/image sync between local clipboard and the remote session, surfaced through `ClipboardStatus`.
- **Network metrics** (`rdp/network_metrics.rs`): transport receive/send rates and totals shown in the HUD; these count TLS payload overhead but not TCP/IP headers or retransmissions, and they are separate from decoded local frame delivery.
- **Auto-update** (`tauri-plugin-updater`, `commands/updater.rs`): checks the signed preview feed once per launch and from Settings; installation is explicit and blocked while sessions are active.
- **Server compatibility matrix**: the connector is exercised against Windows RDP Server (NLA), FreeRDP/xfreerdp (TLS), and IronRDP-based Rust servers, with NLA/CredSSP, cliprdr, and resize behavior verified per class.

## Key design decisions

1. Event-driven frames over polling: Rust pushes frames to the renderer via a condvar, never a spin loop
2. OS keychain for credentials: keyring crate, never plaintext
3. tokio::fs for persistence: non-blocking I/O in the async context
4. Zero-copy SharedFrame instead of per-frame copies or image encoding on the hot path
5. SessionView owns the session lifecycle, so there are no double-connect races
6. Max 10 sessions, 30s connect timeout: bounded resources
7. 8K resolution cap on resize: no OOM from malformed input
