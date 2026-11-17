use chrono::{DateTime, Utc};
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::{Mutex, mpsc, watch};
use uuid::Uuid;

use crate::rdp::client::{ConnectionOutcome, SessionCommand, attempt_rdp_connection};
use crate::rdp::clipboard::SessionClipboardBackend;
use crate::rdp::codec_policy::CodecErrorPolicy;
use crate::rdp::display::FrameBuffer;
use crate::rdp::frame_batch;
use crate::rdp::frame_delivery::{FrameDelivery, FrameReceipt};
use crate::rdp::frame_transport::{
    encode_full_frame_packet, encode_full_frame_packet_from_parts, encode_image_update_packet,
};
// Task 5.4 (Req 12.4/12.10): frame-kind spy module, compiled ONLY in
// test-support builds.
#[cfg(feature = "test-support")]
use crate::rdp::frame_spy;
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
    pub display_stalled: bool,
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

/// Task 5.3 (Req 12.2/12.10): evaluate a mock-session request at the connect
/// command boundary.
///
/// `persisted_mock` is the saved connection's `mock` field; `requested_mock`
/// is the command-level flag. Either selecting mock means a synthetic
/// session request:
///
/// - **test-support builds**: `Ok(true)` admits the request; the actor runs
///   the existing mock loop and never dials the connector (no network).
/// - **production builds**: the request is rejected. The allow-path is not
///   compiled in, so no environment variable or persisted field can bypass
///   the gate (Req 12.10).
///
/// `Ok(false)` means no mock was requested: proceed as a real connection.
pub fn admit_mock_session(
    persisted_mock: bool,
    requested_mock: Option<bool>,
) -> Result<bool, String> {
    if !persisted_mock && requested_mock != Some(true) {
        return Ok(false);
    }
    #[cfg(feature = "test-support")]
    {
        // Test-support build: synthetic sessions are available by design.
        Ok(true)
    }
    #[cfg(not(feature = "test-support"))]
    {
        // Production build: reject persisted and command mock flags alike.
        Err("Mock sessions are only available in test-support builds; this production build rejects mock-session requests".to_string())
    }
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
    clipboard_epoch: String,
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
    delivery: FrameDelivery,
    cmd_rx: mpsc::Receiver<SessionCommand>,
    info_tx: watch::Sender<SessionInfo>,
    clipboard_tx: watch::Sender<Option<String>>,
    frame_channel: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
    events: SessionEventSink,
    local_clipboard_text: Option<String>,
    reconnect_attempts: u32,
    last_error: Option<String>,
    auto_reconnect: bool,
    pending_resize: Option<(u32, u32)>,
    connect: ConnectFn,
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
            delivery: FrameDelivery::new(),
            cmd_rx,
            info_tx,
            clipboard_tx,
            frame_channel,
            events,
            local_clipboard_text: None,
            clipboard_epoch: Uuid::new_v4().to_string(),
            reconnect_attempts: 0,
            last_error: None,
            auto_reconnect,
            pending_resize: None,
            connect,
            shared_frame,
        }
    }

    /// Build a SessionInfo snapshot from the current actor state.
    fn build_info(&self) -> SessionInfo {
        SessionInfo {
            display_stalled: self.delivery.stalled,
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
        if let Err(error) = self.events.emit(
            &format!("clipboard-{}", self.id),
            serde_json::json!({"epoch": self.clipboard_epoch, "text": text}),
        ) {
            log::warn!(
                "Failed to emit clipboard event for session {}: {}",
                self.id,
                error
            );
        }
    }

    fn publish_clipboard_failure(&self, message: &str) {
        let _ = self.events.emit(
            &format!("clipboard-error-{}", self.id),
            serde_json::json!({"epoch": self.clipboard_epoch, "message": message}),
        );
    }

    fn send_frame_packet(
        &mut self,
        receipt: FrameReceipt,
        packet: Vec<u8>,
    ) -> Result<(), DisconnectReason> {
        // Metadata and binary use the SAME ordered channel. Binary packet bytes
        // remain unchanged; independent event ordering cannot misbind receipts.
        let metadata = serde_json::json!({ "kind": "frameDelivery", "sessionId": self.id,
            "generation": receipt.generation, "sequence": receipt.sequence });
        if self
            .frame_channel
            .send(tauri::ipc::InvokeResponseBody::Json(metadata.to_string()))
            .is_err()
        {
            self.delivery.failed_send();
            return Err(DisconnectReason::ConnectionLost);
        }
        // Task 5.4 (Req 12.4): the kind byte is captured before the send
        // consumes the buffer; the counting hook below is compiled out of
        // production builds entirely.
        #[cfg(feature = "test-support")]
        let emitted_kind = packet.first().copied();
        match self
            .frame_channel
            .send(tauri::ipc::InvokeResponseBody::Raw(packet))
        {
            Ok(()) => {
                // Task 5.4 (Req 12.4): test-support builds may count the
                // kinds that actually crossed the frame channel here.
                // Production builds compile this hook out, so the emission
                // path and its failure handling are unchanged there
                // (Req 12.10).
                #[cfg(feature = "test-support")]
                if let Some(kind) = emitted_kind {
                    self.observe_emitted_frame_kind(kind);
                }
                Ok(())
            }
            Err(error) => {
                self.delivery.failed_send();
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

    /// Task 5.4 (Req 12.4): frame-kind spy hook, compiled ONLY into
    /// test-support builds. When the spy env var (see `frame_spy`) activates
    /// it, each actually-emitted packet kind is counted in the process-wide
    /// spy registry and the cumulative counts are published as a
    /// per-session Tauri event so the E2E harness can assert the
    /// bitmap-only wire policy. Production builds contain neither this hook
    /// nor the event (Req 12.10); the frame channel send above is untouched
    /// either way.
    #[cfg(feature = "test-support")]
    fn observe_emitted_frame_kind(&self, kind: u8) {
        let Some(counts) = frame_spy::record_emission(&self.id, kind) else {
            return; // spy inactive: no recording, no event
        };
        let event_name = frame_spy::spy_event_name(&self.id);
        if let Err(error) = self.events.emit(&event_name, counts) {
            log::debug!(
                "Failed to emit frame-spy event for session {}: {}",
                self.id,
                error
            );
        }
    }

    /// Send the mock frame as a decoded RGBA bitmap packet (kinds 1/2 only;
    /// the H.264 transcode hop was removed by task 4.4, Req 9.4).
    fn send_mock_frame(&mut self) -> Result<(), DisconnectReason> {
        let surface = (self.width as u16, self.height as u16);
        self.delivery.damage(
            surface,
            frame_batch::BatchPlan {
                full_refresh: true,
                rect: ironrdp_pdu::geometry::InclusiveRectangle {
                    left: 0,
                    top: 0,
                    right: surface.0 - 1,
                    bottom: surface.1 - 1,
                },
            },
        );
        self.flush_mock_frame()
    }

    fn flush_mock_frame(&mut self) -> Result<(), DisconnectReason> {
        let Some((receipt, _)) = self.delivery.take(tokio::time::Instant::now()) else {
            return Ok(());
        };
        let packet = encode_full_frame_packet(&self.frame_buffer);
        self.send_frame_packet(receipt, packet)
    }

    fn flush_image_frame(
        &mut self,
        image: &ironrdp_session::image::DecodedImage,
    ) -> Result<(), DisconnectReason> {
        let Some((receipt, plan)) = self.delivery.take(tokio::time::Instant::now()) else {
            return Ok(());
        };
        let packet = if plan.full_refresh {
            encode_full_frame_packet_from_parts(image.width(), image.height(), image.data())
        } else {
            encode_image_update_packet(image, &[plan.rect], &mut self.frame_packet_scratch)
        };
        self.send_frame_packet(receipt, packet)
    }

    fn check_display_stall(&mut self) {
        if self.delivery.check_stall(tokio::time::Instant::now()) {
            self.publish_info();
        }
    }

    async fn flush_clipboard_messages<W>(
        &mut self,
        active_stage: &mut ironrdp_session::ActiveStage,
        writer: &mut W,
    ) -> Result<(), DisconnectReason>
    where
        W: ironrdp_tokio::FramedWrite,
    {
        use ironrdp_cliprdr::{
            Client as CliprdrRoleClient, CliprdrClient, CliprdrSvcMessages,
            backend::ClipboardMessage,
        };

        let (pending_messages, remote_text_update, failure) = {
            let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() else {
                return Ok(());
            };
            let Some(backend) = cliprdr.downcast_backend_mut::<SessionClipboardBackend>() else {
                return Ok(());
            };

            (
                backend.take_pending_messages(),
                backend.take_remote_text_update(),
                backend.take_failure(),
            )
        };

        if let Some(error) = failure {
            self.publish_clipboard_failure(&error);
        }

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
                    // cliprdr 0.9 file-transfer variants: the plain-text backend
                    // never queues them (file clipboard sync is out of scope), so
                    // skipping them cannot change observable behavior.
                    ClipboardMessage::SendFileContentsRequest(_)
                    | ClipboardMessage::SendFileContentsResponse(_)
                    | ClipboardMessage::SendInitiateFileCopy(_) => {
                        log::debug!("Ignoring file-clipboard message for session {}", self.id);
                        continue;
                    }
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
                        if let Some(backend) =
                            cliprdr.downcast_backend_mut::<SessionClipboardBackend>()
                        {
                            backend.fail_pending("Clipboard channel rejected the operation. Reconnect to restore the channel.");
                        }
                        self.publish_clipboard_failure("Clipboard channel rejected the operation. Reconnect to restore the channel.");
                        return Ok(());
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
                    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>()
                        && let Some(backend) =
                            cliprdr.downcast_backend_mut::<SessionClipboardBackend>()
                    {
                        backend.fail_pending(
                            "Clipboard encoding failed. Reconnect to restore the channel.",
                        );
                    }
                    self.publish_clipboard_failure(
                        "Clipboard encoding failed. Reconnect to restore the channel.",
                    );
                    return Ok(());
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
        active_stage: &mut ironrdp_session::ActiveStage,
        writer: &mut W,
        text: String,
    ) -> Result<(), DisconnectReason>
    where
        W: ironrdp_tokio::FramedWrite,
    {
        use ironrdp_cliprdr::CliprdrClient;

        self.local_clipboard_text = Some(text.clone());

        if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>()
            && let Some(backend) = cliprdr.downcast_backend_mut::<SessionClipboardBackend>()
        {
            let (reply, _result) = tokio::sync::oneshot::channel();
            backend.begin_local(text, reply);
        }

        self.flush_clipboard_messages(active_stage, writer).await
    }

    /// Task 5.3: a synthetic (mock) session executes only in test-support
    /// builds. The feature check is defense in depth on top of the command
    /// boundary rejection ([`admit_mock_session`]): a production build can
    /// never select mock execution, even from a hand-edited connections.json.
    #[cfg(feature = "test-support")]
    fn mock_enabled(&self) -> bool {
        self.config.mock && cfg!(feature = "test-support")
    }

    /// Resolve the next connection outcome. Synthetic sessions short-circuit
    /// to the mock path WITHOUT invoking the connector seam (no network,
    /// Req 12.2); real sessions dial exactly as before.
    async fn connect_outcome(&mut self) -> Option<ConnectionOutcome> {
        #[cfg(feature = "test-support")]
        if self.mock_enabled() {
            return Some(ConnectionOutcome::Synthetic);
        }
        let mut connect = (self.connect)(
            self.config.clone(),
            self.password.clone(),
            self.width,
            self.height,
        );
        // Preserve initial-connect ownership; reconnect cancellation must
        // remain responsive even while the next handshake is pending.
        if self.state == SessionState::Connecting {
            return Some(connect.await);
        }
        loop {
            tokio::select! {
                // A queued cancellation wins over a concurrently completed dial.
                biased;
                cmd = self.cmd_rx.recv() => match cmd {
                    Some(SessionCommand::Disconnect) | None => {
                        self.state = SessionState::Disconnected;
                        self.publish_info();
                        return None;
                    }
                    Some(SessionCommand::FrameConsumed { receipt, refresh }) => {
                        self.delivery.acknowledge(&receipt, refresh);
                    }
                    Some(SessionCommand::Resize { width, height }) => {
                        let (width, height) = normalize_remote_resolution(width, height);
                        self.pending_resize = Some((width, height));
                    }
                    Some(SessionCommand::ClipboardContext { .. }) => {}
                Some(SessionCommand::ClipboardPaste { reply, .. }) => { let _ = reply.send(Err("Session is not connected".into())); }
                Some(SessionCommand::ClipboardSync { reply, .. }) => { let _ = reply.send(Err("Clipboard requires a connected real session.".into())); }
                Some(SessionCommand::ClipboardWrite { text }) => {
                        self.local_clipboard_text = Some(text);
                    }
                    _ => {}
                },
                outcome = &mut connect => return Some(outcome),
            }
        }
    }

    /// Main actor loop.
    async fn run(mut self) {
        // Publish initial "Connecting" state
        self.publish_info();

        // Step 1: Attempt RDP connection; failures remain visible to the user.
        log::info!(
            "Session {} attempting RDP connection to {}:{}",
            self.id,
            self.config.host,
            self.config.port
        );

        let Some(outcome) = self.connect_outcome().await else {
            return;
        };

        // Both real-session variants share one actor path; only the framed
        // transport differs (production TLS vs the in-memory test seam).
        // Task 9.3: both carry the session's shared codec-error policy —
        // created at connector attach time, reported into by the EGFX
        // handler, sampled by the session loop below.
        let real_session = match outcome {
            ConnectionOutcome::RealSession {
                connection_result,
                framed,
                codec_policy,
            } => Some((
                *connection_result,
                SessionTransport::Tls(framed),
                codec_policy,
            )),
            ConnectionOutcome::RealSessionLocal {
                connection_result,
                framed,
                codec_policy,
            } => Some((
                *connection_result,
                SessionTransport::Local(framed),
                codec_policy,
            )),
            ConnectionOutcome::Failed(reason) => {
                self.state = SessionState::Error(reason.clone());
                self.last_error = Some(reason);
                self.publish_info();
                return;
            }
            #[cfg(any(test, feature = "test-support"))]
            ConnectionOutcome::Synthetic => None,
        };

        match real_session {
            Some((connection_result, transport, codec_policy)) => {
                log::info!("Session {} entered real RDP session", self.id);
                self.state = SessionState::Connected;
                self.connected_at = Some(Utc::now());

                // Update dimensions from the server's negotiated desktop size
                self.width = connection_result.desktop_size.width as u32;
                self.height = connection_result.desktop_size.height as u32;
                self.publish_info();

                // Run the real RDP session loop
                let disconnect_reason = self
                    .run_real_session_loop(connection_result, transport, codec_policy)
                    .await;

                if disconnect_reason == DisconnectReason::ConnectionLost && self.auto_reconnect {
                    self.run_reconnect_loop().await;
                }
            }
            None => {
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

        // Terminal errors remain observable after the actor has stopped.
        if !matches!(self.state, SessionState::Error(_)) {
            self.state = SessionState::Disconnected;
        }
        self.publish_info();
        log::info!("Session {} actor stopped", self.id);
    }

    /// Real RDP session loop: reads PDUs from the server, decodes graphics,
    /// and emits frame data to the frontend.
    async fn run_real_session_loop(
        &mut self,
        connection_result: ironrdp_connector::ConnectionResult,
        transport: SessionTransport,
        codec_policy: Arc<CodecErrorPolicy>,
    ) -> DisconnectReason {
        // 0.13 session API: the stage is built through the connector adapter
        // (ActiveStage::new(ConnectionResult) no longer exists upstream).
        // The activation factory is cloned out first: task 7.4 keeps it for
        // the in-place Deactivation-Reactivation Sequence (its Config and MCS
        // channel ids are invariant for the whole connection lifetime).
        let activation_factory = connection_result.activation_factory.clone();
        let active_stage =
            crate::rdp::client::active_stage_from_connection_result(connection_result);

        // Both transports split identically: upstream's `split_tokio_framed`
        // only needs `AsyncRead + AsyncWrite + Unpin + Sync`, which the TLS
        // stream and the in-memory duplex both satisfy.
        match transport {
            SessionTransport::Tls(framed) => {
                let (reader, writer) = ironrdp_tokio::split_tokio_framed(*framed);
                self.run_framed_session_loop(
                    activation_factory,
                    active_stage,
                    reader,
                    writer,
                    codec_policy,
                )
                .await
            }
            SessionTransport::Local(framed) => {
                let (reader, writer) = ironrdp_tokio::split_tokio_framed(*framed);
                self.run_framed_session_loop(
                    activation_factory,
                    active_stage,
                    reader,
                    writer,
                    codec_policy,
                )
                .await
            }
        }
    }

    /// Transport-generic real session loop.
    ///
    /// Split into its own function (task 7.4) so the production TLS framing
    /// and the in-memory characterization seam run byte-for-byte the same
    /// session logic, including in-place reactivation on DeactivateAll.
    async fn run_framed_session_loop<SR, SW>(
        &mut self,
        activation_factory: ironrdp_connector::connection_activation::ConnectionActivationFactory,
        mut active_stage: ironrdp_session::ActiveStage,
        mut reader: ironrdp_tokio::Framed<SR>,
        mut writer: ironrdp_tokio::Framed<SW>,
        codec_policy: Arc<CodecErrorPolicy>,
    ) -> DisconnectReason
    where
        SR: ironrdp_tokio::FramedRead,
        SW: ironrdp_tokio::FramedWrite,
    {
        use ironrdp_graphics::image_processing::PixelFormat;
        use ironrdp_session::ActiveStageOutput;
        use ironrdp_session::image::DecodedImage;
        use ironrdp_tokio::FramedWrite;
        let mut image =
            DecodedImage::new(PixelFormat::RgbA32, self.width as u16, self.height as u16);

        // Task 9.4 (design L316): surface dims already delivered to the UI
        // on this connection. `None` until the first frame — the initial
        // delivery is a full kind-1 frame — and reset by construction on
        // every (re)connect because this loop (and its decoded image)
        // starts fresh per connection.
        let mut delivered_surface: Option<(u16, u16)> = None;
        let mut held_modifiers = std::collections::BTreeSet::new();
        self.delivery.reset();

        self.clipboard_epoch = Uuid::new_v4().to_string();
        self.local_clipboard_text = None;
        let _ = self.clipboard_tx.send(None);

        'session: loop {
            if let Some(result) = encode_pending_resize(&mut self.pending_resize, &mut active_stage)
            {
                match result {
                    Ok(frame) => {
                        if let Err(error) = writer.write_all(&frame).await {
                            self.last_error = Some(format!("Resize write failed: {error}"));
                            return DisconnectReason::ConnectionLost;
                        }
                    }
                    Err(error) => {
                        self.last_error = Some(format!("Resize request failed: {error}"));
                        self.publish_info();
                    }
                }
            }
            tokio::select! {
                _ = crate::rdp::frame_delivery::wait_for_stall(self.delivery.deadline()) => self.check_display_stall(),
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

                    // `received_at: None` preserves the pre-0.13 behavior: the
                    // timestamp only feeds upstream's new bandwidth auto-detect.
                    let outputs = match active_stage.process(&mut image, action, &payload, None) {
                        Ok(outputs) => outputs,
                        Err(e) => {
                            log::warn!("Session {} process error: {}", self.id, e);
                            self.last_error = Some(format!("Session error: {}", e));
                            self.state = SessionState::Error(format!("Session error: {}", e));
                            self.publish_info();
                            return DisconnectReason::ConnectionLost;
                        }
                    };

                    // Task 9.3 (Req 9.6/9.7): terminal codec/decode error
                    // check. The EGFX handler (inside the stage's graphics
                    // pipeline) has already reported unsupported-codec
                    // arrivals during `process` above; sample upstream's
                    // decode-failure counter exactly like the dependency
                    // spike proved (`dropped_updates()` deltas per processed
                    // PDU) and apply the three-consecutive-strikes policy.
                    // A terminal error is SESSION-TERMINAL: publish the
                    // error and stop WITHOUT the transport-drop reconnect
                    // path — the configuration is incompatible, retrying it
                    // would repeat the failure (Req 9.6; transport loss
                    // stays `ConnectionLost` for the Req 2.3 reconnect).
                    if let Some(terminal) = codec_policy.evaluate_decode_outcome(
                        active_stage
                            .get_dvc_mut::<ironrdp_egfx::client::GraphicsPipelineClient>()
                            .map(|mut gfx| gfx.processor_mut().dropped_updates()),
                    ) {
                        log::warn!(
                            "Session {} terminal codec/decode error: {}",
                            self.id, terminal
                        );
                        self.last_error = Some(terminal.clone());
                        self.state = SessionState::Error(terminal);
                        self.publish_info();
                        return DisconnectReason::CodecTerminal;
                    }

                    let mut updated_regions = Vec::new();
                    let mut deactivate_all = false;
                    for out in outputs {
                        match out {
                            ActiveStageOutput::ResponseFrame(frame) => {
                                if !frame.is_empty()
                                    && let Err(e) = writer.write_all(&frame).await {
                                        log::warn!("Session {} write error: {}", self.id, e);
                                        self.last_error = Some(format!("Write error: {}", e));
                                        return DisconnectReason::ConnectionLost;
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
                            ActiveStageOutput::DeactivateAll => {
                                log::info!(
                                    "Session {} deactivate-all received; reactivating in place",
                                    self.id
                                );
                                // Task 7.4 (user decision 2026-11-14): adopt
                                // the proven upstream mechanism instead of
                                // the task 4.3 recorded behavior
                                // (DeactivateAll → ConnectionLost → the
                                // backoff reconnect loop; see
                                // baseline-coverage.md task 4.3). The
                                // sequence is driven below, over the SAME
                                // transport and stage, so session identity
                                // never changes. Unexpected transport drops
                                // (Req 2.3) and cancel during reconnect
                                // (Req 2.4) keep their existing semantics —
                                // a reactivation that cannot complete still
                                // unwinds as ConnectionLost.
                                deactivate_all = true;
                                break;
                            }
                            _ => {
                                // PointerDefault, PointerHidden, PointerPosition, PointerBitmap
                                // Ignored for now
                            }
                        }
                    }

                    if deactivate_all {
                        // A replacement share invalidates old dirty coordinates,
                        // even when its dimensions stay the same. Keep only the
                        // occupied transport slot; its receipt can drain that
                        // slot but cannot flush pixels from the previous share.
                        self.delivery.reset();
                        delivered_surface = None;
                        match self
                            .drive_reactivation(
                                &activation_factory,
                                &mut active_stage,
                                &mut image,
                                &mut reader,
                                &mut writer,
                            )
                            .await
                        {
                            ReactivationOutcome::Reactivated => {
                                // Skip the pending region packet and the
                                // clipboard flush: pre-deactivation image
                                // content is stale until the reactivated
                                // share redraws.
                                continue 'session;
                            }
                            ReactivationOutcome::Stop(reason) => return reason,
                        }
                    }

                    if !updated_regions.is_empty() {
                        // Task 9.4 (design L316; Reqs 4.1/4.4/9.8): merge
                        // every dirty region drained from this processed
                        // PDU batch into ONE bounding rectangle and choose
                        // the packet kind — kind 1 (full frame) for the
                        // initial delivery on this surface, a resize, or
                        // full-surface invalidation (upstream
                        // `widen_to_full_refresh` after a graphics reset);
                        // otherwise kind 2 with the single merged rect. The
                        // payload is extracted from the authoritative
                        // composited decoded image, so unchanged pixels
                        // inside the bounds come from the framebuffer and
                        // accumulated dirty regions are never lost.
                        let surface = (image.width(), image.height());
                        if (self.width, self.height) != (u32::from(surface.0), u32::from(surface.1)) {
                            self.width = u32::from(surface.0);
                            self.height = u32::from(surface.1);
                            self.publish_info();
                        }
                        if let Some(plan) =
                            frame_batch::plan_batch(&updated_regions, surface, delivered_surface)
                        {
                            let rect_start = std::time::Instant::now();
                            let region_count = updated_regions.len();

                            // Write the merged damage to the SharedFrame for
                            // the GPU renderer (one lock, one rect)
                            if let Some(ref sf) = self.shared_frame {
                                use ironrdp_pdu::geometry::Rectangle as _;
                                let stride = image.stride();
                                let img_data = image.data();
                                let mut guard = sf.begin_write();
                                // Task 9.2 (Req 4.1/4.2): an EGFX
                                // graphics-output reset resizes the decoded
                                // image upstream (`ResetGraphics` →
                                // `reset_preserving_pointer` inside
                                // `ActiveStage::process`). Resize the
                                // SharedFrame with it and repaint in full
                                // so the GPU surface and the webview canvas
                                // never see a mixed-size buffer. Task 9.4
                                // extends the full repaint to every
                                // full-refresh plan (initial delivery,
                                // full-surface invalidation); partial
                                // batches write their single merged
                                // bounding rect — unchanged pixels inside
                                // the bounds copy from the composited
                                // image (identical values).
                                if plan.full_refresh
                                    || guard.width() != u32::from(surface.0)
                                    || guard.height() != u32::from(surface.1)
                                {
                                    guard.update_full(
                                        u32::from(surface.0),
                                        u32::from(surface.1),
                                        img_data,
                                    );
                                } else {
                                    let row_start = usize::from(plan.rect.top) * stride
                                        + usize::from(plan.rect.left) * 4;
                                    if row_start < img_data.len() {
                                        guard.update_rect(
                                            plan.rect.left,
                                            plan.rect.top,
                                            plan.rect.width(),
                                            plan.rect.height(),
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
                                        "[Session {}] SharedFrame write: {} regions in {:.2}ms",
                                        self.id, region_count, write_time.as_secs_f64() * 1000.0
                                    );
                                }

                                // Track FPS at the session level
                                self.metrics.frames_sent += 1;
                                if self.metrics.frames_sent.is_multiple_of(100) {
                                    let elapsed = self
                                        .metrics
                                        .started_at
                                        .unwrap_or_else(std::time::Instant::now)
                                        .elapsed();
                                    let fps =
                                        self.metrics.frames_sent as f64 / elapsed.as_secs_f64();
                                    log::info!(
                                        "[Session {}] RDP frames received: {} (avg {:.1} fps) | {} regions this PDU",
                                        self.id, self.metrics.frames_sent, fps, region_count
                                    );
                                }
                            }

                            self.delivery.damage(surface, plan);
                            if self.flush_image_frame(&image).is_err() {
                                return DisconnectReason::ConnectionLost;
                            }
                            delivered_surface = Some(surface);
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
                        Some(SessionCommand::FrameConsumed { receipt, refresh }) => {
                            let was_stalled = self.delivery.stalled;
                            if self.delivery.acknowledge(&receipt, refresh) {
                                if was_stalled { self.publish_info(); }
                                if self.flush_image_frame(&image).is_err() { return DisconnectReason::ConnectionLost; }
                            }
                        }
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
                                            if let ActiveStageOutput::ResponseFrame(frame) = out
                                                && let Err(e) = writer.write_all(&frame).await {
                                                    log::warn!("Write error on key event: {}", e);
                                                    return DisconnectReason::ConnectionLost;
                                                }
                                        }
                                        if matches!(key_code, 0x1d | 0x11d | 0x2a | 0x36 | 0x38 | 0x138 | 0x15b | 0x15c) {
                                            if is_down { held_modifiers.insert(key_code); } else { held_modifiers.remove(&key_code); }
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
                                        if let ActiveStageOutput::ResponseFrame(frame) = out
                                            && let Err(e) = writer.write_all(&frame).await {
                                                log::warn!("Write error on mouse event: {}", e);
                                                return DisconnectReason::ConnectionLost;
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
                            self.pending_resize = Some((width, height));
                        }
                        Some(SessionCommand::ClipboardPaste { epoch, reply }) => {
                            if epoch != self.clipboard_epoch || !self.config.host_profile.clipboard_direction.sends() { let _ = reply.send(Err("Clipboard session or policy changed.".into())); continue; }
                            let keys: Vec<_> = clipboard_paste_sequence(&held_modifiers).into_iter().filter_map(|(code, down)| encode_keyboard_fastpath_event(code, down)).collect();
                            match active_stage.process_fastpath_input(&mut image, &keys) {
                                Ok(outputs) => {
                                    for output in outputs { if let ActiveStageOutput::ResponseFrame(frame) = output && writer.write_all(&frame).await.is_err() { let _ = reply.send(Err("Clipboard paste transport failed".into())); return DisconnectReason::ConnectionLost; } }
                                    let _ = reply.send(Ok(()));
                                }
                                Err(_) => { let _ = reply.send(Err("Remote paste could not be encoded. Retry resync.".into())); }
                            }
                        }
                        Some(SessionCommand::ClipboardContext { reply }) => { let _ = reply.send(crate::rdp::clipboard::ClipboardContext { epoch: self.clipboard_epoch.clone(), direction: self.config.host_profile.clipboard_direction }); }
                        Some(SessionCommand::ClipboardSync { epoch, text, reply }) => {
                            if epoch != self.clipboard_epoch { let _ = reply.send(Err("Session changed. Retry clipboard in the active connection.".into())); continue; }
                            use ironrdp_cliprdr::CliprdrClient;
                            if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                                if !cliprdr.is_ready() {
                                    let _ = reply.send(Err("Clipboard channel is not ready. Retry shortly or reconnect.".into()));
                                } else if let Some(backend) = cliprdr.downcast_backend_mut::<SessionClipboardBackend>() {
                                    match text { Some(text) => backend.begin_local(text, reply), None => backend.begin_remote(reply) }
                                }
                            } else { let _ = reply.send(Err("Clipboard channel is unavailable. Reconnect the session.".into())); }
                            if self.flush_clipboard_messages(&mut active_stage, &mut writer).await.is_err() {
                                return DisconnectReason::ConnectionLost;
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
                        // Task 5.3: the loss control targets synthetic
                        // sessions only; on a real session the (real)
                        // transport alone decides connection loss.
                        #[cfg(feature = "test-support")]
                        Some(SessionCommand::DropConnection) => {
                            log::warn!(
                                "Session {} ignored test loss control on a real session",
                                self.id
                            );
                        }
                    }
                }
            }
        }
    }

    /// Drive the upstream Deactivation-Reactivation Sequence in place
    /// (task 7.4; MS-RDPBCGR §1.3.1.3).
    ///
    /// A fresh `ConnectionActivationSequence` is produced from the
    /// connection's `ConnectionActivationFactory` (its Config and MCS
    /// channel ids are invariant for the connection's whole lifetime) and
    /// driven over the SAME transport — the pattern upstream's own sequence
    /// drivers use (`step` when the sequence wants a server PDU,
    /// `step_no_input` when it has unsolicited client PDUs to send). On
    /// Finalized the still-existing `ActiveStage` is rearmed via
    /// `ActiveStage::reactivate` (new share id, pointer settings and
    /// negotiated chunk size), so the session keeps its identity: no
    /// ConnectionLost, no reconnecting/backoff cycle.
    ///
    /// Failures (transport drop, protocol error, server redirection, user
    /// disconnect) surface as [`ReactivationOutcome::Stop`]; the caller maps
    /// `ConnectionLost` into the existing reconnect loop, preserving Req
    /// 2.3/2.4 semantics for genuinely broken transports.
    async fn drive_reactivation<SR, SW>(
        &mut self,
        factory: &ironrdp_connector::connection_activation::ConnectionActivationFactory,
        active_stage: &mut ironrdp_session::ActiveStage,
        image: &mut ironrdp_session::image::DecodedImage,
        reader: &mut ironrdp_tokio::Framed<SR>,
        writer: &mut ironrdp_tokio::Framed<SW>,
    ) -> ReactivationOutcome
    where
        SR: ironrdp_tokio::FramedRead,
        SW: ironrdp_tokio::FramedWrite,
    {
        use ironrdp_connector::Sequence as _;
        use ironrdp_connector::connection_activation::ConnectionActivationState;
        use ironrdp_graphics::image_processing::PixelFormat;
        use ironrdp_session::image::DecodedImage;
        use ironrdp_tokio::FramedWrite as _;

        let mut sequence = factory.create();
        let mut buf = ironrdp_core::WriteBuf::new();
        log::info!(
            "Session {} driving deactivation-reactivation sequence",
            self.id
        );

        loop {
            buf.clear();

            // `received_at: None` preserves the pre-0.13 behavior used by
            // the surrounding session loop (timestamps only feed upstream's
            // bandwidth auto-detect).
            let written = if sequence.next_pdu_hint().is_some() {
                // Wait for the next server PDU while staying responsive to
                // commands (cancel mirrors Req 2.4's intent).
                let (_action, payload) = tokio::select! {
                    _ = crate::rdp::frame_delivery::wait_for_stall(self.delivery.deadline()) => {
                        self.check_display_stall();
                        continue;
                    }
                    pdu = reader.read_pdu() => match pdu {
                        Ok(pdu) => pdu,
                        Err(e) => {
                            log::warn!("Session {} reactivation read error: {}", self.id, e);
                            return ReactivationOutcome::Stop(DisconnectReason::ConnectionLost);
                        }
                    },
                    cmd = self.cmd_rx.recv() => {
                        match cmd {
                            Some(SessionCommand::FrameConsumed { receipt, refresh }) => {
                                let was_stalled = self.delivery.stalled;
                                if self.delivery.acknowledge(&receipt, refresh) && was_stalled {
                                    self.publish_info();
                                }
                                continue;
                            }
                            Some(SessionCommand::Disconnect) | None => {
                                log::info!(
                                    "Session {} disconnect requested during reactivation",
                                    self.id
                                );
                                self.state = SessionState::Disconnected;
                                self.publish_info();
                                return ReactivationOutcome::Stop(
                                    DisconnectReason::UserDisconnect,
                                );
                            }
                            Some(SessionCommand::Resize { width, height }) => {
                                self.pending_resize = Some(normalize_remote_resolution(width, height));
                                continue;
                            }
                            // Input against a deactivated share cannot be
                            // encoded; the window is a handful of PDUs.
                            Some(other) => {
                                log::debug!(
                                    "Session {} dropping command during reactivation: {:?}",
                                    self.id,
                                    other
                                );
                                continue;
                            }
                        }
                    }
                };

                match sequence.step(&payload, None, &mut buf) {
                    Ok(written) => written,
                    Err(e) => {
                        log::warn!("Session {} reactivation sequence error: {}", self.id, e);
                        self.last_error = Some(format!("Reactivation failed: {}", e));
                        return ReactivationOutcome::Stop(DisconnectReason::ConnectionLost);
                    }
                }
            } else {
                match sequence.step_no_input(&mut buf) {
                    Ok(written) => written,
                    Err(e) => {
                        log::warn!("Session {} reactivation sequence error: {}", self.id, e);
                        self.last_error = Some(format!("Reactivation failed: {}", e));
                        return ReactivationOutcome::Stop(DisconnectReason::ConnectionLost);
                    }
                }
            };

            if let Some(len) = written.size()
                && let Err(e) = writer.write_all(&buf.filled()[..len]).await
            {
                log::warn!("Session {} reactivation write error: {}", self.id, e);
                return ReactivationOutcome::Stop(DisconnectReason::ConnectionLost);
            }

            match sequence.connection_activation_state() {
                ConnectionActivationState::Finalized {
                    desktop_size,
                    share_id,
                    static_channel_chunk_size,
                    enable_server_pointer,
                    pointer_software_rendering,
                    ..
                } => {
                    let rearmed = active_stage.reactivate(
                        factory.io_channel_id(),
                        factory.user_channel_id(),
                        share_id,
                        enable_server_pointer,
                        pointer_software_rendering,
                        static_channel_chunk_size,
                    );

                    if !rearmed {
                        log::warn!(
                            "Session {} refused invalid negotiated chunk size during reactivation",
                            self.id
                        );
                        return ReactivationOutcome::Stop(DisconnectReason::ConnectionLost);
                    }

                    let new_width = u32::from(desktop_size.width);
                    let new_height = u32::from(desktop_size.height);
                    if new_width != self.width || new_height != self.height {
                        self.width = new_width;
                        self.height = new_height;
                        *image = DecodedImage::new(
                            PixelFormat::RgbA32,
                            desktop_size.width,
                            desktop_size.height,
                        );
                        self.publish_info();
                    }

                    log::info!(
                        "Session {} reactivated in place (share id {})",
                        self.id,
                        share_id
                    );
                    return ReactivationOutcome::Reactivated;
                }
                // A mid-session redirect cannot be honored without tearing
                // this transport down; same policy as the connect path
                // (redirect → error), so fall back to the reconnect loop.
                ConnectionActivationState::Redirected { .. } => {
                    log::warn!("Session {} server redirection during reactivation", self.id);
                    return ReactivationOutcome::Stop(DisconnectReason::ConnectionLost);
                }
                _ => {}
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

            // Attempt real reconnection (synthetic sessions re-admit the
            // mock loop without dialing — see connect_outcome).
            let Some(outcome) = self.connect_outcome().await else {
                return;
            };

            let real_session = match outcome {
                ConnectionOutcome::RealSession {
                    connection_result,
                    framed,
                    codec_policy,
                } => Some((
                    *connection_result,
                    SessionTransport::Tls(framed),
                    codec_policy,
                )),
                ConnectionOutcome::RealSessionLocal {
                    connection_result,
                    framed,
                    codec_policy,
                } => Some((
                    *connection_result,
                    SessionTransport::Local(framed),
                    codec_policy,
                )),
                ConnectionOutcome::Failed(reason) => {
                    self.last_error = Some(reason);
                    continue;
                }
                #[cfg(any(test, feature = "test-support"))]
                ConnectionOutcome::Synthetic => None,
            };

            match real_session {
                Some((connection_result, transport, codec_policy)) => {
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
                        .run_real_session_loop(connection_result, transport, codec_policy)
                        .await;
                    match reason {
                        DisconnectReason::UserDisconnect => return,
                        DisconnectReason::ConnectionLost => continue,
                        // Task 9.3: a codec/decode terminal inside a
                        // reconnected session is still terminal — the
                        // incompatible-configuration rule (Req 9.6) applies
                        // per connection, not per attempt.
                        DisconnectReason::CodecTerminal => return,
                    }
                }
                None => {
                    // Explicit synthetic session in a test build.
                    self.state = SessionState::Connected;
                    self.reconnect_attempts = 0;
                    self.last_error = None;
                    self.publish_info();
                    log::info!(
                        "Reconnected session {} (synthetic) on attempt {}",
                        self.id,
                        attempt
                    );

                    let reason = self.run_mock_frame_loop().await;
                    match reason {
                        DisconnectReason::UserDisconnect => return,
                        DisconnectReason::ConnectionLost => continue,
                        // Unreachable from the mock loop (no EGFX traffic),
                        // but a terminal stays terminal by construction.
                        DisconnectReason::CodecTerminal => return,
                    }
                }
            }
        }

        // Exhausted all attempts
        let msg = format!(
            "Reconnection failed after {} attempts: {}",
            MAX_RECONNECT_ATTEMPTS,
            self.last_error.as_deref().unwrap_or("connection lost")
        );
        self.state = SessionState::Error(msg.clone());
        self.last_error = Some(msg);
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
                        Some(SessionCommand::FrameConsumed { receipt, refresh }) => {
                            self.delivery.acknowledge(&receipt, refresh);
                        }
                        Some(SessionCommand::Disconnect) | None => return true,
                        // Process other commands while waiting
                        Some(SessionCommand::Resize { width, height }) => {
                            let (width, height) = normalize_remote_resolution(width, height);
                            self.pending_resize = Some((width, height));
                        }
                        Some(SessionCommand::ClipboardContext { .. }) => {}
                Some(SessionCommand::ClipboardPaste { reply, .. }) => { let _ = reply.send(Err("Session is not connected".into())); }
                Some(SessionCommand::ClipboardSync { reply, .. }) => { let _ = reply.send(Err("Clipboard requires a connected real session.".into())); }
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
        if let Some((width, height)) = self.pending_resize.take() {
            self.width = width;
            self.height = height;
            self.frame_buffer = FrameBuffer::generate_mock_frame(width, height);
            self.publish_info();
        }
        self.delivery.reset();
        if self.send_mock_frame().is_err() {
            return DisconnectReason::ConnectionLost;
        }

        loop {
            let command = tokio::select! {
                command = self.cmd_rx.recv() => command,
                _ = crate::rdp::frame_delivery::wait_for_stall(self.delivery.deadline()) => { self.check_display_stall(); continue; }
            };
            match command {
                Some(SessionCommand::FrameConsumed { receipt, refresh }) => {
                    let was_stalled = self.delivery.stalled;
                    if self.delivery.acknowledge(&receipt, refresh) {
                        if was_stalled {
                            self.publish_info();
                        }
                        if self.flush_mock_frame().is_err() {
                            return DisconnectReason::ConnectionLost;
                        }
                    }
                }
                Some(SessionCommand::Disconnect) | None => {
                    log::info!("Session {} received disconnect command", self.id);
                    self.state = SessionState::Disconnected;
                    self.publish_info();
                    return DisconnectReason::UserDisconnect;
                }
                // Task 5.3 loss control (test-support builds only, Req 12.2):
                // the E2E harness forces a connection loss so the shared
                // reconnect loop runs against the synthetic session.
                #[cfg(feature = "test-support")]
                Some(SessionCommand::DropConnection) => {
                    log::info!(
                        "Session {} synthetic connection dropped by test control",
                        self.id
                    );
                    return DisconnectReason::ConnectionLost;
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
                Some(SessionCommand::ClipboardContext { .. }) => {}
                Some(SessionCommand::ClipboardPaste { reply, .. }) => {
                    let _ = reply.send(Err("Session is not connected".into()));
                }
                Some(SessionCommand::ClipboardSync { reply, .. }) => {
                    let _ = reply.send(Err("Clipboard requires a connected real session.".into()));
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
    /// Task 9.3 (Req 9.6/9.7): a terminal codec/decode error — the session
    /// configuration is incompatible (unsupported codec, or a stream that
    /// repeatedly fails to decode). Distinct from `ConnectionLost` on
    /// purpose: `run()` only auto-reconnects transport loss (Req 2.3);
    /// retrying an incompatible configuration would repeat the failure, so
    /// the actor ends and only a NEW Connect gesture restarts (task 7.1
    /// admission).
    CodecTerminal,
}

/// The framed transport a real session runs over.
///
/// Production sessions ride the TLS stream from [`attempt_rdp_connection`];
/// the `Local` arm is the task 7.4 characterization seam (an in-memory tokio
/// duplex wrapped in the same upstream `Framed` API) so deterministic tests
/// can drive the real session loop — including the Deactivation-Reactivation
/// Sequence — without network I/O.
enum SessionTransport {
    Tls(Box<crate::rdp::client::TlsFramed>),
    Local(Box<ironrdp_tokio::TokioFramed<tokio::io::DuplexStream>>),
}

/// Outcome of driving the upstream Deactivation-Reactivation Sequence.
enum ReactivationOutcome {
    /// The server completed the sequence; the existing stage is re-armed and
    /// the session loop continues in place (same actor, same session id).
    Reactivated,
    /// The sequence cannot complete on this transport (transport drop,
    /// protocol failure, server redirection, user disconnect). The actor
    /// must unwind with the carried reason.
    Stop(DisconnectReason),
}

/// Keep one request while the server opens the channel and sends capabilities.
/// Encoding consumes it only once; server activation/graphics own actual dimensions.
pub(crate) fn encode_pending_resize(
    pending: &mut Option<(u32, u32)>,
    stage: &mut ironrdp_session::ActiveStage,
) -> Option<ironrdp_session::SessionResult<Vec<u8>>> {
    let (width, height) = (*pending)?;
    let result = stage.encode_resize(width, height, None, None)?;
    *pending = None;
    Some(result)
}

fn normalize_remote_resolution(width: u32, height: u32) -> (u32, u32) {
    ironrdp_displaycontrol::pdu::MonitorLayoutEntry::adjust_display_size(width, height)
}

/// Encode a frontend keyboard event as a FastPath keyboard event.
///
/// The frontend `key_code` packs the set-1 scancode in the low byte and an
/// "extended key" flag in bit 8 (`0x100`, set for e.g. right-side modifiers,
/// arrows, NumpadEnter). Modifiers are ordinary scancodes — the client does
/// not track modifier state, it forwards each modifier key-down/-up itself.
/// Returns `None` for events without a scancode (unmapped keys), which the
/// session loop drops without touching the transport.
fn clipboard_paste_sequence(held_modifiers: &std::collections::BTreeSet<u32>) -> Vec<(u32, bool)> {
    // Preserve the actor's actual forwarded state at execution time, not the
    // physical state when Ctrl+V was initially intercepted before its ACK wait.
    held_modifiers
        .iter()
        .map(|code| (*code, false))
        .chain([(0x1d, true), (0x2f, true), (0x2f, false), (0x1d, false)])
        .chain(held_modifiers.iter().map(|code| (*code, true)))
        .collect()
}

fn encode_keyboard_fastpath_event(key_code: u32, is_down: bool) -> Option<FastPathInputEvent> {
    use ironrdp_pdu::input::fast_path::KeyboardFlags;

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
    use ironrdp_pdu::input::mouse::{MousePdu, PointerFlags};

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
/// Task 7.1 (Req 11.8/11.9): whether a session status still owns a live
/// actor whose entry Connect must reuse (connecting/connected/reconnecting),
/// as opposed to a terminal entry (error/disconnected) that Connect
/// restarts in place with the same session identity.
fn session_is_live(status: &SessionState) -> bool {
    matches!(
        status,
        SessionState::Connecting | SessionState::Connected | SessionState::Reconnecting
    )
}

/// Single-lock manager state: the task 7.1 admission map plus the task
/// 7.3 deletion tombstones share one mutex, so Connect admission and
/// destructive-operation coordination can never interleave.
#[derive(Default)]
struct SessionMap {
    handles: HashMap<String, SessionHandle>,
    /// Connection IDs whose coordinated deletion has begun (task 7.3):
    /// admission under the same lock rejects these, so no actor can
    /// appear for a connection between its stop phase and the store
    /// commit — or after a committed deletion. Rolled back when a
    /// deletion aborts (stop timeout / persist failure), kept forever
    /// once the deletion persisted.
    deleted_connections: HashSet<String>,
}

/// Bound for awaiting actor stop during a coordinated deletion (task
/// 7.3, design "Destructive actions and identity"): five seconds shared
/// across every affected session of one operation. Enforced with
/// `timeout_at` on the actor `JoinHandle`s — no polling, no wall-clock
/// sleeps (the paused test clock drives it deterministically).
const DELETION_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Task 7.3 remediation (review round 1): the timeout half of the
/// post-commit extras sweep (see
/// [`SessionManager::stop_sessions_for_committed_connections`]). The
/// store transaction already committed, so the tombstones stay — a
/// stale-config Connect may never resurrect a committed deletion —
/// while `stopped` surfaces the sessions that DID stop before the
/// bound (Req 10.18 exactness) and `offenders` keep their live entries.
pub(crate) struct CommittedStopTimeout {
    pub(crate) offenders: Vec<String>,
    pub(crate) stopped: Vec<String>,
}

pub struct SessionManager {
    state: Arc<Mutex<SessionMap>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(SessionMap::default())),
        }
    }

    /// Map entry count. Task 7.1 moved the session cap into the atomic
    /// admission path (`create_session_with_transport`), so this is
    /// introspection used by the lifecycle tests.
    #[cfg(test)]
    pub async fn session_count(&self) -> usize {
        self.state.lock().await.handles.len()
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
    /// Admission is idempotent per connection ID (task 7.1).
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
        // Task 7.1 (Req 11.8/11.9): admission is decided and applied under a
        // single lock acquisition, so concurrent Connect commands for the
        // same connection serialize here: exactly one actor per connection
        // and a stable returned identity.
        let mut state = self.state.lock().await;

        // Task 7.3 (design "Destructive actions and identity"): a
        // connection whose coordinated deletion has begun can never
        // admit a new actor — a Connect that raced the deletion (or a
        // double-click that read the config before the store commit)
        // fails closed here, under the same lock as the deletion's
        // stop phase, so no actor can appear between stop and commit
        // or resurrect a committed deletion.
        if state.deleted_connections.contains(&config.id) {
            return Err(format!(
                "Connection {} was deleted; no session can be created for it",
                config.id
            ));
        }

        let existing = state
            .handles
            .values()
            .find(|handle| handle.info_rx.borrow().connection_id == config.id)
            .map(|handle| {
                let info = handle.info_rx.borrow();
                (info.id.clone(), info.status.clone())
            });

        let session_id = match existing {
            // Req 11.8: a live connecting/connected/reconnecting session
            // already exists — return its identity instead of starting
            // another actor (the caller focuses its tab).
            Some((id, status)) if session_is_live(&status) => {
                log::info!(
                    "Connection {} already has live session {}; reusing it",
                    config.id,
                    id
                );
                return Ok(id);
            }
            // Req 11.9: a terminal (error/disconnected) entry keeps its
            // slot; restart in place with the same session/tab identity.
            Some((id, status)) => {
                log::info!(
                    "Connection {} has terminal session {:?}; restarting as session {}",
                    config.id,
                    status,
                    id
                );
                id
            }
            // Fresh admission enforces the concurrent-session cap (Req 2.2)
            // under the same lock as the insert, so racing Connects cannot
            // both pass the check.
            None => {
                if state.handles.len() >= MAX_SESSIONS {
                    return Err(format!(
                        "Maximum session limit ({}) reached. Disconnect an existing session first.",
                        MAX_SESSIONS
                    ));
                }
                Uuid::new_v4().to_string()
            }
        };
        let (width, height) = normalize_remote_resolution(
            config.display_width.unwrap_or(1920),
            config.display_height.unwrap_or(1080),
        );

        // Create channels
        let (cmd_tx, cmd_rx) = mpsc::channel::<SessionCommand>(64);

        // Build initial info for the watch channel
        let initial_info = SessionInfo {
            display_stalled: false,
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

        // Store the handle. A restart replaces the terminal entry under the
        // same session ID while still holding the admission lock.
        let handle = SessionHandle {
            cmd_tx,
            info_rx,
            clipboard_rx,
            task_handle,
        };

        state.handles.insert(session_id.clone(), handle);

        log::info!("Created session {}", session_id);
        Ok(session_id)
    }

    /// Send a disconnect command and remove the session handle.
    pub async fn remove_session(&self, session_id: &str) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if let Some(handle) = state.handles.remove(session_id) {
            // Send disconnect command (ignore error if actor already stopped)
            let _ = handle.cmd_tx.send(SessionCommand::Disconnect).await;
            log::info!("Session {} removed", session_id);
            Ok(())
        } else {
            Err(format!("Session '{}' not found", session_id))
        }
    }

    /// Task 7.3 (Req 10.17, design "Destructive actions and identity"):
    /// begin a coordinated deletion for the given connections. Under the
    /// SAME lock as task 7.1 admission this:
    ///
    /// 1. tombstones every connection ID — Connects racing the deletion
    ///    serialize on this lock and fail closed until the outcome is
    ///    finalized (or rolled back);
    /// 2. sends `Disconnect` to every session actor of those connections
    ///    (the command that also cancels reconnect/backoff retries);
    /// 3. awaits each actor's stop with ONE shared five-second bound —
    ///    [`DELETION_STOP_TIMEOUT`] — measured on the actor
    ///    `JoinHandle`s, so "stopped" means the actor task finished.
    ///
    /// `Ok(stopped)` lists the session IDs that were live and stopped;
    /// their entries are removed and the tombstones stay for the
    /// caller's store commit. `Err(offenders)` means at least one actor
    /// missed the bound: the deletion must abort — the offenders keep
    /// their entries (still live, still Connect-reusable), the
    /// tombstones are rolled back here, but sessions that DID stop
    /// within the bound remain stopped and removed ("already-stopped
    /// sessions remain stopped").
    pub(crate) async fn stop_sessions_for_connections(
        &self,
        connection_ids: &[String],
    ) -> Result<Vec<String>, Vec<String>> {
        let mut state = self.state.lock().await;
        let (stopped, offenders) = Self::stop_sessions_under_lock(&mut state, connection_ids).await;
        if offenders.is_empty() {
            Ok(stopped)
        } else {
            // Deletion aborts here: the saved connections survive, so
            // they must be connectable again immediately.
            for id in connection_ids {
                state.deleted_connections.remove(id);
            }
            Err(offenders)
        }
    }

    /// Task 7.3 remediation (review round 1): the post-commit sweep for
    /// connections the committed store transaction deleted WITHOUT
    /// previewing (they joined the deleted library between the
    /// coordination layer's preview and the commit). Same bounded stop
    /// as [`SessionManager::stop_sessions_for_connections`], but the
    /// caller cannot abort — the deletion is already persisted — so a
    /// stop timeout must NOT roll the tombstones back: every id was
    /// committed-deleted and stays tombstoned forever, so a stale-config
    /// Connect can never resurrect it. The sessions that DID stop
    /// before the bound are surfaced alongside the offenders
    /// ([`CommittedStopTimeout`]) so the caller still reports them
    /// (Req 10.18).
    pub(crate) async fn stop_sessions_for_committed_connections(
        &self,
        connection_ids: &[String],
    ) -> Result<Vec<String>, CommittedStopTimeout> {
        let mut state = self.state.lock().await;
        let (stopped, offenders) = Self::stop_sessions_under_lock(&mut state, connection_ids).await;
        if offenders.is_empty() {
            Ok(stopped)
        } else {
            // No rollback: the committed transaction already deleted
            // every id (no-resurrection invariant); the offenders keep
            // their live, tracked entries.
            Err(CommittedStopTimeout { offenders, stopped })
        }
    }

    /// The shared bounded-stop core (task 7.3), with the admission lock
    /// HELD by the caller: tombstone `connection_ids`, `Disconnect`
    /// every affected actor, then await the stops against ONE shared
    /// five-second bound. Returns the sessions that stopped (entries
    /// removed; live ones counted for the 10.18 report) and the
    /// offenders that missed the bound (entries kept, still live).
    /// Tombstone settlement stays with the caller — inside the same
    /// lock hold — because the abortable path rolls back while the
    /// committed path keeps them.
    async fn stop_sessions_under_lock(
        state: &mut SessionMap,
        connection_ids: &[String],
    ) -> (Vec<String>, Vec<String>) {
        for id in connection_ids {
            state.deleted_connections.insert(id.clone());
        }

        // Deterministic order: sorted session IDs, so reports and stop
        // sequences are reproducible regardless of HashMap layout.
        let mut affected: Vec<(String, bool)> = state
            .handles
            .values()
            .filter_map(|handle| {
                let info = handle.info_rx.borrow();
                connection_ids
                    .contains(&info.connection_id)
                    .then(|| (info.id.clone(), session_is_live(&info.status)))
            })
            .collect();
        affected.sort();
        if affected.is_empty() {
            return (Vec::new(), Vec::new());
        }

        // Cancel every affected actor FIRST — reconnect/backoff loops
        // and idle session loops all take `Disconnect` from their
        // command channel — then await the stops, so the shared
        // deadline measures stopping, not command delivery.
        for (session_id, _) in &affected {
            if let Some(handle) = state.handles.get(session_id) {
                let _ = handle.cmd_tx.send(SessionCommand::Disconnect).await;
            }
        }

        let deadline = tokio::time::Instant::now() + DELETION_STOP_TIMEOUT;
        let mut stopped = Vec::new();
        let mut offenders = Vec::new();
        for (session_id, was_live) in &affected {
            let Some(handle) = state.handles.get_mut(session_id) else {
                continue;
            };
            let stop = tokio::time::timeout_at(deadline, &mut handle.task_handle).await;
            match stop {
                Ok(joined) => {
                    if let Err(join_error) = joined {
                        log::warn!("Session {session_id} actor ended with {join_error}");
                    }
                    state.handles.remove(session_id);
                    if *was_live {
                        // Only sessions that were live count as "stopped"
                        // for the 10.18 confirmation counts; terminal
                        // entries were already stopped and just get
                        // retired with their connection.
                        stopped.push(session_id.clone());
                    }
                }
                Err(_elapsed) => {
                    // The entry stays: the actor is still running and
                    // must remain addressable (Connect reuses it).
                    offenders.push(session_id.clone());
                }
            }
        }

        (stopped, offenders)
    }

    /// Task 7.3: abort a coordinated deletion (stop timeout or persist
    /// failure) — clear the tombstones so the surviving connections can
    /// admit sessions again.
    pub(crate) async fn rollback_connection_deletions(&self, connection_ids: &[String]) {
        let mut state = self.state.lock().await;
        for id in connection_ids {
            state.deleted_connections.remove(id);
        }
    }

    /// Task 7.3: settle the tombstones after the store transaction
    /// committed. Only connections the transaction actually deleted stay
    /// tombstoned (forever — a deleted connection can never connect
    /// again); `blocked` ids that survived (e.g. a connection moved out
    /// of a deleted library mid-deletion) are released so Connect works
    /// for them again.
    pub(crate) async fn finalize_connection_deletions(
        &self,
        blocked: &[String],
        deleted: &[String],
    ) {
        let mut state = self.state.lock().await;
        for id in blocked {
            if !deleted.contains(id) {
                state.deleted_connections.remove(id);
            }
        }
    }

    /// Get a clone of the command sender for a session.
    pub async fn get_cmd_sender(&self, session_id: &str) -> Option<mpsc::Sender<SessionCommand>> {
        let state = self.state.lock().await;
        state.handles.get(session_id).map(|h| h.cmd_tx.clone())
    }

    /// Get the latest session info (lock-free read from watch channel).
    pub async fn get_session_info(&self, session_id: &str) -> Option<SessionInfo> {
        let state = self.state.lock().await;
        state
            .handles
            .get(session_id)
            .map(|h| h.info_rx.borrow().clone())
    }

    /// List all sessions.
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        let state = self.state.lock().await;
        state
            .handles
            .values()
            .map(|h| h.info_rx.borrow().clone())
            .collect()
    }

    pub async fn get_remote_clipboard(&self, session_id: &str) -> Option<String> {
        let state = self.state.lock().await;
        state
            .handles
            .get(session_id)
            .and_then(|handle| handle.clipboard_rx.borrow().clone())
    }

    /// Get a snapshot of the current frame for a session (generates mock frame on demand).
    /// This is a fallback for the get_frame command.
    pub async fn get_frame(&self, session_id: &str) -> Option<String> {
        let state = self.state.lock().await;
        if let Some(handle) = state.handles.get(session_id) {
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
    #[test]
    fn delayed_clipboard_paste_preserves_intervening_shortcut_modifiers() {
        use std::collections::BTreeSet;
        // Ctrl+V releases initial Ctrl while waiting. An intervening Ctrl+C
        // re-presses it before ACK; the delayed paste must preserve that state.
        for held in [
            BTreeSet::from([0x1d]),
            BTreeSet::from([0x2a]),
            BTreeSet::from([0x11d, 0x36]),
            BTreeSet::new(),
        ] {
            let mut remote = held.clone();
            let sequence = super::clipboard_paste_sequence(&held);
            for (code, down) in sequence {
                if down {
                    remote.insert(code);
                } else {
                    remote.remove(&code);
                }
                if code == 0x2f && down {
                    assert_eq!(
                        remote,
                        BTreeSet::from([0x1d, 0x2f]),
                        "paste must not inherit Shift/Alt/Windows modifiers"
                    );
                }
            }
            assert_eq!(
                remote, held,
                "next physical shortcut must see the same modifiers, even before IPC reply"
            );
        }
    }
    use super::*;
    use crate::rdp::client::ConnectionOutcome;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tauri::ipc::{Channel, InvokeResponseBody};

    /// Scripted frame-channel behavior.
    enum FrameScript {
        Ok,
        Fail,
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
            font_smoothing: true,
            host_profile: Default::default(),
            mock: false,
            library_id: String::new(),
            folder_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_connected_at: None,
        }
    }

    // Existing fixtures model an immediately consuming UI. Withheld-consumption
    // tests use a raw Channel instead and explicitly send receipts themselves.
    fn consuming_channel(
        manager: &SessionManager,
        sink: impl Fn(InvokeResponseBody) -> tauri::Result<()> + Send + Sync + 'static,
    ) -> Channel<InvokeResponseBody> {
        let state = Arc::downgrade(&manager.state);
        let pending = std::sync::Mutex::new(None::<(String, FrameReceipt)>);
        Channel::new(move |response| {
            if let InvokeResponseBody::Json(json) = &response {
                let value: serde_json::Value = serde_json::from_str(json).unwrap();
                *pending.lock().unwrap() = Some((
                    value["sessionId"].as_str().unwrap().to_owned(),
                    serde_json::from_value(value).unwrap(),
                ));
                return Ok(());
            }
            sink(response)?;
            if let Some((id, receipt)) = pending.lock().unwrap().take() {
                let state = state.clone();
                tokio::spawn(async move {
                    let Some(state) = state.upgrade() else {
                        return;
                    };
                    let sender = state
                        .lock()
                        .await
                        .handles
                        .get(&id)
                        .map(|h| h.cmd_tx.clone());
                    if let Some(sender) = sender {
                        let _ = sender
                            .send(SessionCommand::FrameConsumed {
                                receipt,
                                refresh: false,
                            })
                            .await;
                    }
                });
            }
            Ok(())
        })
    }

    fn scripted_frame_channel(
        manager: &SessionManager,
        script: FrameScript,
        sizes: Arc<std::sync::Mutex<Vec<usize>>>,
    ) -> Channel<InvokeResponseBody> {
        let sent = Arc::new(AtomicUsize::new(0));
        consuming_channel(manager, move |response| {
            let n = sent.fetch_add(1, Ordering::SeqCst) + 1;
            let fail = match &script {
                FrameScript::Ok => false,
                FrameScript::Fail => true,
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
            Box::pin(async { ConnectionOutcome::Synthetic })
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
                scripted_frame_channel(manager, script, Arc::clone(&frame_sizes)),
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
                scripted_frame_channel(&manager, FrameScript::Ok, sizes),
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

    // ── Task 7.1 (Req 2.2/11.4/11.8/11.9): idempotent connect admission ──

    /// Classification behind the admission decision: only
    /// connecting/connected/reconnecting sessions are live; error and
    /// disconnected entries are terminal and restart in place.
    #[test]
    fn test_session_is_live_classification_covers_all_states() {
        assert!(session_is_live(&SessionState::Connecting));
        assert!(session_is_live(&SessionState::Connected));
        assert!(session_is_live(&SessionState::Reconnecting));
        assert!(!session_is_live(&SessionState::Disconnected));
        assert!(!session_is_live(&SessionState::Error("boom".to_string())));
    }

    /// Concurrent Connect requests for one connection must serialize into a
    /// single actor and share one session identity; the serialization point
    /// is the SessionManager admission lock (Req 11.8).
    #[tokio::test(start_paused = true)]
    async fn test_concurrent_connects_for_one_connection_admit_one_actor_and_shared_session_id() {
        let manager = Arc::new(SessionManager::new());
        let (events, mut events_rx) = RecordedEvents::new();
        let connect_calls = Arc::new(AtomicUsize::new(0));

        let mut admissions = Vec::new();
        for _ in 0..8 {
            let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
            let sink = SessionEventSink::recorded(Arc::clone(&events));
            let connect = scripted_connect(Arc::clone(&connect_calls));
            let manager = Arc::clone(&manager);
            admissions.push(tokio::spawn(async move {
                manager
                    .create_session_with_transport(
                        test_config("dup"),
                        None,
                        scripted_frame_channel(&manager, FrameScript::Ok, sizes),
                        sink,
                        true,
                        None,
                        connect,
                    )
                    .await
            }));
        }

        let mut ids = Vec::new();
        for admission in admissions {
            ids.push(
                admission
                    .await
                    .expect("admission task alive")
                    .expect("concurrent duplicate connects are admitted idempotently"),
            );
        }

        for id in &ids[1..] {
            assert_eq!(
                id, &ids[0],
                "every concurrent Connect shares one session id"
            );
        }
        assert_eq!(manager.session_count().await, 1);
        let listed = manager.list_sessions().await;
        assert_eq!(listed.len(), 1, "the tree reflects a single session");
        assert_eq!(listed[0].id, ids[0]);
        assert_eq!(listed[0].connection_id, "conn-dup");

        // The one admitted actor dialed the connector exactly once and
        // settled into the live mock loop.
        let session_id = ids[0].clone();
        wait_for(&session_id, &events, &mut events_rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
        })
        .await;
        assert_eq!(
            connect_calls.load(Ordering::SeqCst),
            1,
            "exactly one actor was spawned for the connection"
        );
    }

    /// Req 11.8: Connect while a session is connecting, connected or
    /// reconnecting returns the existing session ID and starts no second
    /// actor.
    #[tokio::test(start_paused = true)]
    async fn test_connect_while_session_live_returns_existing_session_id_without_new_actor() {
        let manager = SessionManager::new();
        let first = create_scripted(&manager, "dup-live", true, FrameScript::Ok).await;
        let mut rx = first.events_rx.clone();
        wait_for(&first.id, &first.events, &mut rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
        })
        .await;

        let second = create_scripted(&manager, "dup-live", true, FrameScript::Ok).await;
        assert_eq!(
            second.id, first.id,
            "Connect while live must return the existing session identity"
        );
        assert_eq!(
            second.connect_calls.load(Ordering::SeqCst),
            0,
            "no second actor may dial for a live session"
        );
        assert_eq!(manager.session_count().await, 1);
    }

    /// Req 11.9: Connect after a terminal (error/disconnected) session
    /// restarts in place — same session ID, a fresh actor, still exactly one
    /// entry for the connection.
    #[tokio::test(start_paused = true)]
    async fn test_connect_after_terminal_session_restarts_in_place_with_stable_session_id() {
        let manager = SessionManager::new();
        let session = create_scripted(&manager, "restart", true, FrameScript::Ok).await;
        let mut rx = session.events_rx.clone();
        wait_for(&session.id, &session.events, &mut rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
        })
        .await;

        // Stop the actor WITHOUT removing the entry: a terminal tab stays
        // open with a stable identity.
        session
            .cmd_tx
            .send(SessionCommand::Disconnect)
            .await
            .expect("actor alive");
        wait_for(&session.id, &session.events, &mut rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
        })
        .await;
        assert_eq!(
            manager.session_count().await,
            1,
            "a terminal session keeps its slot until the tab is explicitly closed"
        );

        let dials_before_restart = session.connect_calls.load(Ordering::SeqCst);
        let (events, mut events_rx) = RecordedEvents::new();
        let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let restarted_id = manager
            .create_session_with_transport(
                test_config("restart"),
                None,
                scripted_frame_channel(&manager, FrameScript::Ok, sizes),
                SessionEventSink::recorded(Arc::clone(&events)),
                true,
                None,
                scripted_connect(Arc::clone(&session.connect_calls)),
            )
            .await
            .expect("restarting a terminal session is admitted");

        assert_eq!(
            restarted_id, session.id,
            "restart keeps the session/tab identity stable (Req 11.4/11.9)"
        );
        // The restarted actor dialed again and brought the entry back to a
        // live state under the same identity.
        wait_for(&session.id, &events, &mut events_rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
        })
        .await;
        assert_eq!(
            session.connect_calls.load(Ordering::SeqCst),
            dials_before_restart + 1,
            "the restart spawned a fresh actor"
        );
        assert_eq!(manager.session_count().await, 1);
        let listed = manager.list_sessions().await;
        assert_eq!(listed.len(), 1, "the tree reflects that single session");
        assert_eq!(listed[0].id, session.id);
        assert!(
            matches!(
                listed[0].status,
                SessionState::Connecting | SessionState::Connected
            ),
            "restarted entry is live again, got {:?}",
            listed[0].status
        );
    }

    /// Design "Destructive actions and identity": an explicit tab close
    /// removes the entry, so a later Connect gets a NEW session identity.
    #[tokio::test(start_paused = true)]
    async fn test_connect_after_explicit_close_admits_new_session_identity() {
        let manager = SessionManager::new();
        let closed = create_scripted(&manager, "reclose", true, FrameScript::Ok).await;
        let mut rx = closed.events_rx.clone();
        wait_for(&closed.id, &closed.events, &mut rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
        })
        .await;

        manager
            .remove_session(&closed.id)
            .await
            .expect("explicit close succeeds");
        assert_eq!(manager.session_count().await, 0);

        let reopened = create_scripted(&manager, "reclose", true, FrameScript::Ok).await;
        assert_ne!(
            reopened.id, closed.id,
            "after an explicit close, Connect gets a new session identity"
        );
        assert_eq!(manager.session_count().await, 1);
    }

    /// Req 2.2 + 11.8/11.9 at the cap boundary: a NEW connection is rejected
    /// at MAX_SESSIONS, while duplicate-live and terminal-restart Connects
    /// reuse their own slot instead of being rejected.
    #[tokio::test(start_paused = true)]
    async fn test_session_cap_rejects_new_connections_but_reuses_live_and_restarts_terminal_entries_at_limit()
     {
        let manager = SessionManager::new();
        let mut sessions = Vec::new();
        for i in 1..=10u32 {
            sessions
                .push(create_scripted(&manager, &format!("cap{i}"), true, FrameScript::Ok).await);
        }
        assert_eq!(manager.session_count().await, 10);

        // A NEW connection is rejected with the explanatory message (Req 2.2).
        let (events, _events_rx) = RecordedEvents::new();
        let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let err = manager
            .create_session_with_transport(
                test_config("cap11"),
                None,
                scripted_frame_channel(&manager, FrameScript::Ok, sizes),
                SessionEventSink::recorded(events),
                true,
                None,
                scripted_connect(Arc::new(AtomicUsize::new(0))),
            )
            .await
            .expect_err("an 11th new connection must be rejected");
        assert_eq!(
            err,
            "Maximum session limit (10) reached. Disconnect an existing session first."
        );

        // Duplicate Connect for a LIVE session is NOT rejected (Req 11.8),
        // even with all ten slots taken.
        let live = &sessions[1];
        let mut live_rx = live.events_rx.clone();
        wait_for(&live.id, &live.events, &mut live_rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
        })
        .await;
        let (events, _events_rx) = RecordedEvents::new();
        let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let dup_id = manager
            .create_session_with_transport(
                test_config("cap2"),
                None,
                scripted_frame_channel(&manager, FrameScript::Ok, sizes),
                SessionEventSink::recorded(events),
                true,
                None,
                scripted_connect(Arc::new(AtomicUsize::new(0))),
            )
            .await
            .expect("duplicate Connect for a live session reuses it, even at the cap");
        assert_eq!(dup_id, live.id);
        assert_eq!(manager.session_count().await, 10);

        // Restart of a TERMINAL entry at the cap reuses its own slot.
        let terminal = &sessions[0];
        let mut terminal_rx = terminal.events_rx.clone();
        wait_for(&terminal.id, &terminal.events, &mut terminal_rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
        })
        .await;
        terminal
            .cmd_tx
            .send(SessionCommand::Disconnect)
            .await
            .expect("actor alive");
        wait_for(&terminal.id, &terminal.events, &mut terminal_rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
        })
        .await;

        let dials_before_restart = terminal.connect_calls.load(Ordering::SeqCst);
        let (events, mut events_rx) = RecordedEvents::new();
        let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let restarted_id = manager
            .create_session_with_transport(
                test_config("cap1"),
                None,
                scripted_frame_channel(&manager, FrameScript::Ok, sizes),
                SessionEventSink::recorded(Arc::clone(&events)),
                true,
                None,
                scripted_connect(Arc::clone(&terminal.connect_calls)),
            )
            .await
            .expect("terminal restart reuses its own slot, even at the cap");
        assert_eq!(restarted_id, terminal.id);
        // The restarted actor dialed again under the same identity.
        wait_for(&terminal.id, &events, &mut events_rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
        })
        .await;
        assert_eq!(
            terminal.connect_calls.load(Ordering::SeqCst),
            dials_before_restart + 1,
            "the restart spawned a fresh actor"
        );
        assert_eq!(manager.session_count().await, 10);
    }

    #[tokio::test(start_paused = true)]
    async fn test_initial_failed_connect_has_no_connected_or_frames() {
        let manager = SessionManager::new();
        let (events, _) = RecordedEvents::new();
        let frames = Arc::new(std::sync::Mutex::new(Vec::new()));
        let connect: ConnectFn = Arc::new(move |_, _, _, _| {
            Box::pin(async { ConnectionOutcome::Failed("TLS authentication rejected".into()) })
        });
        let id = manager
            .create_session_with_transport(
                test_config("failed-initial"),
                None,
                scripted_frame_channel(&manager, FrameScript::Ok, frames.clone()),
                SessionEventSink::recorded(events.clone()),
                true,
                None,
                connect,
            )
            .await
            .unwrap();
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert!(
            !statuses_of(&info_events(&id, &events))
                .iter()
                .any(|s| s == "connected")
        );
        assert!(frames.lock().unwrap().is_empty());
        let info = manager.get_session_info(&id).await.unwrap();
        assert!(matches!(info.status, SessionState::Error(_)));
        assert_eq!(
            info.last_error.as_deref(),
            Some("TLS authentication rejected")
        );
        assert!(info.connected_at.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn test_failed_reconnect_is_bounded_without_false_connected_or_frames() {
        let manager = SessionManager::new();
        let (events, _) = RecordedEvents::new();
        let frames = Arc::new(std::sync::Mutex::new(Vec::new()));
        let dials = Arc::new(std::sync::Mutex::new(Vec::new()));
        let times = dials.clone();
        let connect: ConnectFn = Arc::new(move |_, _, _, _| {
            let mut times = times.lock().unwrap();
            times.push(tokio::time::Instant::now());
            let first = times.len() == 1;
            Box::pin(async move {
                if first {
                    let (client, server) = tokio::io::duplex(1024);
                    drop(server);
                    ConnectionOutcome::RealSessionLocal {
                        connection_result: Box::new(
                            crate::rdp::baseline::synthetic_connection_result(64, 64),
                        ),
                        framed: Box::new(ironrdp_tokio::TokioFramed::new(client)),
                        codec_policy: Arc::new(CodecErrorPolicy::new()),
                    }
                } else {
                    ConnectionOutcome::Failed("NLA rejected on retry".into())
                }
            })
        });
        let id = manager
            .create_session_with_transport(
                test_config("failed-retry"),
                None,
                scripted_frame_channel(&manager, FrameScript::Ok, frames.clone()),
                SessionEventSink::recorded(events.clone()),
                true,
                None,
                connect,
            )
            .await
            .unwrap();
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        for delay in [2, 4, 8, 16, 30] {
            tokio::time::advance(std::time::Duration::from_secs(delay)).await;
            for _ in 0..20 {
                tokio::task::yield_now().await;
            }
        }
        assert_eq!(
            statuses_of(&info_events(&id, &events))
                .iter()
                .filter(|s| *s == "connected")
                .count(),
            1
        );
        assert!(frames.lock().unwrap().is_empty());
        assert_eq!(dials.lock().unwrap().len(), 6);
        assert_eq!(
            dials
                .lock()
                .unwrap()
                .windows(2)
                .map(|p| (p[1] - p[0]).as_secs())
                .collect::<Vec<_>>(),
            vec![2, 4, 8, 16, 30]
        );
        let info = manager.get_session_info(&id).await.unwrap();
        assert!(matches!(info.status, SessionState::Error(_)));
        assert!(info.last_error.unwrap().contains("NLA rejected on retry"));
        assert_eq!(info.reconnect_attempts, MAX_RECONNECT_ATTEMPTS);
    }

    #[tokio::test(start_paused = true)]
    async fn test_cancel_pending_reconnect_dial_stops_actor() {
        let manager = SessionManager::new();
        let (events, _) = RecordedEvents::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let connect: ConnectFn = Arc::new(move |_, _, _, _| {
            let first = count.fetch_add(1, Ordering::SeqCst) == 0;
            Box::pin(async move {
                if first {
                    ConnectionOutcome::Synthetic
                } else {
                    std::future::pending().await
                }
            })
        });
        let id = manager
            .create_session_with_transport(
                test_config("cancel-dial"),
                None,
                scripted_frame_channel(
                    &manager,
                    FrameScript::Fail,
                    Arc::new(std::sync::Mutex::new(Vec::new())),
                ),
                SessionEventSink::recorded(events.clone()),
                true,
                None,
                connect,
            )
            .await
            .unwrap();
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        manager
            .get_cmd_sender(&id)
            .await
            .unwrap()
            .send(SessionCommand::Disconnect)
            .await
            .unwrap();
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            manager.get_session_info(&id).await.unwrap().status,
            SessionState::Disconnected
        );
        tokio::time::advance(std::time::Duration::from_secs(120)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn test_synthetic_transport_loss_retries_capped_backoff_then_retains_error() {
        let manager = SessionManager::new();
        let session = create_scripted(&manager, "recon", true, FrameScript::Fail).await;
        let mut rx = session.events_rx.clone();

        wait_for(&session.id, &session.events, &mut rx, |events| {
            events
                .iter()
                .any(|(_, v)| status_string(v).starts_with("error:Reconnection failed"))
        })
        .await;

        let recorded = info_events(&session.id, &session.events);
        // Explicit synthetic sessions reconnect through the same capped loop.
        let mut expected: Vec<&str> = vec!["connecting", "connected"];
        for _ in 0..MAX_RECONNECT_ATTEMPTS {
            expected.push("reconnecting");
            expected.push("connected");
        }
        let terminal = recorded.last().unwrap().1["lastError"].as_str().unwrap();
        let terminal_status = format!("error:{terminal}");
        expected.push(&terminal_status);
        expected.push(&terminal_status);
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

        // Terminal error survives actor teardown in both status and lastError.
        let (_, last) = recorded.last().expect("final event recorded");
        assert_eq!(status_string(last), terminal_status);
        assert!(
            last["lastError"]
                .as_str()
                .unwrap()
                .starts_with("Reconnection failed after 5 attempts:")
        );
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

    fn mouse_pdu(event: FastPathInputEvent) -> ironrdp_pdu::input::MousePdu {
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
        use ironrdp_pdu::input::fast_path::KeyboardFlags;

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
        use ironrdp_pdu::input::mouse::PointerFlags;

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
        assert!(
            down_middle
                .flags
                .contains(PointerFlags::MIDDLE_BUTTON_OR_WHEEL)
        );

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
        use ironrdp_pdu::input::mouse::PointerFlags;

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

    /// Req 3.1/3.2: the explicit synthetic frame loop accepts
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

    /// Req 4.2 — the webview-canvas IPC path is independent of the GPU
    /// surface: frames keep flowing to the frontend channel even when a
    /// SharedFrame is attached, and the mock/canvas path never writes it
    /// (the GPU renderer consumes it separately; `GpuRenderer` itself needs
    /// a real window/adapter — audit, not unit-testable here).
    #[tokio::test(start_paused = true)]
    async fn test_mock_frames_flow_without_touching_the_gpu_shared_surface() {
        use crate::renderer::shared_frame::SharedFrame;

        let manager = SessionManager::new();
        let shared_frame = SharedFrame::new(64, 64);
        let (events, events_rx) = RecordedEvents::new();
        let frame_sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let frame_kinds = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let connect_calls = Arc::new(AtomicUsize::new(0));
        let kinds_sink = Arc::clone(&frame_kinds);
        let sizes_sink = Arc::clone(&frame_sizes);
        // Inline channel (instead of scripted_frame_channel) so this test can
        // also pin the emitted packet KIND byte at the actor seam: mock frames
        // must be kind-1 full frames — kind 3 (H.264) is retired (task 4.4)
        // and must never be re-emitted here.
        let frame_channel = tauri::ipc::Channel::new(move |response| {
            if let tauri::ipc::InvokeResponseBody::Raw(bytes) = &response {
                if !bytes.is_empty() {
                    kinds_sink.lock().unwrap().push(bytes[0]);
                }
                sizes_sink.lock().unwrap().push(bytes.len());
            }
            Ok(())
        });
        let id = manager
            .create_session_with_transport(
                test_config("gpu-independent"),
                None,
                frame_channel,
                SessionEventSink::recorded(Arc::clone(&events)),
                true,
                Some(Arc::clone(&shared_frame)),
                scripted_connect(Arc::clone(&connect_calls)),
            )
            .await
            .expect("session with an attached GPU surface should be created");

        let mut rx = events_rx.clone();
        wait_for(&id, &events, &mut rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                && !frame_sizes.lock().unwrap().is_empty()
        })
        .await;

        // IPC full frames reached the webview channel — and every emitted
        // packet is a kind-1 full frame (kind byte pinned at the actor seam).
        let kinds = frame_kinds.lock().unwrap().clone();
        assert!(!kinds.is_empty());
        assert!(
            kinds
                .iter()
                .all(|k| *k == crate::rdp::frame_transport::PACKET_KIND_FULL_FRAME),
            "mock frames must emit only kind-1 full frames, got {kinds:?}"
        );
        assert!(!frame_sizes.lock().unwrap().is_empty());
        // …while the GPU surface stayed untouched: no dirty flag, no
        // published snapshot, no version bump.
        assert_eq!(shared_frame.version(), 0);
        assert!(shared_frame.publish().is_none());

        let cmd_tx = manager
            .get_cmd_sender(&id)
            .await
            .expect("session handle exists");
        cmd_tx.send(SessionCommand::Disconnect).await.unwrap();
        wait_for(&id, &events, &mut rx, |evs| {
            evs.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
        })
        .await;
    }

    // ---- Task 5.3: build-gated synthetic (mock) sessions (Req 12.2/12.7/12.10).
    //
    // The `test-support` cargo feature is the single build gate: synthetic
    // sessions, their loss/cancel controls and the E2E seeding commands exist
    // only in feature builds. Production builds (no feature) reject every
    // mock-session request — persisted flag or command flag — and read no
    // environment variables anywhere on that path (Req 12.10).

    /// Production-path admission (compiled WITHOUT the test-support feature):
    /// every mock-session request is rejected, and no environment variable can
    /// bypass the gate because the allow-path is not compiled in (Req 12.10).
    #[cfg(not(feature = "test-support"))]
    mod production_mock_rejection {
        use super::*;

        /// Removes a test-set environment variable on scope exit, including
        /// the panic path, so shared-process test runs never inherit it.
        struct EnvVarGuard(&'static str);
        impl EnvVarGuard {
            fn set(key: &'static str, value: &str) -> Self {
                unsafe { std::env::set_var(key, value) };
                EnvVarGuard(key)
            }
        }
        impl Drop for EnvVarGuard {
            fn drop(&mut self) {
                unsafe { std::env::remove_var(self.0) };
            }
        }

        #[test]
        fn test_production_rejects_persisted_mock_flag() {
            let err = admit_mock_session(true, None)
                .expect_err("production must reject persisted mock flags");
            assert!(
                err.to_lowercase().contains("mock"),
                "rejection must name mock sessions: {err}"
            );
        }

        #[test]
        fn test_production_rejects_command_mock_flag() {
            admit_mock_session(false, Some(true))
                .expect_err("production must reject command mock flags");
        }

        #[test]
        fn test_production_rejects_mock_even_with_test_env_vars_set() {
            // Req 12.10: environment variables must not admit mock sessions.
            let _a = EnvVarGuard::set(
                "DRIFT_TEST_DATA_DIR",
                "D:\\BuildArtifacts\\drift-rdp\\env-bypass-probe",
            );
            let _b = EnvVarGuard::set("DRIFT_TEST_CREDENTIAL_NAMESPACE", "env-bypass-probe");
            let rejected = admit_mock_session(true, Some(true)).is_err();
            assert!(
                rejected,
                "env vars must never admit mock sessions in production builds"
            );
        }

        #[test]
        fn test_production_admits_untagged_and_explicitly_real_requests() {
            assert_eq!(admit_mock_session(false, None), Ok(false));
            assert_eq!(admit_mock_session(false, Some(false)), Ok(false));
        }
    }

    /// Task 5.4 (Req 12.10): production builds expose no frame spy. With the
    /// spy environment flag set, frames still flow through the unchanged
    /// emission path, no spy events are emitted, and mock admission stays
    /// rejected. These pin the production absence at behavior level; the
    /// binary-level counterpart is the task-5.4 rlib byte-scan evidence.
    #[cfg(not(feature = "test-support"))]
    mod production_frame_spy_absence {
        use super::*;

        /// Removes the variable on scope exit (including panics) so
        /// shared-process test runs never inherit it.
        struct EnvVarGuard(&'static str);
        impl EnvVarGuard {
            fn set(key: &'static str, value: &str) -> Self {
                unsafe { std::env::set_var(key, value) };
                EnvVarGuard(key)
            }
        }
        impl Drop for EnvVarGuard {
            fn drop(&mut self) {
                unsafe { std::env::remove_var(self.0) };
            }
        }

        #[tokio::test(start_paused = true)]
        async fn test_production_with_spy_env_flag_frames_flow_without_spy_events() {
            let _g = EnvVarGuard::set("DRIFT_TEST_FRAME_SPY", "1");
            let manager = SessionManager::new();
            let session = create_scripted(&manager, "prod-spy", true, FrameScript::Ok).await;
            let mut rx = session.events_rx.clone();
            let sizes = Arc::clone(&session.frame_sizes);

            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                    && !sizes.lock().unwrap().is_empty()
            })
            .await;

            // Frames actually crossed the unchanged emission path…
            assert!(
                !sizes.lock().unwrap().is_empty(),
                "frame flow is undisturbed by the env flag"
            );
            // …but no spy event of any name was emitted alongside them.
            let spy_named: Vec<String> = session
                .events
                .snapshot()
                .into_iter()
                .filter(|(_, event, _)| event.contains("frame-spy"))
                .map(|(_, event, _)| event)
                .collect();
            assert!(
                spy_named.is_empty(),
                "production builds emit no frame-spy events even with the env flag set: {spy_named:?}"
            );

            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .unwrap();
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;
        }

        #[test]
        fn test_production_rejects_mock_with_spy_env_flag_set() {
            // Req 12.10: the spy env var must not admit mock sessions either.
            let _g = EnvVarGuard::set("DRIFT_TEST_FRAME_SPY", "1");
            assert!(
                admit_mock_session(true, Some(true)).is_err(),
                "spy env flag must never admit mock sessions in production"
            );
        }
    }

    /// Synthetic-session behavior in test-support builds (Req 12.2): the
    /// admitted session reuses the existing mock loop, Tokio time and actor
    /// state — never dialing the connector seam — and the loss control drives
    /// the shared reconnect/cancel path.
    #[cfg(feature = "test-support")]
    mod synthetic_sessions {
        use super::*;
        use crate::rdp::client::ConnectionOutcome;
        use crate::rdp::frame_spy::FrameSpySnapshot;
        use crate::rdp::frame_transport::PACKET_KIND_FULL_FRAME;

        struct SyntheticSession {
            id: String,
            cmd_tx: mpsc::Sender<SessionCommand>,
            events: Arc<RecordedEvents>,
            events_rx: watch::Receiver<usize>,
            frame_kinds: Arc<std::sync::Mutex<Vec<u8>>>,
            connect_calls: Arc<AtomicUsize>,
        }

        fn synthetic_config(tag: &str) -> ConnectionConfig {
            let mut config = test_config(tag);
            config.mock = true;
            config
        }

        /// Connector seam that records every call. A synthetic session must
        /// NEVER invoke it (Req 12.2: synthetic frames, no live server, no
        /// network at all).
        fn counting_connect(calls: Arc<AtomicUsize>) -> ConnectFn {
            Arc::new(move |_config, _password, _width, _height| {
                calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async {
                    ConnectionOutcome::Failed("connector dialed by a synthetic session".to_string())
                })
            })
        }

        async fn create_synthetic(
            manager: &SessionManager,
            tag: &str,
            auto_reconnect: bool,
        ) -> SyntheticSession {
            let (events, events_rx) = RecordedEvents::new();
            let frame_kinds = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
            let connect_calls = Arc::new(AtomicUsize::new(0));
            let kinds_sink = Arc::clone(&frame_kinds);
            let frame_channel = consuming_channel(manager, move |response| {
                if let tauri::ipc::InvokeResponseBody::Raw(bytes) = &response
                    && let Some(kind) = bytes.first()
                {
                    kinds_sink.lock().unwrap().push(*kind);
                }
                Ok(())
            });
            let id = manager
                .create_session_with_transport(
                    synthetic_config(tag),
                    None,
                    frame_channel,
                    SessionEventSink::recorded(Arc::clone(&events)),
                    auto_reconnect,
                    None,
                    counting_connect(Arc::clone(&connect_calls)),
                )
                .await
                .expect("synthetic session creation should succeed");
            let cmd_tx = manager
                .get_cmd_sender(&id)
                .await
                .expect("session handle exists right after creation");
            SyntheticSession {
                id,
                cmd_tx,
                events,
                events_rx,
                frame_kinds,
                connect_calls,
            }
        }

        /// Req 12.2: an admitted synthetic session renders kind-1 synthetic
        /// frames through the existing mock loop without a live RDP server —
        /// the connector seam is never dialed.
        #[tokio::test(start_paused = true)]
        async fn test_synthetic_session_reuses_mock_loop_frames_without_dialing() {
            // Boundary admission in test-support builds: the persisted flag
            // and the command-level flag both select synthetic execution.
            assert_eq!(admit_mock_session(true, None), Ok(true));
            assert_eq!(admit_mock_session(false, Some(true)), Ok(true));
            assert_eq!(admit_mock_session(false, None), Ok(false));
            assert_eq!(admit_mock_session(false, Some(false)), Ok(false));

            let manager = SessionManager::new();
            let session = create_synthetic(&manager, "synth-frames", true).await;
            let mut rx = session.events_rx.clone();
            let kinds = Arc::clone(&session.frame_kinds);

            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                    && !kinds.lock().unwrap().is_empty()
            })
            .await;

            // Synthetic frames are kind-1 full frames on the IPC transport.
            let recorded_kinds = kinds.lock().unwrap().clone();
            assert!(!recorded_kinds.is_empty());
            assert!(
                recorded_kinds.iter().all(|k| *k == PACKET_KIND_FULL_FRAME),
                "synthetic frames must be kind-1 full frames, got {recorded_kinds:?}"
            );

            // The connector seam was never invoked: no network (Req 12.2).
            assert_eq!(session.connect_calls.load(Ordering::SeqCst), 0);

            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .unwrap();
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;
            assert_eq!(
                session.connect_calls.load(Ordering::SeqCst),
                0,
                "disconnect also never dials"
            );
        }

        /// Task 5.3 loss control: the test-only DropConnection command makes
        /// the synthetic session take the existing ConnectionLost → reconnect
        /// path, which re-admits the mock loop — still without dialing.
        #[tokio::test(start_paused = true)]
        async fn test_synthetic_loss_control_reconnects_without_network() {
            let manager = SessionManager::new();
            let session = create_synthetic(&manager, "synth-recon", true).await;
            let mut rx = session.events_rx.clone();
            let kinds = Arc::clone(&session.frame_kinds);

            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                    && !kinds.lock().unwrap().is_empty()
            })
            .await;
            let frames_before = kinds.lock().unwrap().len();

            session
                .cmd_tx
                .send(SessionCommand::DropConnection)
                .await
                .expect("loss control accepted");

            // Reconnecting (backoff) … then connected again with a fresh
            // synthetic frame — the shared reconnect loop drives it.
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events
                    .iter()
                    .any(|(_, v)| status_string(v) == "reconnecting")
                    && events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                    && kinds.lock().unwrap().len() > frames_before
            })
            .await;

            // Reconnects stay synthetic: the connector seam is still untouched.
            assert_eq!(session.connect_calls.load(Ordering::SeqCst), 0);
        }

        /// Task 5.3 cancel control: a Disconnect during the reconnect backoff
        /// stops the synthetic session — reusing the actor's existing
        /// cancel-on-disconnect behavior (characterized in task 3.2).
        #[tokio::test(start_paused = true)]
        async fn test_synthetic_loss_then_cancel_during_backoff_stops_reconnecting() {
            let manager = SessionManager::new();
            let session = create_synthetic(&manager, "synth-cancel", true).await;
            let mut rx = session.events_rx.clone();
            let kinds = Arc::clone(&session.frame_kinds);

            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                    && !kinds.lock().unwrap().is_empty()
            })
            .await;

            session
                .cmd_tx
                .send(SessionCommand::DropConnection)
                .await
                .expect("loss control accepted");
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events
                    .iter()
                    .any(|(_, v)| status_string(v) == "reconnecting")
            })
            .await;

            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .expect("cancel accepted");
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;

            assert_eq!(
                session.connect_calls.load(Ordering::SeqCst),
                0,
                "cancel also never dials"
            );
            assert_eq!(
                statuses_of(&info_events(&session.id, &session.events)),
                vec![
                    "connecting".to_string(),
                    "connected".to_string(),
                    "reconnecting".to_string(),
                    "disconnected".to_string(), // cancel during backoff
                    "disconnected".to_string(), // actor tail publish
                ],
                "loss → cancel drives the shared reconnect state machine"
            );
            let _ = kinds;
        }

        // ---- Task 5.4 (Req 12.4/12.10): frame-kind spy on the real
        // emission path. The spy counts packets that actually crossed the
        // frame channel (the single `send_frame_packet` funnel both loops
        // send through), exposes cumulative counts as per-session events
        // and snapshots, and is compiled ONLY into test-support builds.

        /// Removes the spy env var on scope exit (including panics) and
        /// clears the process-wide spy registry, mirroring the harness env
        /// guards. `activate()` sets the spy flag; `deactivate()` removes it.
        struct SpyEnvGuard(bool);
        impl SpyEnvGuard {
            fn activate() -> Self {
                unsafe { std::env::set_var(frame_spy::FRAME_SPY_ENV, "1") };
                frame_spy::reset();
                SpyEnvGuard(true)
            }
            fn deactivate() -> Self {
                unsafe { std::env::remove_var(frame_spy::FRAME_SPY_ENV) };
                frame_spy::reset();
                SpyEnvGuard(false)
            }
        }
        impl Drop for SpyEnvGuard {
            fn drop(&mut self) {
                if self.0 {
                    unsafe { std::env::remove_var(frame_spy::FRAME_SPY_ENV) };
                }
                frame_spy::reset();
            }
        }

        /// Recorded spy events for a session: `(event name, payload)`.
        fn spy_events(id: &str, events: &Arc<RecordedEvents>) -> Vec<(String, serde_json::Value)> {
            let name = frame_spy::spy_event_name(id);
            events
                .snapshot()
                .into_iter()
                .filter(|(_, event, _)| *event == name)
                .map(|(_, event, payload)| (event, payload))
                .collect()
        }

        /// Req 12.4: the spy counts exactly the packets that actually
        /// crossed the frame channel — not what the loop intended — and
        /// publishes matching cumulative events for the E2E harness.
        #[tokio::test(start_paused = true)]
        async fn test_spy_counts_match_actually_emitted_packet_kinds() {
            let _g = SpyEnvGuard::activate();
            let manager = SessionManager::new();
            let session = create_synthetic(&manager, "spy-counts", true).await;
            let mut rx = session.events_rx.clone();
            let kinds = Arc::clone(&session.frame_kinds);

            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                    && !kinds.lock().unwrap().is_empty()
            })
            .await;

            // A resize drives the mock loop through the send funnel again.
            session
                .cmd_tx
                .send(SessionCommand::Resize {
                    width: 96,
                    height: 72,
                })
                .await
                .unwrap();
            wait_for(&session.id, &session.events, &mut rx, |_events| {
                kinds.lock().unwrap().len() >= 2
            })
            .await;

            // Independent observation: kinds that crossed the test channel.
            let emitted = kinds.lock().unwrap().clone();
            assert!(emitted.len() >= 2, "frames actually crossed the channel");

            let snapshot = frame_spy::snapshot(&session.id)
                .expect("active spy records the session that emitted frames");
            assert_eq!(
                snapshot.total_packets,
                emitted.len() as u64,
                "spy totals equal the actually-emitted packet count"
            );
            let mut expected_counts = std::collections::BTreeMap::new();
            for kind in &emitted {
                *expected_counts.entry(*kind).or_insert(0u64) += 1;
            }
            assert_eq!(
                snapshot.counts_by_kind, expected_counts,
                "spy per-kind counts equal the emitted kinds"
            );

            // The E2E-observable events carry the same cumulative counts.
            let recorded = spy_events(&session.id, &session.events);
            assert!(
                !recorded.is_empty(),
                "spy events are emitted for the E2E harness"
            );
            let last =
                serde_json::from_value::<FrameSpySnapshot>(recorded.last().unwrap().1.clone())
                    .unwrap();
            assert_eq!(last, snapshot, "latest event payload is the snapshot");

            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .unwrap();
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;
        }

        /// Req 12.4 (Req 9 policy): every kind the spy observed is a bitmap
        /// kind (kind 1/2); nothing else ever appears.
        #[tokio::test(start_paused = true)]
        async fn test_spy_reports_only_bitmap_kinds() {
            let _g = SpyEnvGuard::activate();
            let manager = SessionManager::new();
            let session = create_synthetic(&manager, "spy-bitmap", true).await;
            let mut rx = session.events_rx.clone();
            let kinds = Arc::clone(&session.frame_kinds);

            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                    && !kinds.lock().unwrap().is_empty()
            })
            .await;

            session
                .cmd_tx
                .send(SessionCommand::Resize {
                    width: 128,
                    height: 64,
                })
                .await
                .unwrap();
            wait_for(&session.id, &session.events, &mut rx, |_events| {
                kinds.lock().unwrap().len() >= 2
            })
            .await;

            let snapshot = frame_spy::snapshot(&session.id).unwrap();
            assert!(!snapshot.counts_by_kind.is_empty(), "spy observed packets");
            assert!(
                snapshot
                    .counts_by_kind
                    .keys()
                    .all(|k| frame_spy::BITMAP_PACKET_KINDS.contains(k)),
                "only bitmap kinds observed: {:?}",
                snapshot.counts_by_kind
            );
            assert!(snapshot.only_bitmap_kinds);

            for (_, payload) in spy_events(&session.id, &session.events) {
                let parsed = serde_json::from_value::<FrameSpySnapshot>(payload).unwrap();
                assert!(
                    parsed.only_bitmap_kinds,
                    "every spy event payload is bitmap-only"
                );
            }

            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .unwrap();
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;
        }

        /// Task 5.4: loss and cancel stay observable — counts keep
        /// accumulating across the reconnect loop and remain queryable
        /// after the actor stops.
        #[tokio::test(start_paused = true)]
        async fn test_spy_observable_across_loss_reconnect_and_cancel() {
            let _g = SpyEnvGuard::activate();
            let manager = SessionManager::new();
            let session = create_synthetic(&manager, "spy-loss", true).await;
            let mut rx = session.events_rx.clone();
            let kinds = Arc::clone(&session.frame_kinds);

            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                    && !kinds.lock().unwrap().is_empty()
            })
            .await;
            let before_loss = frame_spy::snapshot(&session.id).unwrap().total_packets;

            session
                .cmd_tx
                .send(SessionCommand::DropConnection)
                .await
                .expect("loss control accepted");
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events
                    .iter()
                    .any(|(_, v)| status_string(v) == "reconnecting")
                    && events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                    && kinds.lock().unwrap().len() as u64 > before_loss
            })
            .await;

            let after_reconnect = frame_spy::snapshot(&session.id).unwrap();
            assert!(
                after_reconnect.total_packets > before_loss,
                "reconnected frames keep counting"
            );
            assert!(after_reconnect.only_bitmap_kinds);

            // Cancel: session stops; counts remain queryable afterwards.
            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .expect("cancel accepted");
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;
            let final_snapshot = frame_spy::snapshot(&session.id)
                .expect("spy counts stay queryable after the actor stopped");
            assert_eq!(final_snapshot, after_reconnect);
        }

        /// Req 12.10 (env gate): without the spy env var the spy records
        /// nothing and emits no events — frames still flow through the
        /// unchanged emission path.
        #[tokio::test(start_paused = true)]
        async fn test_spy_inactive_without_env_flag_frames_still_flow() {
            let _g = SpyEnvGuard::deactivate();
            let manager = SessionManager::new();
            let session = create_synthetic(&manager, "spy-off", true).await;
            let mut rx = session.events_rx.clone();
            let kinds = Arc::clone(&session.frame_kinds);

            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("connected".to_string())
                    && !kinds.lock().unwrap().is_empty()
            })
            .await;

            assert!(
                !kinds.lock().unwrap().is_empty(),
                "frames flow regardless of the spy"
            );
            assert!(
                frame_spy::snapshot(&session.id).is_none(),
                "inactive spy records nothing"
            );
            assert!(
                spy_events(&session.id, &session.events).is_empty(),
                "inactive spy emits no events"
            );

            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .unwrap();
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;
        }
    }

    /// Task 7.4 — server DeactivateAll reactivates the session IN PLACE.
    ///
    /// These tests drive the REAL session loop over the in-memory framed
    /// seam (`ConnectionOutcome::RealSessionLocal`) with a scripted server
    /// side. They replace the task 4.3 recorded behavior (DeactivateAll →
    /// ConnectionLost → backoff reconnect; rationale preserved in
    /// baseline-coverage.md task 4.3) with the upstream
    /// Deactivation-Reactivation Sequence
    /// (`ConnectionActivationFactory` + `ActiveStage::reactivate`).
    mod real_session_reactivation {
        use super::*;
        use crate::rdp::client::ConnectionOutcome;
        use ironrdp_pdu::rdp::headers::ShareControlPdu;

        // Channel ids mirroring `baseline::synthetic_connection_result`.
        const IO_CHANNEL_ID: u16 = 1003;
        const USER_CHANNEL_ID: u16 = 1004;
        // The share id the scripted server picks for the reactivation.
        const REACTIVATION_SHARE_ID: u32 = 0x0011_2233;
        // Desktop the scripted server negotiates for the reactivation. It
        // differs from the fixture's 64x64 so applying the sequence's
        // Finalized state is observable in published session info.
        const REACTIVATION_WIDTH: u16 = 96;
        const REACTIVATION_HEIGHT: u16 = 80;

        /// Encode a ShareControlPdu as a server→client MCS SendDataIndication
        /// — the full wire frame, byte-identical in shape to what `read_pdu`
        /// delivers from a real transport. Upstream ships an encoder for the
        /// client→server SendDataRequest only
        /// (`mcs::encode_send_data_request`); the server direction is built
        /// from the same pieces the decoder consumes.
        fn server_share_control_frame(pdu: ShareControlPdu, share_id: u32) -> Vec<u8> {
            use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
            use ironrdp_pdu::rdp::capability_sets::SERVER_CHANNEL_ID;
            use ironrdp_pdu::rdp::headers::ShareControlHeader;
            use ironrdp_pdu::x224::X224;

            let user_data = ironrdp_pdu::encode_vec(&ShareControlHeader {
                share_control_pdu: pdu,
                pdu_source: SERVER_CHANNEL_ID,
                share_id,
            })
            .expect("scripted share control encodes");

            ironrdp_pdu::encode_vec(&X224(McsMessage::SendDataIndication(SendDataIndication {
                initiator_id: SERVER_CHANNEL_ID,
                channel_id: IO_CHANNEL_ID,
                user_data: std::borrow::Cow::Owned(user_data),
            })))
            .expect("scripted x224 frame encodes")
        }

        fn deactivate_all_frame() -> Vec<u8> {
            server_share_control_frame(
                ShareControlPdu::ServerDeactivateAll(
                    ironrdp_pdu::rdp::headers::ServerDeactivateAll,
                ),
                REACTIVATION_SHARE_ID,
            )
        }

        fn demand_active_frame() -> Vec<u8> {
            demand_active_frame_for(REACTIVATION_WIDTH, REACTIVATION_HEIGHT)
        }

        fn demand_active_frame_for(width: u16, height: u16) -> Vec<u8> {
            use ironrdp_pdu::rdp::capability_sets::{
                Bitmap, BitmapDrawingFlags, CapabilitySet, DemandActive, General, PROTOCOL_VER,
                ServerDemandActive,
            };

            server_share_control_frame(
                ShareControlPdu::ServerDemandActive(ServerDemandActive {
                    pdu: DemandActive {
                        source_descriptor: "scripted-server".to_owned(),
                        capability_sets: vec![
                            CapabilitySet::General(General {
                                protocol_version: PROTOCOL_VER,
                                ..Default::default()
                            }),
                            CapabilitySet::Bitmap(Bitmap {
                                pref_bits_per_pix: 32,
                                desktop_width: width,
                                desktop_height: height,
                                desktop_resize_flag: true,
                                drawing_flags: BitmapDrawingFlags::ALLOW_SKIP_ALPHA,
                            }),
                        ],
                    },
                }),
                REACTIVATION_SHARE_ID,
            )
        }

        fn finalization_frame(pdu: ironrdp_pdu::rdp::headers::ShareDataPdu) -> Vec<u8> {
            use ironrdp_pdu::rdp::client_info::CompressionType;
            use ironrdp_pdu::rdp::headers::{CompressionFlags, ShareDataHeader, StreamPriority};

            server_share_control_frame(
                ShareControlPdu::Data(ShareDataHeader {
                    share_data_pdu: pdu,
                    stream_priority: StreamPriority::Medium,
                    compression_flags: CompressionFlags::empty(),
                    compression_type: CompressionType::K8,
                }),
                REACTIVATION_SHARE_ID,
            )
        }

        /// All server frames of a complete Deactivation-Reactivation Sequence.
        fn reactivation_script_frames() -> Vec<Vec<u8>> {
            use ironrdp_pdu::rdp::capability_sets::SERVER_CHANNEL_ID;
            use ironrdp_pdu::rdp::finalization_messages::{
                ControlAction, ControlPdu, FontPdu, SynchronizePdu,
            };
            use ironrdp_pdu::rdp::headers::ShareDataPdu;

            let server_finalization = [
                ShareDataPdu::Synchronize(SynchronizePdu {
                    target_user_id: USER_CHANNEL_ID,
                }),
                ShareDataPdu::Control(ControlPdu {
                    action: ControlAction::Cooperate,
                    grant_id: 0,
                    control_id: 0,
                }),
                ShareDataPdu::Control(ControlPdu {
                    action: ControlAction::GrantedControl,
                    grant_id: USER_CHANNEL_ID,
                    control_id: u32::from(SERVER_CHANNEL_ID),
                }),
                ShareDataPdu::FontMap(FontPdu::default()),
            ];

            let mut frames = vec![deactivate_all_frame(), demand_active_frame()];
            frames.extend(server_finalization.into_iter().map(finalization_frame));
            frames
        }

        /// Create a real-session actor over the in-memory framed seam.
        ///
        /// The scripted server writes `server_frames` in order; `hold_open`
        /// keeps the transport alive afterwards (a live reactivated session)
        /// or drops it (transport EOF).
        async fn create_scripted_real_session(
            manager: &SessionManager,
            tag: &str,
            auto_reconnect: bool,
            server_frames: Vec<Vec<u8>>,
            hold_open: bool,
        ) -> TestSession {
            let (events, events_rx) = RecordedEvents::new();
            let frame_sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
            let connect_calls = Arc::new(AtomicUsize::new(0));
            let (client_end, mut server_end) = tokio::io::duplex(64 * 1024);

            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt as _;
                // Whole pre-encoded frames — the server side needs no framing.
                for frame in server_frames {
                    server_end
                        .write_all(&frame)
                        .await
                        .expect("scripted server write");
                }
                if hold_open {
                    // Keep the server half alive: a live (possibly
                    // reactivated) session, parked like an idle server.
                    std::future::pending::<()>().await;
                }
            });

            let scripted_once = std::sync::Mutex::new(Some(client_end));
            let calls = Arc::clone(&connect_calls);
            let connect: ConnectFn = Arc::new(move |_config, _password, _width, _height| {
                calls.fetch_add(1, Ordering::SeqCst);
                let scripted = scripted_once.lock().unwrap().take();
                Box::pin(async move {
                    match scripted {
                        Some(stream) => ConnectionOutcome::RealSessionLocal {
                            connection_result: Box::new(
                                crate::rdp::baseline::synthetic_connection_result(64, 64),
                            ),
                            framed: Box::new(ironrdp_tokio::TokioFramed::new(stream)),
                            codec_policy: Arc::new(CodecErrorPolicy::new()),
                        },
                        // Later dials fail like a dead endpoint so the OLD
                        // observable (DeactivateAll → backoff reconnect
                        // cycle) stays distinguishable from the new
                        // in-place reactivation.
                        None => ConnectionOutcome::Failed(
                            "scripted real session already consumed".to_string(),
                        ),
                    }
                })
            });

            let id = manager
                .create_session_with_transport(
                    test_config(tag),
                    None,
                    scripted_frame_channel(manager, FrameScript::Ok, Arc::clone(&frame_sizes)),
                    SessionEventSink::recorded(Arc::clone(&events)),
                    auto_reconnect,
                    None,
                    connect,
                )
                .await
                .expect("scripted real session creation should succeed");
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

        fn bitmap_wire_frames(
            rect: ironrdp_pdu::geometry::InclusiveRectangle,
            rows: &[u8],
        ) -> Vec<Vec<u8>> {
            use ironrdp_pdu::fast_path::{EncryptionFlags, FastPathHeader};
            crate::rdp::baseline::encode_bitmap_update_pdu_16bpp(&[rect], &[rows])
                .into_iter()
                .map(|frame| {
                    // Baseline fixtures target ActiveStage directly. The framed
                    // transport also needs the update-header bytes in its length.
                    let header_len = if frame[1] & 0x80 == 0 { 2 } else { 3 };
                    let payload = &frame[header_len..];
                    let mut wire = ironrdp_pdu::encode_vec(&FastPathHeader::new(
                        EncryptionFlags::empty(),
                        payload.len(),
                    ))
                    .unwrap();
                    wire.extend_from_slice(payload);
                    wire
                })
                .collect()
        }

        async fn settle_actor() {
            // All transport and command work here is in-memory on one runtime.
            // Yield without advancing paused time: specifically ACK before the
            // first graphics update of the replacement share, not on a timer.
            for _ in 0..100 {
                tokio::task::yield_now().await;
            }
        }

        async fn verify_delivery_reactivation(size: u16, ack_during_activation: bool) {
            use crate::rdp::baseline::full_frame_rect;
            use ironrdp_pdu::geometry::InclusiveRectangle;
            use tokio::io::AsyncWriteExt as _;
            let manager = SessionManager::new();
            let (client_end, mut server_end) = tokio::io::duplex(64 * 1024);
            let stream = std::sync::Mutex::new(Some(client_end));
            let connect: ConnectFn = Arc::new(move |_, _, _, _| {
                let stream = stream.lock().unwrap().take().unwrap();
                Box::pin(async move {
                    ConnectionOutcome::RealSessionLocal {
                        connection_result: Box::new(
                            crate::rdp::baseline::synthetic_connection_result(64, 64),
                        ),
                        framed: Box::new(ironrdp_tokio::TokioFramed::new(stream)),
                        codec_policy: Arc::new(CodecErrorPolicy::new()),
                    }
                })
            });
            let receipts = Arc::new(std::sync::Mutex::new(Vec::<FrameReceipt>::new()));
            let packets = Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
            let rs = Arc::clone(&receipts);
            let ps = Arc::clone(&packets);
            let channel = Channel::new(move |response| {
                match response {
                    InvokeResponseBody::Json(json) => rs
                        .lock()
                        .unwrap()
                        .push(serde_json::from_str(&json).unwrap()),
                    InvokeResponseBody::Raw(bytes) => ps.lock().unwrap().push(bytes),
                }
                Ok(())
            });
            let (events, _) = RecordedEvents::new();
            let shared = SharedFrame::new(64, 64);
            let id = manager
                .create_session_with_transport(
                    test_config("delivery-reactivation"),
                    None,
                    channel,
                    SessionEventSink::recorded(events),
                    false,
                    Some(Arc::clone(&shared)),
                    connect,
                )
                .await
                .unwrap();
            let sender = manager.get_cmd_sender(&id).await.unwrap();
            let red_rows = [0x00, 0xf8].repeat(64 * 64);
            for frame in bitmap_wire_frames(full_frame_rect(64, 64), &red_rows) {
                server_end.write_all(&frame).await.unwrap();
            }
            settle_actor().await;
            assert_eq!(packets.lock().unwrap().len(), 1);
            let old = receipts.lock().unwrap()[0].clone();
            // This damage is outside the future 32x32 image. A stale flush
            // would slice beyond the replacement DecodedImage and panic.
            let dirty = InclusiveRectangle {
                left: 48,
                top: 48,
                right: 63,
                bottom: 63,
            };
            let blue_rows = [0x1f, 0x00].repeat(16 * 16);
            for frame in bitmap_wire_frames(dirty, &blue_rows) {
                server_end.write_all(&frame).await.unwrap();
            }
            settle_actor().await;
            assert_eq!(packets.lock().unwrap().len(), 1);
            assert!(shared.publish().is_some());
            let mut activation = reactivation_script_frames();
            activation[1] = demand_active_frame_for(size, size);
            if ack_during_activation {
                // Leave the sequence awaiting DemandActive to ensure ACK is
                // handled by drive_reactivation, not the main actor loop.
                server_end.write_all(&activation[0]).await.unwrap();
                settle_actor().await;
                sender
                    .send(SessionCommand::Resize {
                        width: 800,
                        height: 600,
                    })
                    .await
                    .unwrap();
                sender
                    .send(SessionCommand::Resize {
                        width: 1024,
                        height: 768,
                    })
                    .await
                    .unwrap();
                settle_actor().await;
                let info = manager.get_session_info(&id).await.unwrap();
                assert_eq!(
                    (info.width, info.height),
                    (64, 64),
                    "requests during activation cannot replace negotiated dimensions"
                );
                tokio::time::advance(std::time::Duration::from_secs(5)).await;
                settle_actor().await;
                assert!(manager.get_session_info(&id).await.unwrap().display_stalled);
                sender
                    .send(SessionCommand::FrameConsumed {
                        receipt: old.clone(),
                        refresh: false,
                    })
                    .await
                    .unwrap();
                settle_actor().await;
                assert!(
                    !manager.get_session_info(&id).await.unwrap().display_stalled,
                    "ACK during reactivation must publish cleared display stall"
                );
                for frame in &activation[1..] {
                    server_end.write_all(frame).await.unwrap();
                }
            } else {
                for frame in activation {
                    server_end.write_all(&frame).await.unwrap();
                }
            }
            settle_actor().await;
            sender
                .send(SessionCommand::FrameConsumed {
                    receipt: old.clone(),
                    refresh: false,
                })
                .await
                .unwrap();
            settle_actor().await;
            assert_eq!(
                packets.lock().unwrap().len(),
                1,
                "old share damage must not flush after reactivation"
            );
            assert!(
                !sender.is_closed(),
                "reactivation ACK must not panic the actor"
            );
            let green_rows = [0xe0, 0x07].repeat(usize::from(size) * usize::from(size));
            for frame in bitmap_wire_frames(full_frame_rect(size, size), &green_rows) {
                server_end.write_all(&frame).await.unwrap();
            }
            settle_actor().await;
            assert_eq!(packets.lock().unwrap().len(), 2);
            let packet = packets.lock().unwrap()[1].clone();
            assert_eq!(packet[0], 1, "new share begins with full image");
            assert_eq!(u16::from_le_bytes([packet[1], packet[2]]), size);
            assert_eq!(
                &packet[15..],
                &[0, 255, 0, 255].repeat(usize::from(size) * usize::from(size))
            );
            let new = receipts.lock().unwrap()[1].clone();
            assert_ne!(
                new.generation, old.generation,
                "same-size shares also start a new generation"
            );
            sender
                .send(SessionCommand::FrameConsumed {
                    receipt: old,
                    refresh: true,
                })
                .await
                .unwrap();
            settle_actor().await;
            assert_eq!(
                packets.lock().unwrap().len(),
                2,
                "duplicate old ACK cannot release new flight"
            );
            manager.remove_session(&id).await.unwrap();
        }

        #[tokio::test(start_paused = true)]
        async fn smaller_reactivation_drops_pending_old_damage_before_ack() {
            verify_delivery_reactivation(32, false).await;
        }
        #[tokio::test(start_paused = true)]
        async fn same_size_reactivation_starts_new_delivery_generation() {
            verify_delivery_reactivation(64, false).await;
        }
        #[tokio::test(start_paused = true)]
        async fn ack_during_same_size_reactivation_publishes_stall_recovery() {
            verify_delivery_reactivation(64, true).await;
        }

        /// Task 7.4 acceptance (Req 11.4): DeactivateAll during an active
        /// session reactivates IN PLACE — stable session identity, no
        /// ConnectionLost, no reconnecting/backoff cycle, exactly one dial —
        /// and the reactivated stage keeps serving the same actor (a later
        /// Disconnect through the original command channel lands cleanly).
        #[tokio::test(start_paused = true)]
        async fn deactivate_all_reactivates_in_place_without_reconnect_cycle() {
            let manager = SessionManager::new();
            let session = create_scripted_real_session(
                &manager,
                "reactivate",
                true,
                reactivation_script_frames(),
                true,
            )
            .await;
            let mut rx = session.events_rx.clone();

            // Reactivation applied: the Finalized desktop size (96x80) is
            // published while the session stays connected.
            let applied = tokio::time::timeout(
                std::time::Duration::from_secs(600),
                wait_for(&session.id, &session.events, &mut rx, |events| {
                    events
                        .last()
                        .map(|(_, v)| {
                            status_string(v) == "connected"
                                && v["width"].as_u64() == Some(u64::from(REACTIVATION_WIDTH))
                                && v["height"].as_u64() == Some(u64::from(REACTIVATION_HEIGHT))
                        })
                        .unwrap_or(false)
                }),
            )
            .await;
            if applied.is_err() {
                panic!(
                    "reactivation did not apply the Finalized desktop size; recorded statuses: {:?}",
                    statuses_of(&info_events(&session.id, &session.events))
                );
            }

            let statuses = statuses_of(&info_events(&session.id, &session.events));
            assert!(
                !statuses.iter().any(|s| s == "reconnecting"),
                "DeactivateAll must not enter the reconnect cycle (got {statuses:?})"
            );
            assert!(
                !statuses.iter().any(|s| s.starts_with("error:")),
                "DeactivateAll must not error (got {statuses:?})"
            );
            // In place means exactly one dial for this connection.
            assert_eq!(session.connect_calls.load(Ordering::SeqCst), 1);

            // Identity stability: the same session id stays listed and live.
            let info = manager
                .get_session_info(&session.id)
                .await
                .expect("session remains listed after reactivation");
            assert_eq!(info.id, session.id);
            assert_eq!(manager.list_sessions().await.len(), 1);

            // Actor continuity: the ORIGINAL command channel still drives the
            // same actor — a user disconnect after reactivation lands cleanly
            // with no reconnect attempt and no error.
            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .expect("disconnect accepted after reactivation");
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;
            assert_eq!(session.connect_calls.load(Ordering::SeqCst), 1);
        }

        #[tokio::test(start_paused = true)]
        async fn resize_without_display_control_keeps_negotiated_dimensions() {
            let manager = SessionManager::new();
            let session =
                create_scripted_real_session(&manager, "resize-unavailable", false, vec![], true)
                    .await;
            let mut rx = session.events_rx.clone();
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events
                    .last()
                    .is_some_and(|(_, v)| status_string(v) == "connected")
            })
            .await;
            session
                .cmd_tx
                .send(SessionCommand::Resize {
                    width: 800,
                    height: 600,
                })
                .await
                .unwrap();
            for _ in 0..20 {
                tokio::task::yield_now().await;
            }
            let info = manager.get_session_info(&session.id).await.unwrap();
            assert_eq!((info.width, info.height), (64, 64));
            assert!(session.frame_sizes.lock().unwrap().is_empty());
            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .unwrap();
        }

        /// Boundary preserved (Req 2.3): DeactivateAll followed by a dead
        /// transport is still an unexpected transport drop — the session
        /// enters the existing reconnecting/backoff loop (identity
        /// preserved); in-place reactivation only applies when the server
        /// completes the sequence.
        #[tokio::test(start_paused = true)]
        async fn deactivate_all_then_transport_drop_still_reconnects_with_backoff() {
            let manager = SessionManager::new();
            // Script: deactivate, then the server vanishes (task ends →
            // duplex EOF) without ever sending DemandActive.
            let session = create_scripted_real_session(
                &manager,
                "react-drop",
                true,
                vec![deactivate_all_frame()],
                false,
            )
            .await;
            let mut rx = session.events_rx.clone();

            // EOF enters reconnect; a failed next dial stays reconnecting
            // with its reason instead of fabricating a connected desktop.
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().is_some_and(|(_, v)| {
                    status_string(v) == "reconnecting" && v["reconnectAttempts"] == 2
                })
            })
            .await;
            assert_eq!(session.connect_calls.load(Ordering::SeqCst), 2);
            let recorded = info_events(&session.id, &session.events);
            assert_eq!(
                statuses_of(&recorded)
                    .iter()
                    .filter(|s| *s == "connected")
                    .count(),
                1
            );
            assert!(
                recorded.last().unwrap().1["lastError"]
                    .as_str()
                    .unwrap()
                    .contains("already consumed")
            );

            // Same session identity across the reconnect cycle.
            assert!(manager.get_session_info(&session.id).await.is_some());

            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .expect("cancel accepted");
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;
        }

        /// While the reactivation sequence is waiting on the server, a user
        /// disconnect stops the session immediately: no reconnect attempt,
        /// terminal disconnected status (cancel semantics mirror Req 2.4).
        #[tokio::test(start_paused = true)]
        async fn disconnect_during_reactivation_window_stops_without_reconnect() {
            let manager = SessionManager::new();
            // DeactivateAll, then the server holds the transport open but
            // stays silent (no DemandActive): the actor is parked inside the
            // reactivation window.
            let session = create_scripted_real_session(
                &manager,
                "react-cancel",
                true,
                vec![deactivate_all_frame()],
                true,
            )
            .await;
            let mut rx = session.events_rx.clone();

            session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .expect("cancel accepted during reactivation");
            wait_for(&session.id, &session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;

            let statuses = statuses_of(&info_events(&session.id, &session.events));
            assert!(
                !statuses.iter().any(|s| s == "reconnecting"),
                "cancel during reactivation must not reconnect (got {statuses:?})"
            );
            assert!(
                !statuses.iter().any(|s| s.starts_with("error:")),
                "cancel during reactivation must not error (got {statuses:?})"
            );
            assert_eq!(session.connect_calls.load(Ordering::SeqCst), 1);
        }
    }

    /// Task 9.2 (Req 4.1/4.2/9.2/9.4) — the EGFX route through the REAL
    /// session loop. A scripted server creates the graphics dynamic virtual
    /// channel over DRDYNVC and sends ClearCodec frames; the tests assert the
    /// existing frame paths (IPC packets + SharedFrame/GPU feed) receive
    /// matching pixels, and that a graphics reset resizes the SharedFrame.
    ///
    /// All frames are real ironrdp_pdu encodings delivered over the
    /// in-memory framed seam (`ConnectionOutcome::RealSessionLocal`) — no
    /// network, no mocks of the decode path.
    mod real_session_egfx {
        use super::*;
        use crate::rdp::client::ConnectionOutcome;
        use ironrdp_core::{WriteBuf, encode_buf};
        use ironrdp_dvc::pdu::{CreateRequestPdu, DataPdu, DrdynvcDataPdu, DrdynvcServerPdu};
        use ironrdp_egfx::CHANNEL_NAME;
        use ironrdp_egfx::pdu::{
            CapabilitiesConfirmPdu, Codec1Type, CreateSurfacePdu, EndFramePdu, GfxPdu,
            MapSurfaceToOutputPdu, PixelFormat, RawCapabilitySet, ResetGraphicsPdu, StartFramePdu,
            Timestamp, WireToSurface1Pdu,
        };
        use ironrdp_graphics::zgfx::{CompressionMode, Compressor, compress_and_wrap_egfx};
        use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
        use ironrdp_pdu::rdp::capability_sets::SERVER_CHANNEL_ID;
        use ironrdp_pdu::x224::X224;
        use ironrdp_pdu::{encode_vec, geometry::ExclusiveRectangle};
        use std::sync::Mutex as StdMutex;

        /// Server-assigned DRDYNVC static channel id in the fixture.
        const DRDYNVC_CHANNEL_ID: u16 = 1005;
        /// Dynamic channel id the scripted server picks for EGFX.
        const GFX_DVC_CHANNEL_ID: u32 = 7;
        const W: u16 = 64;
        const H: u16 = 64;

        fn expected_rgba(width: u16, height: u16) -> Vec<u8> {
            let pixels = usize::from(width) * usize::from(height);
            let mut rgba = Vec::with_capacity(pixels * 4);
            for n in 0..pixels {
                rgba.extend_from_slice(&[
                    (n % 256) as u8,
                    100 + (n % 100) as u8,
                    200 + (n % 55) as u8,
                    255,
                ]);
            }
            rgba
        }

        fn expected_rgba_after_resize(width: u16, height: u16) -> Vec<u8> {
            let pixels = usize::from(width) * usize::from(height);
            let mut rgba = Vec::with_capacity(pixels * 4);
            for n in 0..pixels {
                rgba.extend_from_slice(&[
                    230 - (n % 200) as u8,
                    5 + (n % 250) as u8,
                    60 + (n % 177) as u8,
                    255,
                ]);
            }
            rgba
        }

        fn clearcodec_bgra(rgba: &[u8]) -> Vec<u8> {
            let (pixels, _) = rgba.as_chunks::<4>();
            pixels
                .iter()
                .flat_map(|p| [p[2], p[1], p[0], p[3]])
                .collect()
        }

        fn dvc_frame(pdu: &DrdynvcServerPdu) -> Vec<u8> {
            use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};

            let pdu_bytes = encode_vec(pdu).expect("scripted drdynvc PDU encodes");
            let header = encode_vec(&ChannelPduHeader {
                length: pdu_bytes.len() as u32,
                flags: ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
            })
            .expect("channel pdu header encodes");
            let mut user_data = Vec::with_capacity(header.len() + pdu_bytes.len());
            user_data.extend_from_slice(&header);
            user_data.extend_from_slice(&pdu_bytes);

            encode_vec(&X224(McsMessage::SendDataIndication(SendDataIndication {
                initiator_id: SERVER_CHANNEL_ID,
                channel_id: DRDYNVC_CHANNEL_ID,
                user_data: std::borrow::Cow::Owned(user_data),
            })))
            .expect("scripted x224 frame encodes")
        }

        fn gfx_data(pdu: &GfxPdu, compressor: &mut Compressor) -> Vec<u8> {
            let mut buf = WriteBuf::new();
            encode_buf(pdu, &mut buf).expect("GfxPdu encode");
            let wrapped = compress_and_wrap_egfx(buf.filled(), compressor, CompressionMode::Never)
                .expect("zgfx wrap");
            dvc_frame(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
                GFX_DVC_CHANNEL_ID,
                wrapped,
            ))))
        }

        fn full_rect(width: u16, height: u16) -> ExclusiveRectangle {
            ExclusiveRectangle {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            }
        }

        fn clearcodec_wire(expected: &[u8], width: u16, height: u16) -> WireToSurface1Pdu {
            WireToSurface1Pdu {
                surface_id: 1,
                codec_id: Codec1Type::ClearCodec,
                pixel_format: PixelFormat::XRgb,
                destination_rectangle: full_rect(width, height),
                bitmap_data: ironrdp_graphics::clearcodec::ClearCodecEncoder::new().encode(
                    &clearcodec_bgra(expected),
                    width,
                    height,
                ),
            }
        }

        /// Solid-color RGBA block (task 9.4 sub-rect fixtures).
        fn solid_rgba(width: u16, height: u16, value: [u8; 3]) -> Vec<u8> {
            let pixels = usize::from(width) * usize::from(height);
            let mut rgba = Vec::with_capacity(pixels * 4);
            for _ in 0..pixels {
                rgba.extend_from_slice(&[value[0], value[1], value[2], 255]);
            }
            rgba
        }

        /// ClearCodec wire for a solid-color sub-rect placed at (left, top)
        /// (task 9.4 partial-batch fixtures).
        fn sub_rect_clearcodec_wire(
            value: [u8; 3],
            left: u16,
            top: u16,
            width: u16,
            height: u16,
        ) -> WireToSurface1Pdu {
            WireToSurface1Pdu {
                surface_id: 1,
                codec_id: Codec1Type::ClearCodec,
                pixel_format: PixelFormat::XRgb,
                destination_rectangle: ExclusiveRectangle {
                    left,
                    top,
                    right: left + width,
                    bottom: top + height,
                },
                bitmap_data: ironrdp_graphics::clearcodec::ClearCodecEncoder::new().encode(
                    &clearcodec_bgra(&solid_rgba(width, height, value)),
                    width,
                    height,
                ),
            }
        }

        /// Overlay a solid block onto a full RGBA surface buffer.
        fn overlay_solid(
            full: &mut [u8],
            surface_w: u16,
            left: u16,
            top: u16,
            w: u16,
            h: u16,
            value: [u8; 3],
        ) {
            for y in top..top + h {
                for x in left..left + w {
                    let idx = (usize::from(y) * usize::from(surface_w) + usize::from(x)) * 4;
                    full[idx..idx + 4].copy_from_slice(&[value[0], value[1], value[2], 255]);
                }
            }
        }

        /// The scripted server's full frame sequence: DVC creation handshake,
        /// compositor surface setup, then one ClearCodec frame.
        fn egfx_server_frames(expected: &[u8]) -> Vec<Vec<u8>> {
            let mut compressor = Compressor::new();
            let mut frames = vec![dvc_frame(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
                GFX_DVC_CHANNEL_ID,
                CHANNEL_NAME.to_owned(),
            )))];

            let confirm = ironrdp_egfx::pdu::CapabilitySet::V10_7 {
                flags: ironrdp_egfx::pdu::CapabilitiesV107Flags::AVC_DISABLED
                    | ironrdp_egfx::pdu::CapabilitiesV107Flags::SMALL_CACHE,
            };
            frames.push(gfx_data(
                &GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu(RawCapabilitySet::from(
                    &confirm,
                ))),
                &mut compressor,
            ));
            frames.push(gfx_data(
                &GfxPdu::ResetGraphics(ResetGraphicsPdu {
                    width: u32::from(W),
                    height: u32::from(H),
                    monitors: Vec::new(),
                }),
                &mut compressor,
            ));
            frames.push(gfx_data(
                &GfxPdu::CreateSurface(CreateSurfacePdu {
                    surface_id: 1,
                    width: W,
                    height: H,
                    pixel_format: PixelFormat::XRgb,
                }),
                &mut compressor,
            ));
            frames.push(gfx_data(
                &GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
                    surface_id: 1,
                    output_origin_x: 0,
                    output_origin_y: 0,
                }),
                &mut compressor,
            ));
            frames.extend(egfx_frame_frames(
                1,
                clearcodec_wire(expected, W, H),
                &mut compressor,
            ));
            frames
        }

        /// StartFrame + WireToSurface1 + EndFrame as transport frames.
        fn egfx_frame_frames(
            frame_id: u32,
            wire: WireToSurface1Pdu,
            compressor: &mut Compressor,
        ) -> Vec<Vec<u8>> {
            vec![
                gfx_data(
                    &GfxPdu::StartFrame(StartFramePdu {
                        timestamp: Timestamp {
                            milliseconds: 0,
                            seconds: 0,
                            minutes: 0,
                            hours: 0,
                        },
                        frame_id,
                    }),
                    compressor,
                ),
                gfx_data(&GfxPdu::WireToSurface1(wire), compressor),
                gfx_data(&GfxPdu::EndFrame(EndFramePdu { frame_id }), compressor),
            ]
        }

        #[tokio::test(start_paused = true)]
        async fn delayed_display_caps_flush_latest_actor_resize_without_claiming_success() {
            use ironrdp_displaycontrol::pdu::{DisplayControlCapabilities, DisplayControlPdu};
            use ironrdp_tokio::FramedWrite as _;
            let manager = SessionManager::new();
            let (client, server) = tokio::io::duplex(64 * 1024);
            let client = StdMutex::new(Some(client));
            let connect: ConnectFn = Arc::new(move |_, _, _, _| {
                let client = client.lock().unwrap().take().unwrap();
                Box::pin(async move {
                    let mut result = crate::rdp::baseline::synthetic_connection_result(W, H);
                    let policy = Arc::new(CodecErrorPolicy::new());
                    crate::rdp::egfx::attach_to_stage_channels(
                        &mut result.static_channels,
                        DRDYNVC_CHANNEL_ID,
                        &policy,
                    );
                    ConnectionOutcome::RealSessionLocal {
                        connection_result: Box::new(result),
                        framed: Box::new(ironrdp_tokio::TokioFramed::new(client)),
                        codec_policy: policy,
                    }
                })
            });
            let (events, mut rx) = RecordedEvents::new();
            let id = manager
                .create_session_with_transport(
                    test_config("delayed-display"),
                    None,
                    Channel::new(|_| Ok(())),
                    SessionEventSink::recorded(events.clone()),
                    false,
                    None,
                    connect,
                )
                .await
                .unwrap();
            wait_for(&id, &events, &mut rx, |events| {
                events
                    .last()
                    .is_some_and(|(_, v)| status_string(v) == "connected")
            })
            .await;
            let sender = manager.get_cmd_sender(&id).await.unwrap();
            for (width, height) in [(800, 600), (1024, 768)] {
                sender
                    .send(SessionCommand::Resize { width, height })
                    .await
                    .unwrap();
            }
            for _ in 0..20 {
                tokio::task::yield_now().await;
            }
            let mut server = ironrdp_tokio::TokioFramed::new(server);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(10), server.read_pdu())
                    .await
                    .is_err()
            );
            server
                .write_all(&dvc_frame(&DrdynvcServerPdu::Create(
                    CreateRequestPdu::new(9, ironrdp_displaycontrol::CHANNEL_NAME.to_owned()),
                )))
                .await
                .unwrap();
            // DRDYNVC may emit its capability response as well as CREATE.
            // Drain handshake traffic, proving none is our pending layout.
            let dimensions: Vec<_> = 1024u32
                .to_le_bytes()
                .into_iter()
                .chain(768u32.to_le_bytes())
                .collect();
            let mut responses = 0;
            while let Ok(response) =
                tokio::time::timeout(std::time::Duration::from_millis(10), server.read_pdu()).await
            {
                let (_, payload) = response.unwrap();
                assert!(
                    !payload
                        .windows(dimensions.len())
                        .any(|bytes| bytes == dimensions)
                );
                responses += 1;
            }
            assert!(responses >= 1);
            let caps =
                DisplayControlPdu::Caps(DisplayControlCapabilities::new(1, 8192, 8192).unwrap());
            server
                .write_all(&dvc_frame(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(
                    DataPdu::new(9, ironrdp_core::encode_vec(&caps).unwrap()),
                ))))
                .await
                .unwrap();
            let (_, request) =
                tokio::time::timeout(std::time::Duration::from_secs(1), server.read_pdu())
                    .await
                    .unwrap()
                    .unwrap();
            assert!(
                request
                    .windows(dimensions.len())
                    .any(|bytes| bytes == dimensions)
            );
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(10), server.read_pdu())
                    .await
                    .is_err(),
                "only the latest pending request is sent once"
            );
            let info = manager.get_session_info(&id).await.unwrap();
            assert_eq!(
                (info.width, info.height),
                (u32::from(W), u32::from(H)),
                "a successful write is not a server-confirmed resize"
            );
            sender.send(SessionCommand::Disconnect).await.unwrap();
        }

        /// Captured frame packets, in emission order (raw IPC bytes).
        type CapturedPackets = Arc<StdMutex<Vec<Vec<u8>>>>;

        /// Frame-capturing IPC channel: records every raw packet verbatim.
        fn packet_capturing_channel(
            manager: &SessionManager,
        ) -> (tauri::ipc::Channel<InvokeResponseBody>, CapturedPackets) {
            let packets: CapturedPackets = Arc::new(StdMutex::new(Vec::new()));
            let sink = Arc::clone(&packets);
            let channel = consuming_channel(manager, move |response| {
                if let InvokeResponseBody::Raw(bytes) = response {
                    sink.lock().unwrap().push(bytes);
                }
                Ok(())
            });
            (channel, packets)
        }

        /// Duplex capacity for the EGFX scripted server: ClearCodec frames on
        /// high-entropy fixture patterns encode to tens of KB each, and the
        /// whole script is written up-front — the buffer must hold it (the
        /// tiny reactivation frames never stressed the 64 KB default).
        const EGFX_DUPLEX_CAPACITY: usize = 4 * 1024 * 1024;

        struct EgfxTestSession {
            receipts: Arc<StdMutex<Vec<FrameReceipt>>>,
            session: TestSession,
            packets: CapturedPackets,
            shared_frame: Arc<SharedFrame>,
        }

        /// Create a real-session actor over the in-memory seam whose
        /// ConnectionResult carries the task 9.2 EGFX wiring (the actor-seam
        /// twin of `attempt_rdp_connection`'s connector attachment).
        async fn create_egfx_session(
            manager: &SessionManager,
            tag: &str,
            server_frames: Vec<Vec<u8>>,
            shared_frame: Arc<SharedFrame>,
        ) -> EgfxTestSession {
            create_egfx_session_with_ack(manager, tag, server_frames, shared_frame, true).await
        }

        async fn create_egfx_session_with_ack(
            manager: &SessionManager,
            tag: &str,
            server_frames: Vec<Vec<u8>>,
            shared_frame: Arc<SharedFrame>,
            auto_ack: bool,
        ) -> EgfxTestSession {
            let (events, events_rx) = RecordedEvents::new();
            let frame_sizes = Arc::new(StdMutex::new(Vec::new()));
            let connect_calls = Arc::new(AtomicUsize::new(0));
            let (channel, packets) = packet_capturing_channel(manager);
            let receipts = Arc::new(StdMutex::new(Vec::new()));
            let channel = if auto_ack {
                channel
            } else {
                let receipts = Arc::clone(&receipts);
                let packets = Arc::clone(&packets);
                Channel::new(move |response| {
                    match response {
                        InvokeResponseBody::Json(json) => receipts
                            .lock()
                            .unwrap()
                            .push(serde_json::from_str(&json).unwrap()),
                        InvokeResponseBody::Raw(bytes) => packets.lock().unwrap().push(bytes),
                    }
                    Ok(())
                })
            };
            let (client_end, mut server_end) = tokio::io::duplex(EGFX_DUPLEX_CAPACITY);

            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt as _;
                for frame in server_frames {
                    server_end
                        .write_all(&frame)
                        .await
                        .expect("scripted EGFX server write");
                    // Immediate-consumer fixtures pace PDUs so each batch can be consumed.
                    // Withheld-ACK fixtures deliberately flood the same production loop.
                    if auto_ack {
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                }
                // Park like an idle server: the session stays live.
                std::future::pending::<()>().await;
            });

            let scripted_once = StdMutex::new(Some(client_end));
            let calls = Arc::clone(&connect_calls);
            let connect: ConnectFn = Arc::new(move |_config, _password, _width, _height| {
                calls.fetch_add(1, Ordering::SeqCst);
                let scripted = scripted_once.lock().unwrap().take();
                Box::pin(async move {
                    match scripted {
                        Some(stream) => {
                            let mut connection_result =
                                crate::rdp::baseline::synthetic_connection_result(W, H);
                            // Task 9.3: the scripted connection wires the
                            // SAME policy into the EGFX handler that the
                            // actor loop samples — mirroring
                            // `attempt_rdp_connection`'s connector-side
                            // creation and hand-off.
                            let codec_policy = Arc::new(CodecErrorPolicy::new());
                            crate::rdp::egfx::attach_to_stage_channels(
                                &mut connection_result.static_channels,
                                DRDYNVC_CHANNEL_ID,
                                &codec_policy,
                            );
                            ConnectionOutcome::RealSessionLocal {
                                connection_result: Box::new(connection_result),
                                framed: Box::new(ironrdp_tokio::TokioFramed::new(stream)),
                                codec_policy,
                            }
                        }
                        None => ConnectionOutcome::Failed(
                            "scripted EGFX session already consumed".to_string(),
                        ),
                    }
                })
            });

            let id = manager
                .create_session_with_transport(
                    test_config(tag),
                    None,
                    channel,
                    SessionEventSink::recorded(Arc::clone(&events)),
                    true,
                    Some(Arc::clone(&shared_frame)),
                    connect,
                )
                .await
                .expect("scripted EGFX session creation should succeed");
            let cmd_tx = manager
                .get_cmd_sender(&id)
                .await
                .expect("session handle exists right after creation");
            EgfxTestSession {
                receipts,
                session: TestSession {
                    id,
                    cmd_tx,
                    events,
                    events_rx,
                    frame_sizes,
                    connect_calls,
                },
                packets,
                shared_frame,
            }
        }

        /// Poll until `pred(packet_count)` holds, bounded by a generous
        /// paused-clock timeout (auto-advance keeps this deterministic).
        async fn wait_for_packets(egfx: &EgfxTestSession, pred: impl Fn(usize) -> bool) -> usize {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
            loop {
                let count = egfx.packets.lock().unwrap().len();
                if pred(count) {
                    return count;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "timed out waiting for frame packets; recorded statuses: {:?}",
                    statuses_of(&info_events(&egfx.session.id, &egfx.session.events))
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }

        async fn verify_withheld_delivery(stall: bool) {
            let manager = SessionManager::new();
            let base = expected_rgba(W, H);
            let mut expected = base.clone();
            let mut frames = egfx_server_frames(&base);
            let mut compressor = Compressor::new();
            for (index, (x, y, color)) in [
                (2, 2, [9, 130, 240]),
                (44, 6, [240, 9, 130]),
                (8, 8, [17, 18, 19]),
            ]
            .into_iter()
            .enumerate()
            {
                frames.extend(egfx_frame_frames(
                    index as u32 + 2,
                    sub_rect_clearcodec_wire(color, x, y, 16, 16),
                    &mut compressor,
                ));
                overlay_solid(&mut expected, W, x, y, 16, 16, color);
            }
            let egfx = create_egfx_session_with_ack(
                &manager,
                "withheld",
                frames,
                SharedFrame::new(u32::from(W), u32::from(H)),
                false,
            )
            .await;
            wait_for_packets(&egfx, |n| n == 1).await;
            // Let real upstream decode/composition finish while UI does NOT ACK.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if egfx
                    .shared_frame
                    .publish()
                    .is_some_and(|s| s.data == expected)
                {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "latest decoded image missing"
                );
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            assert_eq!(
                egfx.packets.lock().unwrap().len(),
                1,
                "no packet queue while UI waits"
            );
            let receipt = egfx.receipts.lock().unwrap()[0].clone();
            if stall {
                tokio::time::advance(std::time::Duration::from_secs(5)).await;
                tokio::task::yield_now().await;
                assert!(
                    manager
                        .get_session_info(&egfx.session.id)
                        .await
                        .unwrap()
                        .display_stalled
                );
                assert_eq!(egfx.packets.lock().unwrap().len(), 1);
            }
            egfx.session
                .cmd_tx
                .send(SessionCommand::FrameConsumed {
                    receipt: receipt.clone(),
                    refresh: false,
                })
                .await
                .unwrap();
            wait_for_packets(&egfx, |n| n == 2).await;
            let packets = egfx.packets.lock().unwrap().clone();
            let initial = parse_packet(&packets[0]);
            let next = parse_packet(&packets[1]);
            assert_eq!(next.kind, if stall { 1 } else { 2 });
            let mut received = initial.pixels;
            let mut source = 0;
            for (x, y, width, height) in next.rects {
                for row in y..y + height {
                    let dest = (usize::from(row) * usize::from(W) + usize::from(x)) * 4;
                    let count = usize::from(width) * 4;
                    received[dest..dest + count]
                        .copy_from_slice(&next.pixels[source..source + count]);
                    source += count;
                }
            }
            assert_eq!(
                received, expected,
                "all disjoint and overlapping damage survives"
            );
            assert!(
                !manager
                    .get_session_info(&egfx.session.id)
                    .await
                    .unwrap()
                    .display_stalled
            );
            // Duplicate old ACK may not release the new packet or trigger refresh.
            egfx.session
                .cmd_tx
                .send(SessionCommand::FrameConsumed {
                    receipt,
                    refresh: true,
                })
                .await
                .unwrap();
            tokio::task::yield_now().await;
            assert_eq!(egfx.packets.lock().unwrap().len(), 2);
            disconnect_and_wait(&egfx).await;
        }

        #[tokio::test(start_paused = true)]
        async fn egfx_delayed_ui_ack_preserves_latest_pixels() {
            verify_withheld_delivery(false).await;
        }
        #[tokio::test(start_paused = true)]
        async fn egfx_dropped_ui_ack_stalls_and_full_refresh_recovers_latest_pixels() {
            verify_withheld_delivery(true).await;
        }

        /// One parsed frame packet: kind byte, surface dims, dirty rects
        /// (left, top, width, height) and the packed RGBA payload.
        struct ParsedPacket {
            kind: u8,
            width: u16,
            height: u16,
            rects: Vec<(u16, u16, u16, u16)>,
            pixels: Vec<u8>,
        }

        fn parse_packet(packet: &[u8]) -> ParsedPacket {
            let kind = packet[0];
            let width = u16::from_le_bytes([packet[1], packet[2]]);
            let height = u16::from_le_bytes([packet[3], packet[4]]);
            let count = u16::from_le_bytes([packet[5], packet[6]]) as usize;
            let mut rects = Vec::with_capacity(count);
            let mut offset = 7;
            for _ in 0..count {
                rects.push((
                    u16::from_le_bytes([packet[offset], packet[offset + 1]]),
                    u16::from_le_bytes([packet[offset + 2], packet[offset + 3]]),
                    u16::from_le_bytes([packet[offset + 4], packet[offset + 5]]),
                    u16::from_le_bytes([packet[offset + 6], packet[offset + 7]]),
                ));
                offset += 8;
            }
            ParsedPacket {
                kind,
                width,
                height,
                rects,
                pixels: packet[offset..].to_vec(),
            }
        }

        async fn disconnect_and_wait(egfx: &EgfxTestSession) {
            let mut rx = egfx.session.events_rx.clone();
            egfx.session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .expect("disconnect accepted");
            wait_for(&egfx.session.id, &egfx.session.events, &mut rx, |events| {
                events.last().map(|(_, v)| status_string(v)) == Some("disconnected".to_string())
            })
            .await;
        }

        /// The EGFX DVC route renders ClearCodec frames through the existing
        /// packet + SharedFrame paths with matching pixels (Req 9.2, 4.1).
        #[tokio::test(start_paused = true)]
        async fn egfx_clearcodec_frame_reaches_packets_and_gpu_feed() {
            let manager = SessionManager::new();
            let expected = expected_rgba(W, H);
            let egfx = create_egfx_session(
                &manager,
                "egfx-clear",
                egfx_server_frames(&expected),
                SharedFrame::new(u32::from(W), u32::from(H)),
            )
            .await;

            wait_for_packets(&egfx, |n| n >= 1).await;
            let packets = egfx.packets.lock().unwrap().clone();
            let parsed = parse_packet(&packets[0]);
            assert_eq!(
                parsed.kind, 1,
                "task 9.4 (design L316): the initial frame after surface creation is a full kind-1 packet"
            );
            assert_eq!((parsed.width, parsed.height), (W, H));
            assert_eq!(
                parsed.rects,
                vec![(0, 0, W, H)],
                "kind 1 carries the full-surface rect"
            );
            assert_eq!(
                parsed.pixels, expected,
                "canvas packet pixels match the decoded ClearCodec frame"
            );

            // GPU feed: the SharedFrame snapshot is exactly what
            // GpuRenderer::render uploads to the wgpu texture.
            let snapshot = egfx.shared_frame.publish().expect("dirty after the frame");
            assert_eq!(snapshot.width, u32::from(W));
            assert_eq!(snapshot.height, u32::from(H));
            assert_eq!(snapshot.data, expected);

            // No H.264 transport anywhere on this route (Req 9.4).
            for packet in &packets {
                assert_ne!(packet[0], 3, "no kind-3/H.264 packet may be emitted");
            }

            disconnect_and_wait(&egfx).await;
        }

        /// A server graphics reset (resize) resizes the decoded image, the
        /// SharedFrame follows, and the next frame repaints in full (Req 4.1).
        #[tokio::test(start_paused = true)]
        async fn egfx_reset_graphics_resizes_shared_frame_and_repaints() {
            let manager = SessionManager::new();
            let expected = expected_rgba(W, H);
            let expected2 = expected_rgba_after_resize(96, 64);

            // Script: initial frame, then ResetGraphics to 96x64 + new
            // surface + a full ClearCodec frame at the new size.
            let mut server_frames = egfx_server_frames(&expected);
            let mut compressor = Compressor::new();
            server_frames.push(gfx_data(
                &GfxPdu::ResetGraphics(ResetGraphicsPdu {
                    width: 96,
                    height: 64,
                    monitors: Vec::new(),
                }),
                &mut compressor,
            ));
            server_frames.push(gfx_data(
                &GfxPdu::CreateSurface(CreateSurfacePdu {
                    surface_id: 1,
                    width: 96,
                    height: 64,
                    pixel_format: PixelFormat::XRgb,
                }),
                &mut compressor,
            ));
            server_frames.push(gfx_data(
                &GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
                    surface_id: 1,
                    output_origin_x: 0,
                    output_origin_y: 0,
                }),
                &mut compressor,
            ));
            server_frames.extend(egfx_frame_frames(
                2,
                clearcodec_wire(&expected2, 96, 64),
                &mut compressor,
            ));

            let egfx = create_egfx_session(
                &manager,
                "egfx-reset",
                server_frames,
                SharedFrame::new(u32::from(W), u32::from(H)),
            )
            .await;

            // Wait until the post-reset packet (surface width 96) arrived.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
            let resized_packet = loop {
                let packets = egfx.packets.lock().unwrap().clone();
                let describe = |packets: &[Vec<u8>]| {
                    packets
                        .iter()
                        .map(|p| {
                            format!(
                                "kind={} w={} len={}",
                                p[0],
                                u16::from_le_bytes([p[1], p[2]]),
                                p.len()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                if let Some(p) = packets
                    .iter()
                    .find(|p| p.len() >= 4 && u16::from_le_bytes([p[1], p[2]]) == 96)
                {
                    break p.clone();
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "post-reset packet never arrived; packets so far: [{}]; recorded statuses: {:?}",
                    describe(&packets),
                    statuses_of(&info_events(&egfx.session.id, &egfx.session.events))
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            };

            let parsed = parse_packet(&resized_packet);
            assert_eq!(
                parsed.kind, 1,
                "task 9.4 (design L316): a resize delivers a full kind-1 packet"
            );
            assert_eq!(
                (parsed.width, parsed.height),
                (96, 64),
                "packet carries the resized surface dims"
            );
            assert_eq!(parsed.rects, vec![(0, 0, 96, 64)]);
            assert_eq!(parsed.pixels, expected2);

            // SharedFrame resized + fully repainted at the new size.
            let snapshot = egfx
                .shared_frame
                .publish()
                .expect("dirty after the resize frame");
            assert_eq!(snapshot.width, 96);
            assert_eq!(snapshot.height, 64);
            assert_eq!(snapshot.data, expected2);

            disconnect_and_wait(&egfx).await;
        }

        /// Task 9.4 (design L316; Reqs 4.1/4.4/9.8): the initial delivery
        /// after surface creation is a full kind-1 frame; each subsequent
        /// drained batch emits ONE kind-2 packet scoped to THAT batch's
        /// own bounding rectangle (no cross-batch smearing), and the
        /// SharedFrame ends holding the complete composited image —
        /// disjoint updates accumulate without losing pixels.
        #[tokio::test(start_paused = true)]
        async fn egfx_partial_batches_keep_their_own_bounding_rects() {
            let manager = SessionManager::new();
            let base = expected_rgba(W, H);
            let color_a: [u8; 3] = [9, 130, 240];
            let color_b: [u8; 3] = [240, 9, 130];

            // Script: initial full frame, then two disjoint sub-rect
            // frames, each drained as its own processed-PDU batch.
            let mut server_frames = egfx_server_frames(&base);
            let mut compressor = Compressor::new();
            server_frames.extend(egfx_frame_frames(
                2,
                sub_rect_clearcodec_wire(color_a, 16, 16, 32, 32),
                &mut compressor,
            ));
            server_frames.extend(egfx_frame_frames(
                3,
                sub_rect_clearcodec_wire(color_b, 44, 6, 16, 16),
                &mut compressor,
            ));

            let egfx = create_egfx_session(
                &manager,
                "egfx-batch",
                server_frames,
                SharedFrame::new(u32::from(W), u32::from(H)),
            )
            .await;

            wait_for_packets(&egfx, |n| n >= 3).await;
            let packets = egfx.packets.lock().unwrap().clone();
            assert_eq!(packets.len(), 3, "one packet per drained batch");

            // Initial delivery: kind 1, complete first image.
            let first = parse_packet(&packets[0]);
            assert_eq!(first.kind, 1);
            assert_eq!((first.width, first.height), (W, H));
            assert_eq!(first.rects, vec![(0, 0, W, H)]);
            assert_eq!(first.pixels, base);

            // Batch 2: kind 2 scoped to its own bounding rect only.
            let second = parse_packet(&packets[1]);
            assert_eq!(second.kind, 2);
            assert_eq!(second.rects, vec![(16, 16, 32, 32)]);
            assert_eq!(second.pixels, solid_rgba(32, 32, color_a));

            // Batch 3: kind 2 scoped to its own rect — batch 2's damage
            // is not re-sent and batch 3's rect is not widened by it.
            let third = parse_packet(&packets[2]);
            assert_eq!(third.kind, 2);
            assert_eq!(third.rects, vec![(44, 6, 16, 16)]);
            assert_eq!(third.pixels, solid_rgba(16, 16, color_b));

            // Complete composited image in the GPU feed (Req 4.1).
            let mut composite = base.clone();
            overlay_solid(&mut composite, W, 16, 16, 32, 32, color_a);
            overlay_solid(&mut composite, W, 44, 6, 16, 16, color_b);
            let snapshot = egfx.shared_frame.publish().expect("dirty after batches");
            assert_eq!(snapshot.width, u32::from(W));
            assert_eq!(snapshot.height, u32::from(H));
            assert_eq!(snapshot.data, composite);

            disconnect_and_wait(&egfx).await;
        }

        // ------------------------------------------------------------------
        // Task 9.3 (Req 9.6/9.7): terminal codec/decode errors over the
        // actor seam — unsupported AVC terminates without decode/retry,
        // three consecutive decode failures terminate, a successful decode
        // resets the counter, and transport-drop reconnect (Req 2.3) keeps
        // its semantics.
        // ------------------------------------------------------------------

        /// A ClearCodec W2S1 update whose payload cannot decode — the
        /// dependency spike's failure twin (undecodable supported codec).
        fn malformed_clearcodec_wire() -> WireToSurface1Pdu {
            WireToSurface1Pdu {
                surface_id: 1,
                codec_id: Codec1Type::ClearCodec,
                pixel_format: PixelFormat::XRgb,
                destination_rectangle: full_rect(W, H),
                bitmap_data: vec![0xAB; 8],
            }
        }

        /// An AVC444 W2S1 update — upstream forwards it to the handler
        /// WITHOUT decoding (no H.264 decoder configured).
        fn avc444_wire() -> WireToSurface1Pdu {
            WireToSurface1Pdu {
                surface_id: 1,
                codec_id: Codec1Type::Avc444,
                pixel_format: PixelFormat::XRgb,
                destination_rectangle: full_rect(W, H),
                bitmap_data: vec![0; 16],
            }
        }

        /// Server script frames: channel open + one good ClearCodec frame
        /// (`first_frame_id`), then the caller's extra frames.
        fn opened_channel_frames(first_frame_id: u32, extra: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
            let mut compressor = Compressor::new();
            let mut frames = vec![dvc_frame(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
                GFX_DVC_CHANNEL_ID,
                CHANNEL_NAME.to_owned(),
            )))];
            let confirm = ironrdp_egfx::pdu::CapabilitySet::V10_7 {
                flags: ironrdp_egfx::pdu::CapabilitiesV107Flags::AVC_DISABLED
                    | ironrdp_egfx::pdu::CapabilitiesV107Flags::SMALL_CACHE,
            };
            frames.push(gfx_data(
                &GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu(RawCapabilitySet::from(
                    &confirm,
                ))),
                &mut compressor,
            ));
            frames.push(gfx_data(
                &GfxPdu::ResetGraphics(ResetGraphicsPdu {
                    width: u32::from(W),
                    height: u32::from(H),
                    monitors: Vec::new(),
                }),
                &mut compressor,
            ));
            frames.push(gfx_data(
                &GfxPdu::CreateSurface(CreateSurfacePdu {
                    surface_id: 1,
                    width: W,
                    height: H,
                    pixel_format: PixelFormat::XRgb,
                }),
                &mut compressor,
            ));
            frames.push(gfx_data(
                &GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
                    surface_id: 1,
                    output_origin_x: 0,
                    output_origin_y: 0,
                }),
                &mut compressor,
            ));
            frames.extend(egfx_frame_frames(
                first_frame_id,
                clearcodec_wire(&expected_rgba(W, H), W, H),
                &mut compressor,
            ));
            frames.extend(extra);
            frames
        }

        /// Await the actor's terminal error status and return its message
        /// (deadline-bounded: fails with the recorded statuses instead of
        /// hanging when no terminal ever fires).
        async fn wait_for_terminal_error(egfx: &EgfxTestSession) -> String {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
            loop {
                let terminal = info_events(&egfx.session.id, &egfx.session.events)
                    .iter()
                    .rev()
                    .find(|(_, v)| status_string(v).starts_with("error:"))
                    .map(|(_, v)| status_string(v));
                if let Some(msg) = terminal {
                    return msg;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "timed out waiting for a terminal codec/decode error; \
                     recorded statuses: {:?}",
                    statuses_of(&info_events(&egfx.session.id, &egfx.session.events))
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }

        /// Assert the terminal run never entered the reconnect loop: no
        /// reconnecting status, exactly ONE dial (auto_reconnect is ON for
        /// these sessions, so a ConnectionLost-style reason WOULD have
        /// retried — the codec terminal must not).
        fn assert_no_retry(egfx: &EgfxTestSession) {
            let statuses = statuses_of(&info_events(&egfx.session.id, &egfx.session.events));
            assert!(
                !statuses.iter().any(|s| s == "reconnecting"),
                "a codec/decode terminal must not enter the reconnect loop: {statuses:?}"
            );
            assert_eq!(
                egfx.session.connect_calls.load(Ordering::SeqCst),
                1,
                "the incompatible configuration must not be re-dialed (Req 9.6 no-retry): \
                 statuses so far {statuses:?}"
            );
        }

        /// Req 9.6: an AVC444 wire update arriving on the EGFX route ends
        /// the session with an actionable codec error — no decode attempt
        /// (upstream forwards it pre-decode), no retry/reconnect, and a
        /// subsequent Connect gesture is admitted again (task 7.1).
        #[tokio::test(start_paused = true)]
        async fn egfx_avc444_update_terminates_session_without_retry() {
            let manager = SessionManager::new();
            let mut compressor = Compressor::new();
            let mut extra = Vec::new();
            extra.extend(egfx_frame_frames(2, avc444_wire(), &mut compressor));
            let egfx = create_egfx_session(
                &manager,
                "egfx-avc-terminal",
                opened_channel_frames(1, extra),
                SharedFrame::new(u32::from(W), u32::from(H)),
            )
            .await;

            let terminal = wait_for_terminal_error(&egfx).await;
            assert!(
                terminal.contains("avc444"),
                "error names the codec variant: {terminal}"
            );
            assert!(
                terminal.to_lowercase().contains("h.264"),
                "error says what the codec is: {terminal}"
            );
            assert!(
                terminal.to_lowercase().contains("connect again"),
                "error is actionable: {terminal}"
            );
            assert_no_retry(&egfx);

            // The actor stops with a persistent terminal error, and the
            // AVC frame itself never becomes pixels.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
            loop {
                let done = info_events(&egfx.session.id, &egfx.session.events)
                    .last()
                    .map(|(_, v)| status_string(v))
                    .is_some_and(|s| s.starts_with("error:"));
                if done {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "actor never ended after the codec terminal; statuses: {:?}",
                    statuses_of(&info_events(&egfx.session.id, &egfx.session.events))
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            let recorded = info_events(&egfx.session.id, &egfx.session.events);
            let (_, last) = recorded.last().expect("final event recorded");
            let last_error = last["lastError"]
                .as_str()
                .expect("lastError set")
                .to_string();
            assert!(
                last_error.contains("avc444"),
                "lastError keeps the actionable reason: {last_error}"
            );
            let packets = egfx.packets.lock().unwrap().clone();
            assert_eq!(
                packets.len(),
                1,
                "only the pre-AVC good frame became pixels; the AVC update did not decode"
            );

            // Task 7.1 (Req 11.8/11.9): the terminal entry is no longer
            // live, so a NEW Connect gesture restarts it in place.
            let restart = manager
                .create_session_with_transport(
                    test_config("egfx-avc-terminal"),
                    None,
                    scripted_frame_channel(
                        &manager,
                        FrameScript::Ok,
                        Arc::new(StdMutex::new(Vec::new())),
                    ),
                    SessionEventSink::recorded(Arc::clone(&egfx.session.events)),
                    true,
                    None,
                    scripted_connect(Arc::new(AtomicUsize::new(0))),
                )
                .await;
            assert!(
                restart.is_ok(),
                "a terminal codec error admits a fresh Connect gesture: {restart:?}"
            );
            assert_eq!(
                restart.expect("admitted"),
                egfx.session.id,
                "terminal entries restart in place (Req 11.9)"
            );
        }

        /// Req 9.7: three CONSECUTIVE failed frames terminate the session
        /// with a decode error — no retry, single dial.
        #[tokio::test(start_paused = true)]
        async fn egfx_three_consecutive_decode_failures_terminate() {
            let manager = SessionManager::new();
            let mut compressor = Compressor::new();
            let mut extra = Vec::new();
            for frame_id in 2..=4 {
                extra.extend(egfx_frame_frames(
                    frame_id,
                    malformed_clearcodec_wire(),
                    &mut compressor,
                ));
            }
            let egfx = create_egfx_session(
                &manager,
                "egfx-decode-terminal",
                opened_channel_frames(1, extra),
                SharedFrame::new(u32::from(W), u32::from(H)),
            )
            .await;

            let terminal = wait_for_terminal_error(&egfx).await;
            assert!(
                terminal.to_lowercase().contains("decode"),
                "error says the stream failed to decode: {terminal}"
            );
            assert!(
                terminal.contains("3"),
                "error states the consecutive-failure threshold: {terminal}"
            );
            assert_no_retry(&egfx);
        }

        /// Req 9.7 exact counter semantics: fail, fail, SUCCESS, fail, fail
        /// stays alive (the success reset the counter); only three
        /// CONSECUTIVE failures terminate. Liveness is observed through the
        /// successful frames' packets — a premature terminal would stop
        /// the later packets.
        #[tokio::test(start_paused = true)]
        async fn egfx_successful_decode_resets_failure_counter() {
            let manager = SessionManager::new();
            let mut compressor = Compressor::new();
            let good_wire = || clearcodec_wire(&expected_rgba(W, H), W, H);

            // 1 good (channel open), then F F G F F G F F F: without the
            // reset the second F-pair would already be strikes 3+4 and kill
            // the session before the second good frame's packet arrives.
            let mut extra = Vec::new();
            for (frame_id, wire) in [
                (2u32, malformed_clearcodec_wire()),
                (3, malformed_clearcodec_wire()),
                (4, good_wire()),
                (5, malformed_clearcodec_wire()),
                (6, malformed_clearcodec_wire()),
                (7, good_wire()),
                (8, malformed_clearcodec_wire()),
                (9, malformed_clearcodec_wire()),
                (10, malformed_clearcodec_wire()),
            ] {
                extra.extend(egfx_frame_frames(frame_id, wire, &mut compressor));
            }

            let egfx = create_egfx_session(
                &manager,
                "egfx-decode-reset",
                opened_channel_frames(1, extra),
                SharedFrame::new(u32::from(W), u32::from(H)),
            )
            .await;

            // Both good frames after the failure stretches reached the
            // packet path — THIS is the aliveness proof for "fail, fail,
            // success, fail, fail stays alive": packet #3 is the second
            // post-failure good frame (G7). Had the counter NOT reset at
            // G4, strikes would have hit 3 at F5 and killed the session
            // before G7 — packet #3 could never arrive.
            wait_for_packets(&egfx, |n| n >= 3).await;

            // The third CONSECUTIVE failure (frame 10) then terminates —
            // after the second reset, F8/F9/F10 are strikes 1..3.
            let terminal = wait_for_terminal_error(&egfx).await;
            assert!(
                terminal.to_lowercase().contains("decode"),
                "terminal names the decode failure: {terminal}"
            );
            assert!(
                terminal.contains("3"),
                "error states the consecutive-failure threshold: {terminal}"
            );
            let packets = egfx.packets.lock().unwrap().clone();
            assert_eq!(
                packets.len(),
                3,
                "exactly the three good frames became pixels; failures never did"
            );
            assert_no_retry(&egfx);
        }

        /// Req 2.3 guard: with the terminal policy wired, an ordinary
        /// transport drop (server EOF) still runs the reconnect loop —
        /// codec terminals and transport loss stay distinct.
        #[tokio::test(start_paused = true)]
        async fn egfx_transport_drop_still_reconnects() {
            let manager = SessionManager::new();
            let mut server_frames = opened_channel_frames(1, Vec::new());

            let (events, events_rx) = RecordedEvents::new();
            let frame_sizes = Arc::new(StdMutex::new(Vec::new()));
            let connect_calls = Arc::new(AtomicUsize::new(0));
            let (channel, packets) = packet_capturing_channel(&manager);
            let (client_end, mut server_end) = tokio::io::duplex(EGFX_DUPLEX_CAPACITY);

            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt as _;
                for frame in server_frames.drain(..) {
                    server_end
                        .write_all(&frame)
                        .await
                        .expect("scripted EGFX server write");
                }
                // Transport drop: the server half closes — NOT hold-open.
            });

            let scripted_once = StdMutex::new(Some(client_end));
            let calls = Arc::clone(&connect_calls);
            let connect: ConnectFn = Arc::new(move |_config, _password, _width, _height| {
                calls.fetch_add(1, Ordering::SeqCst);
                let scripted = scripted_once.lock().unwrap().take();
                Box::pin(async move {
                    match scripted {
                        Some(stream) => {
                            let mut connection_result =
                                crate::rdp::baseline::synthetic_connection_result(W, H);
                            let codec_policy = Arc::new(CodecErrorPolicy::new());
                            crate::rdp::egfx::attach_to_stage_channels(
                                &mut connection_result.static_channels,
                                DRDYNVC_CHANNEL_ID,
                                &codec_policy,
                            );
                            ConnectionOutcome::RealSessionLocal {
                                connection_result: Box::new(connection_result),
                                framed: Box::new(ironrdp_tokio::TokioFramed::new(stream)),
                                codec_policy,
                            }
                        }
                        None => ConnectionOutcome::Failed(
                            "scripted EGFX session already consumed".to_string(),
                        ),
                    }
                })
            });

            let id = manager
                .create_session_with_transport(
                    test_config("egfx-transport-drop"),
                    None,
                    channel,
                    SessionEventSink::recorded(Arc::clone(&events)),
                    true,
                    Some(SharedFrame::new(u32::from(W), u32::from(H))),
                    connect,
                )
                .await
                .expect("scripted EGFX session creation should succeed");
            let cmd_tx = manager
                .get_cmd_sender(&id)
                .await
                .expect("session handle exists right after creation");
            let egfx = EgfxTestSession {
                receipts: Arc::new(StdMutex::new(Vec::new())),
                session: TestSession {
                    id,
                    cmd_tx,
                    events,
                    events_rx,
                    frame_sizes,
                    connect_calls,
                },
                packets,
                shared_frame: SharedFrame::new(u32::from(W), u32::from(H)),
            };

            let mut rx = egfx.session.events_rx.clone();
            // The transport drop enters the reconnect loop (Req 2.3) —
            // with the codec-terminal policy in place this must NOT have
            // become a terminal.
            wait_for(&egfx.session.id, &egfx.session.events, &mut rx, |events| {
                events
                    .iter()
                    .any(|(_, v)| status_string(v) == "reconnecting")
            })
            .await;

            // The reconnect backoff (2s paused) then re-dials: the
            // transport-drop path runs its full retry semantics.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
            while egfx.session.connect_calls.load(Ordering::SeqCst) < 2 {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "reconnect never re-dialed; statuses: {:?}",
                    statuses_of(&info_events(&egfx.session.id, &egfx.session.events))
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }

            let statuses = statuses_of(&info_events(&egfx.session.id, &egfx.session.events));
            assert!(
                !statuses
                    .iter()
                    .any(|s| s.starts_with("error:") && s.contains("codec")),
                "a transport drop is not a codec terminal: {statuses:?}"
            );
            assert_eq!(
                egfx.session.connect_calls.load(Ordering::SeqCst),
                2,
                "initial dial plus the first reconnect attempt"
            );

            egfx.session
                .cmd_tx
                .send(SessionCommand::Disconnect)
                .await
                .expect("disconnect accepted");
        }
    }
}
