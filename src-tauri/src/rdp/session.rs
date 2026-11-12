use chrono::{DateTime, Utc};
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::{mpsc, watch, Mutex};
use uuid::Uuid;

use crate::rdp::client::{attempt_rdp_connection, ConnectionOutcome, SessionCommand};
use crate::rdp::clipboard::SessionClipboardBackend;
use crate::rdp::display::FrameBuffer;
use crate::rdp::frame_transport::{
    encode_full_frame_packet, encode_h264_packet, encode_image_update_packet,
};
use crate::renderer::h264_encoder::H264FrameEncoder;
use crate::renderer::shared_frame::SharedFrame;
use crate::store::connections::ConnectionConfig;

pub const MAX_SESSIONS: usize = 10;
pub const MAX_RECONNECT_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceMetrics {
    pub fps: f64,
    pub latency_ms: u32,
    pub bandwidth_kbps: u32,
    #[serde(skip)]
    pub frames_sent: u64,
    #[serde(skip)]
    pub started_at: Option<std::time::Instant>,
}

impl PerformanceMetrics {
    pub fn new() -> Self {
        Self {
            fps: 0.0,
            latency_ms: 15,
            bandwidth_kbps: 0,
            frames_sent: 0,
            started_at: Some(std::time::Instant::now()),
        }
    }
}

/// Flattened session info sent to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub connection_id: String,
    pub connection_name: String,
    pub host: String,
    pub status: SessionState,
    pub fps: f64,
    pub latency: u32,
    pub bandwidth: u32,
    pub width: u32,
    pub height: u32,
    pub connected_at: Option<DateTime<Utc>>,
    pub reconnect_attempts: u32,
    pub max_reconnect_attempts: u32,
    pub last_error: Option<String>,
}

/// Lightweight handle to a running session actor.
/// Stored in SessionManager instead of the full Session struct.
pub struct SessionHandle {
    /// Send commands (key/mouse/resize/disconnect) to the actor.
    pub cmd_tx: mpsc::Sender<SessionCommand>,
    /// Receive the latest session info (lock-free reads via watch).
    pub info_rx: watch::Receiver<SessionInfo>,
    /// Receive the latest remote clipboard text.
    pub clipboard_rx: watch::Receiver<Option<String>>,
    /// The tokio JoinHandle for the actor task.
    #[allow(dead_code)]
    pub task_handle: tokio::task::JoinHandle<()>,
}

/// Narrow transport seam at the actor's RDP connection boundary.
///
/// Production always resolves to [`attempt_rdp_connection`]; characterization
/// tests inject scripted outcomes so admission, retry, cancellation and
/// teardown behavior can be pinned deterministically without network I/O.
/// Behavior is unchanged: the production connector is the real sequence.
pub(crate) type ConnectFn = Arc<
    dyn Fn(
            ConnectionConfig,
            Option<String>,
            u32,
            u32,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = ConnectionOutcome> + Send>>
        + Send
        + Sync,
>;

/// Production connector: always [`attempt_rdp_connection`].
pub(crate) fn real_connect() -> ConnectFn {
    Arc::new(|config, password, width, height| {
        Box::pin(async move {
            attempt_rdp_connection(&config, password.as_deref(), width, height).await
        })
    })
}

/// Recorded frontend-event emissions for deterministic lifecycle tests.
/// Entries carry the (paused-clock) instant so backoff timing is assertable.
#[cfg(test)]
pub(crate) struct RecordedEvents {
    entries: std::sync::Mutex<Vec<(tokio::time::Instant, String, serde_json::Value)>>,
    count_tx: watch::Sender<usize>,
}

#[cfg(test)]
impl RecordedEvents {
    pub(crate) fn new() -> (Arc<Self>, watch::Receiver<usize>) {
        let (count_tx, count_rx) = watch::channel(0usize);
        (
            Arc::new(Self {
                entries: std::sync::Mutex::new(Vec::new()),
                count_tx,
            }),
            count_rx,
        )
    }

    fn push(&self, event: &str, payload: serde_json::Value) -> Result<(), tauri::Error> {
        let mut entries = self.entries.lock().unwrap();
        entries.push((tokio::time::Instant::now(), event.to_string(), payload));
        let count = entries.len();
        drop(entries);
        self.count_tx.send_replace(count);
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> Vec<(tokio::time::Instant, String, serde_json::Value)> {
        self.entries.lock().unwrap().clone()
    }
}

type EmitFn = Arc<dyn Fn(&str, serde_json::Value) -> Result<(), tauri::Error> + Send + Sync>;

/// Narrow seam at the actor's outbound frontend-event boundary.
///
/// Production delegates to the Tauri event emitter; characterization tests
/// inject a recording sink so lifecycle behavior can be observed
/// deterministically without an app harness. Payloads serialize to JSON
/// once here, so both paths deliver identical event data. The production
/// emitter is reached only through this closure, which keeps the Tauri
/// windowing/tray objects out of the deterministic test binaries.
#[derive(Clone)]
pub(crate) struct SessionEventSink {
    emit_fn: EmitFn,
}

impl SessionEventSink {
    /// Production sink backed by the real Tauri event emitter.
    pub(crate) fn app(handle: tauri::AppHandle) -> Self {
        Self {
            emit_fn: Arc::new(move |event, payload| handle.emit(event, payload)),
        }
    }

    /// Test sink recording every emission in-process.
    #[cfg(test)]
    pub(crate) fn recorded(events: Arc<RecordedEvents>) -> Self {
        Self {
            emit_fn: Arc::new(move |event, payload| events.push(event, payload)),
        }
    }

    fn emit<S: serde::Serialize + Clone>(
        &self,
        event: &str,
        payload: S,
    ) -> Result<(), tauri::Error> {
        let value = serde_json::to_value(&payload).map_err(tauri::Error::Json)?;
        (self.emit_fn)(event, value)
    }
}

/// The session actor: a long-lived tokio task that owns all per-session state.
/// It receives commands via an mpsc channel and publishes session info via a watch channel.
/// Frames are streamed to the frontend via a Tauri channel.
struct SessionActor {
    id: String,
    config: ConnectionConfig,
    password: Option<String>,
    state: SessionState,
    metrics: PerformanceMetrics,
    width: u32,
    height: u32,
    connected_at: Option<DateTime<Utc>>,
    frame_buffer: FrameBuffer,
    frame_packet_scratch: Vec<u8>,
    cmd_rx: mpsc::Receiver<SessionCommand>,
    info_tx: watch::Sender<SessionInfo>,
    clipboard_tx: watch::Sender<Option<String>>,
    frame_channel: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
    events: SessionEventSink,
    local_clipboard_text: Option<String>,
    reconnect_attempts: u32,
    last_error: Option<String>,
    auto_reconnect: bool,
    connect: ConnectFn,
    h264_encoder: Option<H264FrameEncoder>,
    shared_frame: Option<Arc<SharedFrame>>,
}

impl SessionActor {
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: String,
        config: ConnectionConfig,
        password: Option<String>,
        cmd_rx: mpsc::Receiver<SessionCommand>,
        info_tx: watch::Sender<SessionInfo>,
        clipboard_tx: watch::Sender<Option<String>>,
        frame_channel: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
        events: SessionEventSink,
        auto_reconnect: bool,
        shared_frame: Option<Arc<SharedFrame>>,
        connect: ConnectFn,
    ) -> Self {
        let (width, height) = normalize_remote_resolution(
            config.display_width.unwrap_or(1920),
            config.display_height.unwrap_or(1080),
        );

        // Try to create the H.264 encoder; fall back to None if it fails
        let h264_encoder = match H264FrameEncoder::new(width, height) {
            Ok(enc) => {
                log::info!("H.264 encoder initialized for {}x{}", width, height);
                Some(enc)
            }
            Err(e) => {
                log::warn!(
                    "Failed to initialize H.264 encoder, will use raw RGBA: {}",
                    e
                );
                None
            }
        };

        Self {
            id,
            config,
            password,
            state: SessionState::Connecting,
            metrics: PerformanceMetrics::new(),
            width,
            height,
            connected_at: None,
            frame_buffer: FrameBuffer::generate_mock_frame(width, height),
            frame_packet_scratch: Vec::with_capacity((width * height * 4) as usize),
            cmd_rx,
            info_tx,
            clipboard_tx,
            frame_channel,
            events,
            local_clipboard_text: None,
            reconnect_attempts: 0,
            last_error: None,
            auto_reconnect,
            connect,
            h264_encoder,
            shared_frame,
        }
    }

    /// Build a SessionInfo snapshot from the current actor state.
    fn build_info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            connection_id: self.config.id.clone(),
            connection_name: self.config.name.clone(),
            host: self.config.host.clone(),
            status: self.state.clone(),
            fps: self.metrics.fps,
            latency: self.metrics.latency_ms,
            bandwidth: self.metrics.bandwidth_kbps,
            width: self.width,
            height: self.height,
            connected_at: self.connected_at,
            reconnect_attempts: self.reconnect_attempts,
            max_reconnect_attempts: MAX_RECONNECT_ATTEMPTS,
            last_error: self.last_error.clone(),
        }
    }

    /// Publish the latest session info through the watch channel.
    fn publish_info(&self) {
        let info = self.build_info();
        let _ = self.info_tx.send(info.clone());
        if let Err(error) = self.events.emit(&format!("session-info-{}", self.id), info) {
            log::debug!(
                "Failed to emit session info for session {}: {}",
                self.id,
                error
            );
        }
    }

    fn publish_remote_clipboard(&mut self, text: String) {
        let _ = self.clipboard_tx.send(Some(text.clone()));
        if let Err(error) = self.events.emit(&format!("clipboard-{}", self.id), text) {
            log::warn!(
                "Failed to emit clipboard event for session {}: {}",
                self.id,
                error
            );
        }
    }

    fn send_frame_packet(&mut self, packet: Vec<u8>) -> Result<(), DisconnectReason> {
        match self
            .frame_channel
            .send(tauri::ipc::InvokeResponseBody::Raw(packet))
        {
            Ok(()) => Ok(()),
            Err(error) => {
                log::warn!(
                    "Failed to send frame packet for session {}: {}",
                    self.id,
                    error
                );
                self.last_error = Some(format!("Frame emit failed: {}", error));
                Err(DisconnectReason::ConnectionLost)
            }
        }
    }

    /// Try to send a frame as H.264. Returns Ok(true) if sent, Ok(false) if encoder unavailable.
    fn try_send_h264_frame(
        &mut self,
        rgba_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<bool, DisconnectReason> {
        let encoder = match self.h264_encoder.as_mut() {
            Some(enc) => enc,
            None => return Ok(false),
        };

        match encoder.encode_rgba(rgba_data, width, height) {
            Ok(h264_data) => {
                if h264_data.is_empty() {
                    return Ok(false);
                }
                let packet = encode_h264_packet(width as u16, height as u16, &h264_data);
                self.send_frame_packet(packet)?;
                Ok(true)
            }
            Err(e) => {
                log::warn!("H.264 encode failed, falling back to raw RGBA: {}", e);
                // Disable H.264 encoder on failure
                self.h264_encoder = None;
                Ok(false)
            }
        }
    }

    fn send_mock_frame(&mut self) -> Result<(), DisconnectReason> {
        // Try H.264 encoding first for the mock frame
        let w = self.frame_buffer.width;
        let h = self.frame_buffer.height;
        // We need to clone data to avoid borrow conflict with self
        let rgba_data = self.frame_buffer.data.clone();
        if self.try_send_h264_frame(&rgba_data, w, h)? {
            return Ok(());
        }

        // Fall back to raw RGBA
        let packet = encode_full_frame_packet(&self.frame_buffer);
        self.send_frame_packet(packet)
    }

    async fn flush_clipboard_messages<W>(
        &mut self,
        active_stage: &mut ironrdp::session::ActiveStage,
        writer: &mut W,
    ) -> Result<(), DisconnectReason>
    where
        W: ironrdp_tokio::FramedWrite,
    {
        use ironrdp::cliprdr::{
            backend::ClipboardMessage, Client as CliprdrRoleClient, CliprdrClient,
            CliprdrSvcMessages,
        };

        let (pending_messages, remote_text_update) = {
            let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() else {
                return Ok(());
            };
            let Some(backend) = cliprdr.downcast_backend_mut::<SessionClipboardBackend>() else {
                return Ok(());
            };

            (
                backend.take_pending_messages(),
                backend.take_remote_text_update(),
            )
        };

        if let Some(text) = remote_text_update {
            self.publish_remote_clipboard(text);
        }

        for message in pending_messages {
            let svc_messages = {
                let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() else {
                    continue;
                };

                let result = match message {
                    ClipboardMessage::SendInitiateCopy(formats) => cliprdr.initiate_copy(&formats),
                    ClipboardMessage::SendFormatData(response) => {
                        cliprdr.submit_format_data(response)
                    }
                    ClipboardMessage::SendInitiatePaste(format) => cliprdr.initiate_paste(format),
                    ClipboardMessage::Error(error) => {
                        log::warn!("Clipboard backend error for session {}: {}", self.id, error);
                        continue;
                    }
                };

                match result {
                    Ok(messages) => Some(Vec::from(messages)),
                    Err(error) => {
                        log::warn!(
                            "Failed to build clipboard response for session {}: {}",
                            self.id,
                            error
                        );
                        self.last_error = Some(format!("Clipboard error: {}", error));
                        return Err(DisconnectReason::ConnectionLost);
                    }
                }
            };

            let Some(svc_messages) = svc_messages else {
                continue;
            };
            if svc_messages.is_empty() {
                continue;
            }

            let frame = match active_stage.process_svc_processor_messages(CliprdrSvcMessages::<
                CliprdrRoleClient,
            >::new(
                svc_messages
            )) {
                Ok(frame) => frame,
                Err(error) => {
                    log::warn!(
                        "Failed to encode clipboard channel payload for session {}: {}",
                        self.id,
                        error
                    );
                    self.last_error = Some(format!("Clipboard encode error: {}", error));
                    return Err(DisconnectReason::ConnectionLost);
                }
            };

            if frame.is_empty() {
                continue;
            }

            if let Err(error) = writer.write_all(&frame).await {
                log::warn!(
                    "Failed to write clipboard payload for session {}: {}",
                    self.id,
                    error
                );
                self.last_error = Some(format!("Clipboard write error: {}", error));
                return Err(DisconnectReason::ConnectionLost);
            }
        }

        Ok(())
    }

    async fn set_local_clipboard_text<W>(
        &mut self,
        active_stage: &mut ironrdp::session::ActiveStage,
        writer: &mut W,
        text: String,
    ) -> Result<(), DisconnectReason>
    where
        W: ironrdp_tokio::FramedWrite,
    {
        use ironrdp::cliprdr::CliprdrClient;

        self.local_clipboard_text = Some(text.clone());

        if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
            if let Some(backend) = cliprdr.downcast_backend_mut::<SessionClipboardBackend>() {
                backend.set_local_text(text);
            }
        }

        self.flush_clipboard_messages(active_stage, writer).await
    }

    /// Main actor loop.
    async fn run(mut self) {
        // Publish initial "Connecting" state
        self.publish_info();

        // Step 1: Attempt RDP connection (with graceful fallback to mock)
        log::info!(
            "Session {} attempting RDP connection to {}:{}",
            self.id,
            self.config.host,
            self.config.port
        );

        let outcome = (self.connect)(
            self.config.clone(),
            self.password.clone(),
            self.width,
            self.height,
        )
        .await;

        match outcome {
            ConnectionOutcome::RealSession {
                connection_result,
                framed,
            } => {
                log::info!("Session {} entered real RDP session", self.id);
                self.state = SessionState::Connected;
                self.connected_at = Some(Utc::now());

                // Update dimensions from the server's negotiated desktop size
                self.width = connection_result.desktop_size.width as u32;
                self.height = connection_result.desktop_size.height as u32;
                self.publish_info();

                // Run the real RDP session loop
                let disconnect_reason = self
                    .run_real_session_loop(*connection_result, *framed)
                    .await;

                if disconnect_reason == DisconnectReason::ConnectionLost && self.auto_reconnect {
                    self.run_reconnect_loop().await;
                }
            }
            ConnectionOutcome::Failed(reason) => {
                log::info!("Session {} using mock frames (reason: {})", self.id, reason);
                self.state = SessionState::Connected;
                self.connected_at = Some(Utc::now());
                self.publish_info();

                // Run the mock frame loop
                let disconnect_reason = self.run_mock_frame_loop().await;

                // Handle auto-reconnect if the loop ended due to a connection-like issue
                // (not a user-initiated disconnect)
                if disconnect_reason == DisconnectReason::ConnectionLost && self.auto_reconnect {
                    self.run_reconnect_loop().await;
                }
            }
        }

        // Actor finished; set state to disconnected
        self.state = SessionState::Disconnected;
        self.publish_info();
        log::info!("Session {} actor stopped", self.id);
    }

    /// Real RDP session loop: reads PDUs from the server, decodes graphics,
    /// and emits frame data to the frontend.
    async fn run_real_session_loop(
        &mut self,
        connection_result: ironrdp::connector::ConnectionResult,
        framed: crate::rdp::client::TlsFramed,
    ) -> DisconnectReason {
        use ironrdp::graphics::image_processing::PixelFormat;
        use ironrdp::session::image::DecodedImage;
        use ironrdp::session::{ActiveStage, ActiveStageOutput};
        use ironrdp_tokio::FramedWrite;
        let mut active_stage = ActiveStage::new(connection_result);
        let mut image =
            DecodedImage::new(PixelFormat::RgbA32, self.width as u16, self.height as u16);

        // Split the framed stream for concurrent read/write
        let (mut reader, mut writer) = ironrdp_tokio::split_tokio_framed(framed);

        if let Some(text) = self.local_clipboard_text.clone() {
            if self
                .set_local_clipboard_text(&mut active_stage, &mut writer, text)
                .await
                .is_err()
            {
                return DisconnectReason::ConnectionLost;
            }
        }

        loop {
            tokio::select! {
                // Read PDUs from the server
                pdu_result = reader.read_pdu() => {
                    let (action, payload) = match pdu_result {
                        Ok(ap) => ap,
                        Err(e) => {
                            log::warn!("Session {} read_pdu error: {}", self.id, e);
                            self.last_error = Some(format!("Connection lost: {}", e));
                            self.state = SessionState::Error(format!("Connection lost: {}", e));
                            self.publish_info();
                            return DisconnectReason::ConnectionLost;
                        }
                    };

                    let outputs = match active_stage.process(&mut image, action, &payload) {
                        Ok(outputs) => outputs,
                        Err(e) => {
                            log::warn!("Session {} process error: {}", self.id, e);
                            self.last_error = Some(format!("Session error: {}", e));
                            self.state = SessionState::Error(format!("Session error: {}", e));
                            self.publish_info();
                            return DisconnectReason::ConnectionLost;
                        }
                    };

                    let mut updated_regions = Vec::new();
                    for out in outputs {
                        match out {
                            ActiveStageOutput::ResponseFrame(frame) => {
                                if !frame.is_empty() {
                                    if let Err(e) = writer.write_all(&frame).await {
                                        log::warn!("Session {} write error: {}", self.id, e);
                                        self.last_error = Some(format!("Write error: {}", e));
                                        return DisconnectReason::ConnectionLost;
                                    }
                                }
                            }
                            ActiveStageOutput::GraphicsUpdate(region) => {
                                updated_regions.push(region);
                            }
                            ActiveStageOutput::Terminate(reason) => {
                                log::info!(
                                    "Session {} terminated by server: {}",
                                    self.id,
                                    reason
                                );
                                self.state = SessionState::Disconnected;
                                self.publish_info();
                                return DisconnectReason::UserDisconnect;
                            }
                            ActiveStageOutput::DeactivateAll(_cas) => {
                                log::info!("Session {} deactivate-all received", self.id);
                                // Server wants to re-negotiate capabilities; treat as disconnect
                                return DisconnectReason::ConnectionLost;
                            }
                            _ => {
                                // PointerDefault, PointerHidden, PointerPosition, PointerBitmap
                                // Ignored for now
                            }
                        }
                    }

                    if !updated_regions.is_empty() {
                        let rect_start = std::time::Instant::now();
                        let rect_count = updated_regions.len();

                        // Write dirty rects to SharedFrame for GPU renderer (batched — one lock)
                        if let Some(ref sf) = self.shared_frame {
                            use ironrdp::pdu::geometry::Rectangle as _;
                            let stride = image.stride();
                            let img_data = image.data();
                            let mut guard = sf.begin_write();
                            for rect in &updated_regions {
                                let x_offset = usize::from(rect.left) * 4;
                                let y_start = usize::from(rect.top);
                                let row_start = y_start * stride + x_offset;
                                if row_start < img_data.len() {
                                    guard.update_rect(
                                        rect.left,
                                        rect.top,
                                        rect.width(),
                                        rect.height(),
                                        &img_data[row_start..],
                                        stride,
                                    );
                                }
                            }
                            drop(guard);
                            sf.mark_dirty(); // Wakes render thread via condvar

                            let write_time = rect_start.elapsed();
                            if write_time.as_millis() > 2 {
                                log::debug!(
                                    "[Session {}] SharedFrame write: {} rects in {:.2}ms",
                                    self.id, rect_count, write_time.as_secs_f64() * 1000.0
                                );
                            }

                            // Track FPS at the session level
                            self.metrics.frames_sent += 1;
                            if self.metrics.frames_sent % 100 == 0 {
                                let elapsed = self.metrics.started_at.unwrap_or_else(std::time::Instant::now).elapsed();
                                let fps = self.metrics.frames_sent as f64 / elapsed.as_secs_f64();
                                log::info!(
                                    "[Session {}] RDP frames received: {} (avg {:.1} fps) | {} rects this PDU",
                                    self.id, self.metrics.frames_sent, fps, rect_count
                                );
                            }
                        }

                        // Always send via IPC — webview Canvas renders these
                        // (wgpu renders behind the webview, not visible yet)
                        let packet = encode_image_update_packet(
                            &image,
                            &updated_regions,
                            &mut self.frame_packet_scratch,
                        );
                        if self.send_frame_packet(packet).is_err() {
                            return DisconnectReason::ConnectionLost;
                        }
                    }

                    if self
                        .flush_clipboard_messages(&mut active_stage, &mut writer)
                        .await
                        .is_err()
                    {
                        return DisconnectReason::ConnectionLost;
                    }
                }

                // Process commands from the frontend
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(SessionCommand::Disconnect) | None => {
                            log::info!("Session {} received disconnect command", self.id);
                            // Try graceful shutdown
                            if let Ok(outputs) = active_stage.graceful_shutdown() {
                                for out in outputs {
                                    if let ActiveStageOutput::ResponseFrame(frame) = out {
                                        let _ = writer.write_all(&frame).await;
                                    }
                                }
                            }
                            self.state = SessionState::Disconnected;
                            self.publish_info();
                            return DisconnectReason::UserDisconnect;
                        }
                        Some(SessionCommand::SendKey { key_code, is_down }) => {
                            log::debug!(
                                "Session {} key event: code={}, down={}",
                                self.id, key_code, is_down
                            );

                            // Encode key event as FastPath input; events without a
                            // scancode (unmapped keys) are dropped silently.
                            if let Some(event) = encode_keyboard_fastpath_event(key_code, is_down) {
                                match active_stage.process_fastpath_input(&mut image, &[event]) {
                                    Ok(outputs) => {
                                        for out in outputs {
                                            if let ActiveStageOutput::ResponseFrame(frame) = out {
                                                if let Err(e) = writer.write_all(&frame).await {
                                                    log::warn!("Write error on key event: {}", e);
                                                    return DisconnectReason::ConnectionLost;
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::warn!("Failed to encode key event: {}", e);
                                    }
                                }
                            }
                        }
                        Some(SessionCommand::SendMouse { x, y, button, event_type, scroll_delta }) => {
                            log::debug!(
                                "Session {} mouse event: ({},{}) btn={:?} type={} scroll={}",
                                self.id, x, y, button, event_type, scroll_delta
                            );
                            // Encode mouse event as FastPath input
                            let event = encode_mouse_fastpath_event(
                                x,
                                y,
                                button.as_deref(),
                                event_type.as_str(),
                                scroll_delta,
                            );

                            match active_stage.process_fastpath_input(&mut image, &[event]) {
                                Ok(outputs) => {
                                    for out in outputs {
                                        if let ActiveStageOutput::ResponseFrame(frame) = out {
                                            if let Err(e) = writer.write_all(&frame).await {
                                                log::warn!("Write error on mouse event: {}", e);
                                                return DisconnectReason::ConnectionLost;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::warn!("Failed to encode mouse event: {}", e);
                                }
                            }
                        }
                        Some(SessionCommand::Resize { width, height }) => {
                            let (width, height) = normalize_remote_resolution(width, height);
                            log::info!("Session {} resize request to {}x{}", self.id, width, height);
                            self.width = width;
                            self.height = height;
                            self.publish_info();
                            // Try to send a display control resize via the DVC
                            if let Some(result) = active_stage.encode_resize(width, height, None, None) {
                                match result {
                                    Ok(frame) => {
                                        if let Err(e) = writer.write_all(&frame).await {
                                            log::warn!("Write error on resize: {}", e);
                                            return DisconnectReason::ConnectionLost;
                                        }
                                    }
                                    Err(e) => {
                                        log::warn!("Failed to encode resize: {}", e);
                                    }
                                }
                            }
                        }
                        Some(SessionCommand::ClipboardWrite { text }) => {
                            if self
                                .set_local_clipboard_text(&mut active_stage, &mut writer, text)
                                .await
                                .is_err()
                            {
                                return DisconnectReason::ConnectionLost;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Auto-reconnect loop with exponential backoff.
    async fn run_reconnect_loop(&mut self) {
        let mut attempt = 0u32;

        while attempt < MAX_RECONNECT_ATTEMPTS {
            attempt += 1;
            self.reconnect_attempts = attempt;
            self.state = SessionState::Reconnecting;
            self.publish_info();

            let delay = std::cmp::min(1u64 << attempt, 30); // 2, 4, 8, 16, 30 seconds
            log::info!(
                "Reconnect attempt {}/{} in {}s for session {}",
                attempt,
                MAX_RECONNECT_ATTEMPTS,
                delay,
                self.id
            );

            // Wait for the backoff delay, but check for disconnect commands
            let should_cancel = self
                .wait_with_cancel(std::time::Duration::from_secs(delay))
                .await;

            if should_cancel {
                log::info!("Reconnect cancelled for session {}", self.id);
                self.state = SessionState::Disconnected;
                self.publish_info();
                return;
            }

            // Attempt real reconnection
            let outcome = (self.connect)(
                self.config.clone(),
                self.password.clone(),
                self.width,
                self.height,
            )
            .await;

            match outcome {
                ConnectionOutcome::RealSession {
                    connection_result,
                    framed,
                } => {
                    self.state = SessionState::Connected;
                    self.reconnect_attempts = 0;
                    self.last_error = None;
                    self.width = connection_result.desktop_size.width as u32;
                    self.height = connection_result.desktop_size.height as u32;
                    self.publish_info();
                    log::info!(
                        "Reconnected session {} (real RDP) on attempt {}",
                        self.id,
                        attempt
                    );

                    let reason = self
                        .run_real_session_loop(*connection_result, *framed)
                        .await;
                    match reason {
                        DisconnectReason::UserDisconnect => return,
                        DisconnectReason::ConnectionLost => continue,
                    }
                }
                ConnectionOutcome::Failed(_reason) => {
                    // Fall back to mock on reconnect too
                    self.state = SessionState::Connected;
                    self.reconnect_attempts = 0;
                    self.last_error = None;
                    self.publish_info();
                    log::info!(
                        "Reconnected session {} (mock fallback) on attempt {}",
                        self.id,
                        attempt
                    );

                    let reason = self.run_mock_frame_loop().await;
                    match reason {
                        DisconnectReason::UserDisconnect => return,
                        DisconnectReason::ConnectionLost => continue,
                    }
                }
            }
        }

        // Exhausted all attempts
        let msg = format!(
            "Reconnection failed after {} attempts",
            MAX_RECONNECT_ATTEMPTS
        );
        self.state = SessionState::Error(msg.clone());
        self.last_error = Some(msg);
        self.reconnect_attempts = 0;
        self.publish_info();
    }

    /// Wait for a duration, but return early if a Disconnect command arrives.
    /// Returns `true` if we should cancel (disconnect received), `false` if the timer expired.
    async fn wait_with_cancel(&mut self, duration: std::time::Duration) -> bool {
        let sleep = tokio::time::sleep(duration);
        tokio::pin!(sleep);

        loop {
            tokio::select! {
                _ = &mut sleep => return false,
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(SessionCommand::Disconnect) | None => return true,
                        // Process other commands while waiting
                        Some(SessionCommand::Resize { width, height }) => {
                            let (width, height) = normalize_remote_resolution(width, height);
                            self.width = width;
                            self.height = height;
                            self.frame_buffer = FrameBuffer::generate_mock_frame(width, height);
                            self.publish_info();
                        }
                        Some(SessionCommand::ClipboardWrite { text }) => {
                            self.local_clipboard_text = Some(text);
                        }
                        _ => {} // Ignore key/mouse during reconnect
                    }
                }
            }
        }
    }

    /// Mock session loop: sends an initial full frame and refreshes on resize.
    /// Returns the reason the loop ended.
    async fn run_mock_frame_loop(&mut self) -> DisconnectReason {
        if self.send_mock_frame().is_err() {
            return DisconnectReason::ConnectionLost;
        }

        loop {
            match self.cmd_rx.recv().await {
                Some(SessionCommand::Disconnect) | None => {
                    log::info!("Session {} received disconnect command", self.id);
                    self.state = SessionState::Disconnected;
                    self.publish_info();
                    return DisconnectReason::UserDisconnect;
                }
                Some(SessionCommand::Resize { width, height }) => {
                    let (width, height) = normalize_remote_resolution(width, height);
                    log::info!("Session {} resized to {}x{}", self.id, width, height);
                    self.width = width;
                    self.height = height;
                    self.frame_buffer = FrameBuffer::generate_mock_frame(width, height);
                    self.publish_info();
                    if self.send_mock_frame().is_err() {
                        return DisconnectReason::ConnectionLost;
                    }
                }
                Some(SessionCommand::SendKey { key_code, is_down }) => {
                    log::debug!(
                        "Session {} key event: code={}, down={}",
                        self.id,
                        key_code,
                        is_down
                    );
                    // TODO: Forward to IronRDP input channel when using real session
                }
                Some(SessionCommand::SendMouse {
                    x,
                    y,
                    button,
                    event_type,
                    scroll_delta: _,
                }) => {
                    log::debug!(
                        "Session {} mouse event: ({},{}) btn={:?} type={}",
                        self.id,
                        x,
                        y,
                        button,
                        event_type
                    );
                    // TODO: Forward to IronRDP input channel when using real session
                }
                Some(SessionCommand::ClipboardWrite { text }) => {
                    self.local_clipboard_text = Some(text);
                }
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum DisconnectReason {
    UserDisconnect,
    ConnectionLost,
}

fn normalize_remote_resolution(width: u32, height: u32) -> (u32, u32) {
    ironrdp::displaycontrol::pdu::MonitorLayoutEntry::adjust_display_size(width, height)
}

/// Encode a frontend keyboard event as a FastPath keyboard event.
///
/// The frontend `key_code` packs the set-1 scancode in the low byte and an
/// "extended key" flag in bit 8 (`0x100`, set for e.g. right-side modifiers,
/// arrows, NumpadEnter). Modifiers are ordinary scancodes — the client does
/// not track modifier state, it forwards each modifier key-down/-up itself.
/// Returns `None` for events without a scancode (unmapped keys), which the
/// session loop drops without touching the transport.
fn encode_keyboard_fastpath_event(key_code: u32, is_down: bool) -> Option<FastPathInputEvent> {
    use ironrdp::pdu::input::fast_path::KeyboardFlags;

    let scancode = (key_code & 0xff) as u8;
    if scancode == 0 {
        return None;
    }

    let mut flags = KeyboardFlags::empty();
    if !is_down {
        flags |= KeyboardFlags::RELEASE;
    }
    if (key_code & 0x100) != 0 {
        flags |= KeyboardFlags::EXTENDED;
    }

    Some(FastPathInputEvent::KeyboardEvent(flags, scancode))
}

/// Encode a frontend pointer event as a FastPath mouse event.
///
/// Button values arrive as the strings "1"/"2"/"3" (from the TS mapper) or
/// "left"/"right"/"middle"; unknown values default to the left button and
/// unknown event types default to a move. Wheel deltas are clamped to
/// ±255 rotation units with `WHEEL_NEGATIVE` marking downward rotation.
fn encode_mouse_fastpath_event(
    x: i32,
    y: i32,
    button: Option<&str>,
    event_type: &str,
    scroll_delta: i32,
) -> FastPathInputEvent {
    use ironrdp::pdu::input::mouse::{MousePdu, PointerFlags};

    let mut flags = PointerFlags::empty();
    let mut wheel_units: i16 = 0;

    match event_type {
        "move" => {
            flags |= PointerFlags::MOVE;
        }
        "down" => {
            flags |= PointerFlags::DOWN;
            match button {
                Some("1") | Some("left") => flags |= PointerFlags::LEFT_BUTTON,
                Some("2") | Some("right") => flags |= PointerFlags::RIGHT_BUTTON,
                Some("3") | Some("middle") => flags |= PointerFlags::MIDDLE_BUTTON_OR_WHEEL,
                _ => flags |= PointerFlags::LEFT_BUTTON,
            }
        }
        "up" => match button {
            Some("1") | Some("left") => flags |= PointerFlags::LEFT_BUTTON,
            Some("2") | Some("right") => flags |= PointerFlags::RIGHT_BUTTON,
            Some("3") | Some("middle") => flags |= PointerFlags::MIDDLE_BUTTON_OR_WHEEL,
            _ => flags |= PointerFlags::LEFT_BUTTON,
        },
        "scroll" => {
            // Per MS-RDPBCGR spec:
            // PTRFLAGS_WHEEL (0x0200) = vertical wheel
            // PTRFLAGS_WHEEL_NEGATIVE (0x0100) = negative rotation
            // WheelRotationMask (0x01FF) = rotation units in bottom 9 bits
            flags |= PointerFlags::VERTICAL_WHEEL;
            let delta = scroll_delta.clamp(-255, 255);
            if delta < 0 {
                flags |= PointerFlags::WHEEL_NEGATIVE;
                wheel_units = (-delta) as i16;
            } else {
                wheel_units = delta as i16;
            }
        }
        _ => {
            flags |= PointerFlags::MOVE;
        }
    }

    FastPathInputEvent::MouseEvent(MousePdu {
        flags,
        number_of_wheel_rotation_units: wheel_units,
        x_position: x.max(0) as u16,
        y_position: y.max(0) as u16,
    })
}

/// Manages all active sessions using lightweight handles.
pub struct SessionManager {
    handles: Arc<Mutex<HashMap<String, SessionHandle>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            handles: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn session_count(&self) -> usize {
        self.handles.lock().await.len()
    }

    /// Spawn a new session actor and return its ID.
    pub async fn create_session(
        &self,
        config: ConnectionConfig,
        password: Option<String>,
        frame_channel: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
        app_handle: tauri::AppHandle,
        auto_reconnect: bool,
        shared_frame: Option<Arc<SharedFrame>>,
    ) -> Result<String, String> {
        self.create_session_with_transport(
            config,
            password,
            frame_channel,
            SessionEventSink::app(app_handle),
            auto_reconnect,
            shared_frame,
            real_connect(),
        )
        .await
    }

    /// `create_session` with the transport seams injected.
    /// Production callers use [`SessionManager::create_session`]; deterministic
    /// lifecycle tests script the connector and record frontend events here.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create_session_with_transport(
        &self,
        config: ConnectionConfig,
        password: Option<String>,
        frame_channel: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
        events: SessionEventSink,
        auto_reconnect: bool,
        shared_frame: Option<Arc<SharedFrame>>,
        connect: ConnectFn,
    ) -> Result<String, String> {
        let count = self.session_count().await;
        if count >= MAX_SESSIONS {
            return Err(format!(
                "Maximum session limit ({}) reached. Disconnect an existing session first.",
                MAX_SESSIONS
            ));
        }

        let session_id = Uuid::new_v4().to_string();
        let (width, height) = normalize_remote_resolution(
            config.display_width.unwrap_or(1920),
            config.display_height.unwrap_or(1080),
        );

        // Create channels
        let (cmd_tx, cmd_rx) = mpsc::channel::<SessionCommand>(64);

        // Build initial info for the watch channel
        let initial_info = SessionInfo {
            id: session_id.clone(),
            connection_id: config.id.clone(),
            connection_name: config.name.clone(),
            host: config.host.clone(),
            status: SessionState::Connecting,
            fps: 0.0,
            latency: 15,
            bandwidth: 0,
            width,
            height,
            connected_at: None,
            reconnect_attempts: 0,
            max_reconnect_attempts: MAX_RECONNECT_ATTEMPTS,
            last_error: None,
        };
        let (info_tx, info_rx) = watch::channel(initial_info);
        let (clipboard_tx, clipboard_rx) = watch::channel(None);

        // Create the actor
        let actor = SessionActor::new(
            session_id.clone(),
            config,
            password,
            cmd_rx,
            info_tx,
            clipboard_tx,
            frame_channel,
            events,
            auto_reconnect,
            shared_frame,
            connect,
        );

        // Spawn the actor task
        let task_handle = tokio::spawn(actor.run());

        // Store the handle
        let handle = SessionHandle {
            cmd_tx,
            info_rx,
            clipboard_rx,
            task_handle,
        };

        self.handles.lock().await.insert(session_id.clone(), handle);

        log::info!("Created session {}", session_id);
        Ok(session_id)
    }

    /// Send a disconnect command and remove the session handle.
    pub async fn remove_session(&self, session_id: &str) -> Result<(), String> {
        let mut handles = self.handles.lock().await;
        if let Some(handle) = handles.remove(session_id) {
            // Send disconnect command (ignore error if actor already stopped)
            let _ = handle.cmd_tx.send(SessionCommand::Disconnect).await;
            log::info!("Session {} removed", session_id);
            Ok(())
        } else {
            Err(format!("Session '{}' not found", session_id))
        }
    }

    /// Get a clone of the command sender for a session.
    pub async fn get_cmd_sender(&self, session_id: &str) -> Option<mpsc::Sender<SessionCommand>> {
        let handles = self.handles.lock().await;
        handles.get(session_id).map(|h| h.cmd_tx.clone())
    }

    /// Get the latest session info (lock-free read from watch channel).
    pub async fn get_session_info(&self, session_id: &str) -> Option<SessionInfo> {
        let handles = self.handles.lock().await;
        handles.get(session_id).map(|h| h.info_rx.borrow().clone())
    }

    /// List all sessions.
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        let handles = self.handles.lock().await;
        handles
            .values()
            .map(|h| h.info_rx.borrow().clone())
            .collect()
    }

    pub async fn get_remote_clipboard(&self, session_id: &str) -> Option<String> {
        let handles = self.handles.lock().await;
        handles
            .get(session_id)
            .and_then(|handle| handle.clipboard_rx.borrow().clone())
    }

    /// Get a snapshot of the current frame for a session (generates mock frame on demand).
    /// This is a fallback for the get_frame command.
    pub async fn get_frame(&self, session_id: &str) -> Option<String> {
        let handles = self.handles.lock().await;
        if let Some(handle) = handles.get(session_id) {
            let info = handle.info_rx.borrow().clone();
            // Generate a mock frame on demand with the session's current dimensions
            let fb = FrameBuffer::generate_mock_frame(info.width, info.height);
            Some(fb.to_base64_png())
        } else {
            None
        }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Characterization tests for the session actor baseline (task 3.2).
///
/// All tests are deterministic: the RDP connector is scripted through the
/// `ConnectFn` seam, frontend events are captured through the
/// `SessionEventSink::Recorded` seam, and frame-transport loss is driven by
/// scripted `tauri::ipc::Channel` callbacks (the production loss model is
/// "frame packet send failure = connection lost"). Time-dependent behavior
/// uses the tokio test-util paused clock with auto-advance; there are no
/// sleeps, no network I/O and no user-data access.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdp::client::ConnectionOutcome;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tauri::ipc::{Channel, InvokeResponseBody};

    /// Scripted frame-channel behavior.
    enum FrameScript {
        Ok,
        Fail,
        /// Only the send with this 1-based index fails.
        FailOnly(usize),
    }

    struct TestSession {
        id: String,
        cmd_tx: mpsc::Sender<SessionCommand>,
        events: Arc<RecordedEvents>,
        events_rx: watch::Receiver<usize>,
        frame_sizes: Arc<std::sync::Mutex<Vec<usize>>>,
        connect_calls: Arc<AtomicUsize>,
    }

    fn test_config(tag: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: format!("conn-{tag}"),
            name: format!("Conn {tag}"),
            // Never dialed: every test below scripts the connector.
            host: "session-actor-test.invalid".to_string(),
            port: 3389,
            username: "user".to_string(),
            domain: String::new(),
            color_accent: None,
            tags: vec![],
            display_width: Some(64),
            display_height: Some(64),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_connected_at: None,
        }
    }

    fn scripted_frame_channel(
        script: FrameScript,
        sizes: Arc<std::sync::Mutex<Vec<usize>>>,
    ) -> Channel<InvokeResponseBody> {
        let sent = Arc::new(AtomicUsize::new(0));
        Channel::new(move |response| {
            let n = sent.fetch_add(1, Ordering::SeqCst) + 1;
            let fail = match &script {
                FrameScript::Ok => false,
                FrameScript::Fail => true,
                FrameScript::FailOnly(idx) => n == *idx,
            };
            if fail {
                return Err(tauri::Error::AssetNotFound(format!(
                    "scripted frame-transport failure #{n}"
                )));
            }
            let size = match &response {
                InvokeResponseBody::Raw(bytes) => bytes.len(),
                InvokeResponseBody::Json(value) => value.to_string().len(),
            };
            sizes.lock().unwrap().push(size);
            Ok(())
        })
    }

    fn scripted_connect(calls: Arc<AtomicUsize>) -> ConnectFn {
        Arc::new(move |_config, _password, _width, _height| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { ConnectionOutcome::Failed("scripted connect failure".to_string()) })
        })
    }

    async fn create_scripted(
        manager: &SessionManager,
        tag: &str,
        auto_reconnect: bool,
        script: FrameScript,
    ) -> TestSession {
        let (events, events_rx) = RecordedEvents::new();
        let frame_sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let connect_calls = Arc::new(AtomicUsize::new(0));
        let id = manager
            .create_session_with_transport(
                test_config(tag),
                None,
                scripted_frame_channel(script, Arc::clone(&frame_sizes)),
                SessionEventSink::recorded(Arc::clone(&events)),
                auto_reconnect,
                None,
                scripted_connect(Arc::clone(&connect_calls)),
            )
            .await
            .expect("scripted session creation should succeed");
        let cmd_tx = manager
            .get_cmd_sender(&id)
            .await
            .expect("session handle exists right after creation");
        TestSession {
            id,
            cmd_tx,
            events,
            events_rx,
            frame_sizes,
            connect_calls,
        }
    }

    /// Session-info payloads recorded for this session, in publication order.
    fn info_events(
        id: &str,
        events: &Arc<RecordedEvents>,
    ) -> Vec<(tokio::time::Instant, serde_json::Value)> {
        let event_name = format!("session-info-{id}");
        events
            .snapshot()
            .into_iter()
            .filter(|(_, event, _)| *event == event_name)
            .map(|(at, _, value)| (at, value))
            .collect()
    }

    /// `SessionState` serializes as camelCase strings; `Error(String)` as
    /// `{"error": msg}`.
    fn status_string(info: &serde_json::Value) -> String {
        match &info["status"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(map) => format!(
                "error:{}",
                map.get("error").and_then(|v| v.as_str()).unwrap_or("?")
            ),
            other => format!("{other}"),
        }
    }

    fn statuses_of(recorded: &[(tokio::time::Instant, serde_json::Value)]) -> Vec<String> {
        recorded.iter().map(|(_, v)| status_string(v)).collect()
    }

    /// Await (without polling or sleeping) until recorded session-info events
    /// satisfy `pred`. The recorder notifies through a watch channel, so the
    /// paused clock only advances when both tasks are parked.
    async fn wait_for(
        id: &str,
        events: &Arc<RecordedEvents>,
        events_rx: &mut watch::Receiver<usize>,
        pred: impl Fn(&[(tokio::time::Instant, serde_json::Value)]) -> bool,
    ) {
        loop {
            if pred(&info_events(id, events)) {
                return;
            }
            events_rx.changed().await.expect("event recorder alive");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_admission_admits_tenth_rejects_eleventh_and_frees_slot_on_removal() {
        let manager = SessionManager::new();
        let mut sessions = Vec::new();
        for i in 1..=10u32 {
            let session =
                create_scripted(&manager, &format!("adm{i}"), true, FrameScript::Ok).await;
            assert_eq!(
                manager.session_count().await,
                i as usize,
                "sessions 1..=10 must be admitted"
            );
            sessions.push(session);
        }

        // The 11th connect is rejected with an explanatory message (Req 2.2).
        let (events, _rx) = RecordedEvents::new();
        let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let err = manager
            .create_session_with_transport(
                test_config("adm11"),
                None,
                scripted_frame_channel(FrameScript::Ok, sizes),
                SessionEventSink::recorded(events),
                true,
                None,
                scripted_connect(Arc::new(AtomicUsize::new(0))),
            )
            .await
            .expect_err("the 11th concurrent session must be rejected");
        assert_eq!(
            err,
            "Maximum session limit (10) reached. Disconnect an existing session first."
        );
        assert_eq!(manager.session_count().await, 10);

        // Teardown race: removal frees the admission slot immediately, even
        // though the removed actor may still be shutting down.
        manager
            .remove_session(&sessions[0].id)
            .await
            .expect("removing a live session succeeds");
        assert_eq!(manager.session_count().await, 9);
        create_scripted(&manager, "adm12", true, FrameScript::Ok).await;
        assert_eq!(manager.session_count().await, 10);
        assert_eq!(manager.list_sessions().await.len(), 10);

        // Unknown ids report not found.
        assert_eq!(
            manager.remove_session("no-such-session").await.unwrap_err(),
            "Session 'no-such-session' not found"
        );
        assert!(manager.get_session_info("no-such-session").await.is_none());

        // The removed session's actor observes the disconnect command and
        // publishes its final disconnected state.
        let removed = &sessions[0];
        let mut rx = removed.events_rx.clone();
        wait_for(&removed.id, &removed.events, &mut rx, |events| {
            events
                .iter()
                .any(|(_, v)| status_string(v) == "disconnected")
        })
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn test_transport_loss_retries_capped_backoff_then_error_state_is_transient_defect_b3_8()
    {
        let manager = SessionManager::new();
        let session = create_scripted(&manager, "recon", true, FrameScript::Fail).await;
        let mut rx = session.events_rx.clone();

        wait_for(&session.id, &session.events, &mut rx, |events| {
            events
                .iter()
                .any(|(_, v)| status_string(v) == "disconnected")
        })
        .await;

        let recorded = info_events(&session.id, &session.events);
        // connecting → connected (mock fallback) → MAX_RECONNECT_ATTEMPTS ×
        // (reconnecting → connected via mock fallback) → terminal error →
        // final disconnected.
        let mut expected: Vec<&str> = vec!["connecting", "connected"];
        for _ in 0..MAX_RECONNECT_ATTEMPTS {
            expected.push("reconnecting");
            expected.push("connected");
        }
        expected.push("error:Reconnection failed after 5 attempts");
        expected.push("disconnected");
        assert_eq!(statuses_of(&recorded), expected);

        // One initial attempt plus exactly MAX_RECONNECT_ATTEMPTS retries.
        assert_eq!(session.connect_calls.load(Ordering::SeqCst), 6);

        // Capped exponential backoff measured on the paused clock:
        // 2, 4, 8, 16, 30 seconds (the 6th doubling is capped at 30).
        let backoffs: Vec<std::time::Duration> = recorded
            .windows(2)
            .filter(|pair| {
                status_string(&pair[0].1) == "reconnecting"
                    && status_string(&pair[1].1) == "connected"
            })
            .map(|pair| pair[1].0 - pair[0].0)
            .collect();
        let expected_backoffs: Vec<std::time::Duration> = [2u64, 4, 8, 16, 30]
            .iter()
            .map(|s| std::time::Duration::from_secs(*s))
            .collect();
        assert_eq!(backoffs, expected_backoffs);

        // Defect B3-8: the terminal error state is published and then
        // immediately overwritten by "disconnected" in the same watch
        // channel; the final resting status is disconnected while the error
        // reason survives only in last_error.
        let (_, last) = recorded.last().expect("final event recorded");
        assert_eq!(status_string(last), "disconnected");
        assert_eq!(
            last["lastError"].as_str(),
            Some("Reconnection failed after 5 attempts")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_failed_reconnect_falls_back_to_mock_as_connected_defect_b3_7() {
        let manager = SessionManager::new();
        // Frame send #1 (initial mock frame) succeeds; send #2 (triggered by
        // resize) fails, modeling transport loss on an otherwise healthy sink.
        let session = create_scripted(&manager, "fallback", true, FrameScript::FailOnly(2)).await;
        let mut rx = session.events_rx.clone();
        let sizes = Arc::clone(&session.frame_sizes);

        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                && !sizes.lock().unwrap().is_empty()
        })
        .await;

        // The second frame send (resize) hits the scripted transport loss.
        session
            .cmd_tx
            .send(SessionCommand::Resize {
                width: 64,
                height: 64,
            })
            .await
            .expect("resize command accepted");

        // The reconnect attempt also fails (scripted connector), yet the
        // session reports connected again via mock frames: defect B3-7.
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events
                .iter()
                .any(|(_, v)| status_string(v) == "reconnecting")
                && events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
        })
        .await;

        let recorded = info_events(&session.id, &session.events);
        assert_eq!(
            statuses_of(&recorded),
            vec![
                "connecting".to_string(),
                "connected".to_string(),
                "connected".to_string(), // resize re-publish
                "reconnecting".to_string(),
                "connected".to_string(),
            ]
        );
        // Defect B3-7: with the server unreachable, retries do NOT run to the
        // cap; exactly one reconnect attempt runs and the failure is masked
        // by the mock fallback.
        assert_eq!(session.connect_calls.load(Ordering::SeqCst), 2);
        // The reconnecting event reports attempt 1 of 5 …
        let (_, reconnecting) = recorded
            .iter()
            .find(|(_, v)| status_string(v) == "reconnecting")
            .expect("reconnecting event recorded");
        assert_eq!(reconnecting["reconnectAttempts"].as_u64(), Some(1));
        assert_eq!(reconnecting["maxReconnectAttempts"].as_u64(), Some(5));
        // … and the mock fallback clears the error surface entirely.
        let (_, settled) = recorded.last().expect("settled event recorded");
        assert_eq!(settled["reconnectAttempts"].as_u64(), Some(0));
        assert_eq!(settled["lastError"].as_str(), None);
        assert_eq!(
            session.frame_sizes.lock().unwrap().len(),
            2,
            "mock frames resumed after the fallback"
        );

        // The session still answers a user disconnect normally.
        session
            .cmd_tx
            .send(SessionCommand::Disconnect)
            .await
            .expect("disconnect accepted");
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
        })
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn test_cancel_during_backoff_stops_retrying_and_marks_disconnected() {
        let manager = SessionManager::new();
        let session = create_scripted(&manager, "cancel", true, FrameScript::Fail).await;
        let mut rx = session.events_rx.clone();

        // First scripted transport loss → first reconnect backoff (2 s on the
        // paused clock).
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events
                .iter()
                .any(|(_, v)| status_string(v) == "reconnecting")
        })
        .await;

        // Cancel while parked in the backoff wait. This is the same command
        // the disconnect/cancel_reconnect commands deliver.
        session
            .cmd_tx
            .send(SessionCommand::Disconnect)
            .await
            .expect("cancel delivered during backoff");

        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
        })
        .await;

        let recorded = info_events(&session.id, &session.events);
        assert_eq!(
            statuses_of(&recorded),
            vec![
                "connecting".to_string(),
                "connected".to_string(),
                "reconnecting".to_string(),
                "disconnected".to_string(),
                "disconnected".to_string(),
            ]
        );
        // Cancellation stopped retrying before any reconnect attempt ran.
        assert_eq!(session.connect_calls.load(Ordering::SeqCst), 1);
        // The backoff never completed: no virtual time passed beyond the
        // cancellation point (Req 2.4 — stop retrying).
        let first = recorded.first().expect("first event recorded");
        let last = recorded.last().expect("last event recorded");
        assert!(
            last.0 - first.0 < std::time::Duration::from_secs(2),
            "cancel must preempt the first backoff, elapsed={:?}",
            last.0 - first.0
        );
        // Session state remains observable with the loss reason (Req 2.5).
        assert!(
            last.1["lastError"]
                .as_str()
                .unwrap_or_default()
                .starts_with("Frame emit failed"),
            "last error should surface the transport loss, got {:?}",
            last.1["lastError"]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_user_disconnect_from_connected_session_publishes_disconnected() {
        let manager = SessionManager::new();
        let session = create_scripted(&manager, "user", false, FrameScript::Ok).await;
        let mut rx = session.events_rx.clone();
        let sizes = Arc::clone(&session.frame_sizes);

        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                && !sizes.lock().unwrap().is_empty()
        })
        .await;

        let recorded = info_events(&session.id, &session.events);
        assert_eq!(
            statuses_of(&recorded),
            vec!["connecting".to_string(), "connected".to_string()]
        );

        // Req 2.5: SessionInfo surfaces identity, progress and error fields.
        let (_, connected) = recorded.last().expect("connected event recorded");
        assert_eq!(connected["status"].as_str(), Some("connected"));
        assert_eq!(connected["connectionId"].as_str(), Some("conn-user"));
        assert_eq!(connected["reconnectAttempts"].as_u64(), Some(0));
        assert_eq!(connected["maxReconnectAttempts"].as_u64(), Some(5));
        assert_eq!(connected["lastError"].as_str(), None);
        assert!(connected["connectedAt"].as_str().is_some());

        session
            .cmd_tx
            .send(SessionCommand::Disconnect)
            .await
            .expect("disconnect accepted");
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
        })
        .await;

        let recorded = info_events(&session.id, &session.events);
        assert_eq!(
            statuses_of(&recorded),
            vec![
                "connecting".to_string(),
                "connected".to_string(),
                "disconnected".to_string(),
                "disconnected".to_string(),
            ]
        );
        assert_eq!(
            session.frame_sizes.lock().unwrap().len(),
            1,
            "no additional frames after disconnect"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dropping_all_command_senders_stops_the_actor() {
        let manager = SessionManager::new();
        let session = create_scripted(&manager, "drop", true, FrameScript::Ok).await;
        let mut rx = session.events_rx.clone();
        let sizes = Arc::clone(&session.frame_sizes);

        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                && !sizes.lock().unwrap().is_empty()
        })
        .await;

        // Teardown race: dropping the manager and the last sender clones
        // closes the command channel; the actor treats channel closure as a
        // user-initiated disconnect and publishes its final state.
        let id = session.id.clone();
        let events = Arc::clone(&session.events);
        let last_sender = manager
            .get_cmd_sender(&session.id)
            .await
            .expect("sender clone for teardown");
        drop(manager);
        drop(last_sender);
        drop(session.cmd_tx);

        wait_for(&id, &events, &mut rx, |event_list| {
            event_list.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
        })
        .await;
        assert_eq!(
            statuses_of(&info_events(&id, &events)),
            vec![
                "connecting".to_string(),
                "connected".to_string(),
                "disconnected".to_string(),
                "disconnected".to_string(),
            ]
        );
    }

    // ── Task 3.3: input and display controls (Req 3.1–3.3) ──────────────

    fn mouse_pdu(event: FastPathInputEvent) -> ironrdp::pdu::input::MousePdu {
        match event {
            FastPathInputEvent::MouseEvent(pdu) => pdu,
            other => panic!("expected a mouse event, got {:?}", other),
        }
    }

    /// Req 3.1 — keyboard events are forwarded as scancodes; key-up adds
    /// RELEASE and the `0x100` bit adds EXTENDED. Modifiers are ordinary
    /// scancodes (right-hand variants only differ by EXTENDED).
    #[test]
    fn test_keyboard_encoding_scancode_release_and_extended_flags() {
        use ironrdp::pdu::input::fast_path::KeyboardFlags;

        // 'A' down: plain scancode, no flags.
        assert_eq!(
            encode_keyboard_fastpath_event(0x1e, true),
            Some(FastPathInputEvent::KeyboardEvent(
                KeyboardFlags::empty(),
                0x1e
            ))
        );
        // 'A' up: RELEASE only.
        assert_eq!(
            encode_keyboard_fastpath_event(0x1e, false),
            Some(FastPathInputEvent::KeyboardEvent(
                KeyboardFlags::RELEASE,
                0x1e
            ))
        );
        // Right Ctrl (0x11d) and Right Alt (0x138): same scancode as the left
        // variants, plus EXTENDED.
        assert_eq!(
            encode_keyboard_fastpath_event(0x11d, true),
            Some(FastPathInputEvent::KeyboardEvent(
                KeyboardFlags::EXTENDED,
                0x1d
            ))
        );
        assert_eq!(
            encode_keyboard_fastpath_event(0x138, false),
            Some(FastPathInputEvent::KeyboardEvent(
                KeyboardFlags::EXTENDED | KeyboardFlags::RELEASE,
                0x38
            ))
        );
        // Left Shift (0x2a) is a plain scancode without EXTENDED.
        assert_eq!(
            encode_keyboard_fastpath_event(0x2a, true),
            Some(FastPathInputEvent::KeyboardEvent(
                KeyboardFlags::empty(),
                0x2a
            ))
        );
    }

    /// Req 3.1 — key codes without a scancode (TS mapper returns 0 for
    /// unmapped keys) are dropped silently at the actor seam: no event, no
    /// transport write, no disconnect.
    #[test]
    fn test_keyboard_encoding_drops_keycode_zero_silently() {
        assert_eq!(encode_keyboard_fastpath_event(0, true), None);
        // The extended flag alone does not rescue a zero scancode.
        assert_eq!(encode_keyboard_fastpath_event(0x100, false), None);
    }

    /// Req 3.2 — pointer button/type encoding: down carries DOWN plus the
    /// button bit, up carries only the button bit, numeric and named button
    /// strings are equivalent, unknown values default to left/move.
    #[test]
    fn test_mouse_encoding_button_flags_for_down_up_move_and_unknown() {
        use ironrdp::pdu::input::mouse::PointerFlags;

        let down_left = mouse_pdu(encode_mouse_fastpath_event(10, 20, Some("1"), "down", 0));
        assert!(down_left.flags.contains(PointerFlags::DOWN));
        assert!(down_left.flags.contains(PointerFlags::LEFT_BUTTON));
        assert!(!down_left.flags.contains(PointerFlags::RIGHT_BUTTON));
        assert_eq!(down_left.x_position, 10);
        assert_eq!(down_left.y_position, 20);

        // Button-up carries the button bit without DOWN.
        let up_right = mouse_pdu(encode_mouse_fastpath_event(10, 20, Some("2"), "up", 0));
        assert!(!up_right.flags.contains(PointerFlags::DOWN));
        assert!(up_right.flags.contains(PointerFlags::RIGHT_BUTTON));

        // Named buttons are equivalent to the numeric TS mapping.
        let down_middle = mouse_pdu(encode_mouse_fastpath_event(1, 2, Some("middle"), "down", 0));
        assert!(down_middle.flags.contains(PointerFlags::DOWN));
        assert!(down_middle
            .flags
            .contains(PointerFlags::MIDDLE_BUTTON_OR_WHEEL));

        // Unknown button defaults to left; unknown event type defaults to move.
        let unknown_button = mouse_pdu(encode_mouse_fastpath_event(1, 2, None, "down", 0));
        assert!(unknown_button.flags.contains(PointerFlags::DOWN));
        assert!(unknown_button.flags.contains(PointerFlags::LEFT_BUTTON));
        let unknown_type = mouse_pdu(encode_mouse_fastpath_event(1, 2, Some("1"), "hover", 0));
        assert!(unknown_type.flags.contains(PointerFlags::MOVE));
        assert!(!unknown_type.flags.contains(PointerFlags::DOWN));

        // Move never carries a button bit.
        let plain_move = mouse_pdu(encode_mouse_fastpath_event(3, 4, Some("1"), "move", 0));
        assert!(plain_move.flags.contains(PointerFlags::MOVE));
        assert!(!plain_move.flags.contains(PointerFlags::LEFT_BUTTON));
    }

    /// Req 3.2 — wheel deltas clamp to ±255 rotation units; negative deltas
    /// set WHEEL_NEGATIVE with the magnitude as units.
    #[test]
    fn test_mouse_encoding_wheel_units_clamp_and_negative_flag() {
        use ironrdp::pdu::input::mouse::PointerFlags;

        let scroll_up = mouse_pdu(encode_mouse_fastpath_event(0, 0, None, "scroll", 120));
        assert!(scroll_up.flags.contains(PointerFlags::VERTICAL_WHEEL));
        assert!(!scroll_up.flags.contains(PointerFlags::WHEEL_NEGATIVE));
        assert_eq!(scroll_up.number_of_wheel_rotation_units, 120);

        let scroll_down = mouse_pdu(encode_mouse_fastpath_event(0, 0, None, "scroll", -30));
        assert!(scroll_down.flags.contains(PointerFlags::VERTICAL_WHEEL));
        assert!(scroll_down.flags.contains(PointerFlags::WHEEL_NEGATIVE));
        assert_eq!(scroll_down.number_of_wheel_rotation_units, 30);

        // Clamped to the 9-bit wheel rotation range.
        let big_up = mouse_pdu(encode_mouse_fastpath_event(0, 0, None, "scroll", 999));
        assert_eq!(big_up.number_of_wheel_rotation_units, 255);
        assert!(!big_up.flags.contains(PointerFlags::WHEEL_NEGATIVE));
        let big_down = mouse_pdu(encode_mouse_fastpath_event(0, 0, None, "scroll", -999));
        assert_eq!(big_down.number_of_wheel_rotation_units, 255);
        assert!(big_down.flags.contains(PointerFlags::WHEEL_NEGATIVE));

        // A zero delta still produces a zero-unit wheel event.
        let zero = mouse_pdu(encode_mouse_fastpath_event(0, 0, None, "scroll", 0));
        assert!(zero.flags.contains(PointerFlags::VERTICAL_WHEEL));
        assert_eq!(zero.number_of_wheel_rotation_units, 0);
    }

    /// Req 3.2 — negative coordinates clamp to 0, but values above `u16::MAX`
    /// wrap modulo 65536 instead of clamping: the IPC boundary accepts any
    /// `i32` and the truncated coordinate silently relocates the pointer.
    /// Pinned as defect B3-9 (the TS mapper clamps today, the backend does not).
    #[test]
    fn test_mouse_encoding_clamps_negative_and_wraps_large_coordinates_defect_b3_9() {
        let clamped = mouse_pdu(encode_mouse_fastpath_event(-5, -1, Some("1"), "move", 0));
        assert_eq!(clamped.x_position, 0);
        assert_eq!(clamped.y_position, 0);

        let wrapped = mouse_pdu(encode_mouse_fastpath_event(
            65536 + 7,
            65536 + 9,
            Some("1"),
            "move",
            0,
        ));
        assert_eq!(
            wrapped.x_position, 7,
            "B3-9: x wraps mod 65536 (expected 7)"
        );
        assert_eq!(
            wrapped.y_position, 9,
            "B3-9: y wraps mod 65536 (expected 9)"
        );
    }

    /// Req 3.3 — remote resolution changes are normalized to the
    /// MS-RDPEDI display-control bounds before use: width becomes even and
    /// both axes clamp to [200, 8192].
    #[test]
    fn test_normalize_remote_resolution_even_width_and_display_bounds() {
        assert_eq!(normalize_remote_resolution(0, 0), (200, 200));
        // Odd width is decremented before clamping, so 199 -> 198 -> 200.
        assert_eq!(normalize_remote_resolution(199, 199), (200, 200));
        assert_eq!(normalize_remote_resolution(201, 201), (200, 201));
        assert_eq!(normalize_remote_resolution(8191, 8192), (8190, 8192));
        assert_eq!(normalize_remote_resolution(9999, 9999), (8192, 8192));
        assert_eq!(normalize_remote_resolution(641, 481), (640, 481));
        assert_eq!(normalize_remote_resolution(200, 200), (200, 200));
    }

    /// Req 3.1/3.2 — the mock frame loop (pre-real-session fallback) accepts
    /// key/mouse/clipboard commands but does not forward them anywhere: no
    /// frames, no state change, no disconnect. This is the documented TODO
    /// zone deferred from task 3.2; mock sessions can never prove input
    /// forwarding (Req 12.9 — real-server check only).
    #[tokio::test(start_paused = true)]
    async fn test_mock_session_accepts_input_commands_without_frames_or_disconnect() {
        let manager = SessionManager::new();
        let session = create_scripted(&manager, "mock-input", true, FrameScript::Ok).await;
        let mut rx = session.events_rx.clone();
        let sizes = Arc::clone(&session.frame_sizes);

        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                && !sizes.lock().unwrap().is_empty()
        })
        .await;

        let frames_before = sizes.lock().unwrap().len();

        session
            .cmd_tx
            .send(SessionCommand::SendKey {
                key_code: 0x1e,
                is_down: true,
            })
            .await
            .expect("key command accepted");
        session
            .cmd_tx
            .send(SessionCommand::SendMouse {
                x: 1,
                y: 2,
                button: Some("1".to_string()),
                event_type: "down".to_string(),
                scroll_delta: 0,
            })
            .await
            .expect("mouse command accepted");
        session
            .cmd_tx
            .send(SessionCommand::ClipboardWrite {
                text: "local text".to_string(),
            })
            .await
            .expect("clipboard command accepted");

        // Ordering barrier: the resize that follows is only processed after
        // the commands above, proving the actor survived them all. The size
        // must sit above the 200 px display-control minimum (the session's
        // 64x64 config already normalized up to 200x200 at creation).
        session
            .cmd_tx
            .send(SessionCommand::Resize {
                width: 320,
                height: 240,
            })
            .await
            .expect("resize command accepted");
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events
                .last()
                .is_some_and(|(_, v)| v["width"].as_u64() == Some(320))
        })
        .await;

        // The actor publishes before it sends the refresh frame, so a
        // disconnect barrier (processed strictly after the resize arm) makes
        // the frame count deterministic without sleeps.
        session
            .cmd_tx
            .send(SessionCommand::Disconnect)
            .await
            .expect("disconnect accepted");
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
        })
        .await;

        // Only the resize refresh produced a frame; input commands produced
        // none and never disturbed the session state.
        assert_eq!(sizes.lock().unwrap().len(), frames_before + 1);
        assert_eq!(
            statuses_of(&info_events(&session.id, &session.events)),
            vec![
                "connecting".to_string(),
                "connected".to_string(),
                "connected".to_string(), // resize re-publish
                "disconnected".to_string(),
                "disconnected".to_string(),
            ],
            "input commands never change mock-session state"
        );
        // Clipboard sync is a real-session-only seam: the mock loop stores
        // the text but never publishes a remote clipboard update.
        assert_eq!(
            manager.get_remote_clipboard(&session.id).await,
            None,
            "mock sessions never publish remote clipboard text"
        );
    }

    /// Req 3.3 — resize commands are normalized (even width, [200, 8192])
    /// before the new dimensions are published and the mock frame refresh is
    /// sent; a repeat resize to the same size still refreshes.
    #[tokio::test(start_paused = true)]
    async fn test_actor_resize_normalizes_before_publish_and_refreshes_mock_frame() {
        let manager = SessionManager::new();
        let session = create_scripted(&manager, "resize-norm", true, FrameScript::Ok).await;
        let mut rx = session.events_rx.clone();
        let sizes = Arc::clone(&session.frame_sizes);

        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                && !sizes.lock().unwrap().is_empty()
        })
        .await;

        session
            .cmd_tx
            .send(SessionCommand::Resize {
                width: 201,
                height: 9999,
            })
            .await
            .expect("resize command accepted");

        // 201 -> 200 (even), 9999 -> 8192 (clamped) before publication.
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().is_some_and(|(_, v)| {
                v["width"].as_u64() == Some(200) && v["height"].as_u64() == Some(8192)
            })
        })
        .await;

        // Repeating the (already normalized) size still refreshes. The frame
        // send follows the publish, so the disconnect barrier below makes the
        // frame count deterministic.
        session
            .cmd_tx
            .send(SessionCommand::Resize {
                width: 200,
                height: 8192,
            })
            .await
            .expect("repeat resize accepted");
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.len() >= 4
        })
        .await;

        // Sub-minimum sizes clamp UP to the 200 px display-control floor
        // (the same rule that lifted this session's 64x64 config to 200x200
        // at creation).
        session
            .cmd_tx
            .send(SessionCommand::Resize {
                width: 128,
                height: 96,
            })
            .await
            .expect("sub-minimum resize accepted");
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().is_some_and(|(_, v)| {
                v["width"].as_u64() == Some(200) && v["height"].as_u64() == Some(200)
            })
        })
        .await;

        session
            .cmd_tx
            .send(SessionCommand::Disconnect)
            .await
            .expect("disconnect accepted");
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
        })
        .await;

        assert_eq!(
            sizes.lock().unwrap().len(),
            4,
            "each resize refreshed the frame"
        );

        let settled = info_events(&session.id, &session.events)
            .last()
            .map(|(_, v)| v.clone())
            .expect("settled event recorded");
        assert_eq!(settled["width"].as_u64(), Some(200));
        assert_eq!(settled["height"].as_u64(), Some(200));
    }

    /// Req 3.6 — the backend frame snapshot (`get_frame`) returns a base64
    /// PNG tracking the session's current dimensions. It fabricates a mock
    /// frame on demand rather than capturing the live frame content (see the
    /// source comment); real-content capture is a real-server-only check.
    #[tokio::test(start_paused = true)]
    async fn test_get_frame_returns_base64_png_tracking_session_dimensions() {
        use base64::Engine as _;

        let manager = SessionManager::new();
        let session = create_scripted(&manager, "get-frame", true, FrameScript::Ok).await;
        let mut rx = session.events_rx.clone();
        let sizes = Arc::clone(&session.frame_sizes);

        wait_for(&session.id, &session.events, &mut rx, |events| {
            events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                && !sizes.lock().unwrap().is_empty()
        })
        .await;

        session
            .cmd_tx
            .send(SessionCommand::Resize {
                width: 320,
                height: 240,
            })
            .await
            .expect("resize command accepted");
        wait_for(&session.id, &session.events, &mut rx, |events| {
            events
                .last()
                .is_some_and(|(_, v)| v["width"].as_u64() == Some(320))
        })
        .await;

        let frame_b64 = manager
            .get_frame(&session.id)
            .await
            .expect("live session has a frame snapshot");
        let png = base64::engine::general_purpose::STANDARD
            .decode(frame_b64.as_bytes())
            .expect("snapshot is valid base64");
        assert_eq!(
            &png[0..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            "snapshot is a PNG"
        );
        let decoded = image::load_from_memory(&png).expect("snapshot PNG decodes");
        use image::GenericImageView as _;
        assert_eq!(decoded.dimensions(), (320, 240));

        assert!(
            manager.get_frame("no-such-session").await.is_none(),
            "unknown sessions have no snapshot"
        );
    }
}
