use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

use ironrdp_connector::{
    self as connector, ClientConnector, ConnectionResult, Credentials, ServerName,
};
use ironrdp_tokio::TokioFramed;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;

use crate::rdp::clipboard::SessionClipboardBackend;
use crate::rdp::network_metrics::{MeasuredStream, NetworkCounters};
use crate::rdp::peer_identity::{self, ConnectionTarget};
use crate::store::connections::ConnectionConfig;

/// Commands that can be sent to a running session actor.
#[derive(Debug)]
pub enum SessionCommand {
    SetDisplayPaused {
        paused: bool,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    ViewReturned,
    FrameConsumed {
        receipt: crate::rdp::frame_delivery::FrameReceipt,
        refresh: bool,
    },
    SendKey {
        key_code: u32,
        is_down: bool,
    },
    /// Typed characters as Unicode keyboard events (spec rdp-keyboard-layout
    /// 2.1): the remote gets exactly these, whatever its own layout.
    SendText {
        text: String,
    },
    SendMouse {
        x: i32,
        y: i32,
        button: Option<String>,
        event_type: String,
        scroll_delta: i32,
    },
    Resize {
        width: u32,
        height: u32,
        /// Desktop scale factor in percent; `None` keeps the current one.
        scale: Option<u32>,
    },
    // Legacy actor characterization seam; public IPC uses acknowledged ClipboardSync.
    #[cfg_attr(not(test), allow(dead_code))]
    ClipboardWrite {
        text: String,
    },
    ClipboardSync {
        epoch: String,
        payload: Option<crate::rdp::clipboard_payload::ClipboardPayload>,
        reply: crate::rdp::clipboard::ClipboardReply,
    },
    ClipboardContext {
        reply: tokio::sync::oneshot::Sender<crate::rdp::clipboard::ClipboardContext>,
    },
    ClipboardPaste {
        epoch: String,
        reply: crate::rdp::clipboard::ClipboardReply,
    },
    Disconnect,
    /// Spec rdp-native-renderer 1.3: the native layer failed; send frames to
    /// the web canvas from now on, starting with the whole current picture.
    UseCanvas,
    /// Task 5.3 (Req 12.2/12.10): force a mock-session connection loss so
    /// E2E can drive the shared reconnect loop. Exists ONLY in
    /// `test-support` builds (compiled out of production); a logged no-op on
    /// real sessions.
    #[cfg(feature = "test-support")]
    DropConnection,
}

/// RDP connection error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RdpError(pub String);

impl std::fmt::Display for RdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for RdpError {}

/// The framed TLS stream type used after TLS upgrade.
pub type TlsFramed = TokioFramed<ironrdp_tls::TlsStream<MeasuredStream<TcpStream>>>;

/// The framed stream of a legacy-security session: RC4 over plain TCP
/// (`rdp/legacy_security.rs`).
pub type LegacyFramed =
    TokioFramed<crate::rdp::legacy_security::LegacyStream<MeasuredStream<TcpStream>>>;

/// Outcome of the RDP connection attempt.
pub enum ConnectionOutcome {
    /// Real IronRDP connection established.
    RealSession {
        connection_result: Box<ConnectionResult>,
        framed: Box<TlsFramed>,
        network_counters: std::sync::Arc<NetworkCounters>,
        /// The session's shared codec-error policy (task 9.3): the EGFX
        /// handler attached during this connection reports into it and the
        /// session loop samples the upstream decode-failure counter.
        codec_policy: std::sync::Arc<crate::rdp::codec_policy::CodecErrorPolicy>,
        /// What each of this connection's shared folders achieved (task 5.3).
        share: Vec<crate::rdp::folder_share::status::ShareStatusCell>,
    },
    /// A session under Standard RDP Security, for a host whose Security
    /// setting is Legacy. The server is not authenticated, so no folder is
    /// shared with it.
    RealSessionLegacy {
        connection_result: Box<ConnectionResult>,
        framed: Box<LegacyFramed>,
        network_counters: std::sync::Arc<NetworkCounters>,
        codec_policy: std::sync::Arc<crate::rdp::codec_policy::CodecErrorPolicy>,
        share: Vec<crate::rdp::folder_share::status::ShareStatusCell>,
    },
    /// Task 7.4 characterization seam: a real session over an in-memory
    /// framed transport. Identical actor semantics to [`ConnectionOutcome::RealSession`],
    /// but the framed stream is a tokio duplex, so deterministic tests can
    /// script the server side of the session loop (Server Deactivate All, the
    /// Deactivation-Reactivation Sequence) without TLS/TCP. Production
    /// connectors never produce this variant.
    #[cfg_attr(not(test), allow(dead_code))]
    RealSessionLocal {
        connection_result: Box<ConnectionResult>,
        framed: Box<ironrdp_tokio::TokioFramed<tokio::io::DuplexStream>>,
        /// Task 9.3: the seam twin of the production `codec_policy` handle —
        /// the scripted connection's EGFX wiring reports into it.
        codec_policy: std::sync::Arc<crate::rdp::codec_policy::CodecErrorPolicy>,
    },
    /// Explicit synthetic session; never returned by the production connector.
    #[cfg(any(test, feature = "test-support"))]
    Synthetic,
    /// Connection failed; reason provided.
    Failed(String),
}

/// Attempt a TCP connection to the RDP server.
async fn tcp_connect(target: &ConnectionTarget<'_>) -> Result<TcpStream, RdpError> {
    let (host, port) = target.socket_target();
    let addr = format!("{}:{}", host, port);
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        TcpStream::connect((host, port)),
    )
    .await
    .map_err(|_| RdpError(format!("TCP connection to {} timed out", addr)))?
    .map_err(|e| RdpError(format!("TCP connection to {} failed: {}", addr, e)))?;

    log::info!("TCP connected to {}", addr);
    Ok(stream)
}

/// Build the IronRDP connector Config.
/// `pub(crate)` seam (task 3.5): the baseline harnesses reuse the production
/// config builder so synthetic fixtures decode through a production-shaped
/// connector state; behavior is unchanged.
pub(crate) fn build_connector_config(
    config: &ConnectionConfig,
    password: &str,
    width: u32,
    height: u32,
) -> connector::Config {
    use ironrdp_pdu::gcc::{ConnectionType, KeyboardType};
    use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
    use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};

    let domain = if config.domain.is_empty() {
        None
    } else {
        Some(config.domain.clone())
    };

    let profile = &config.host_profile;
    // Legacy hosts refuse a request that names TLS or NLA at all.
    let legacy = profile.security == crate::store::connections::SecurityLayer::Legacy;
    let desktop_scale = profile
        .desktop_scale
        .map(ironrdp_displaycontrol::pdu::MonitorLayoutEntry::adjust_desktop_scale_factor);
    let mut performance_flags = PerformanceFlags::empty();
    for (flag, enabled) in [
        (PerformanceFlags::DISABLE_WALLPAPER, profile.wallpaper),
        (
            PerformanceFlags::DISABLE_FULLWINDOWDRAG,
            profile.full_window_drag,
        ),
        (
            PerformanceFlags::DISABLE_MENUANIMATIONS,
            profile.menu_animations,
        ),
        (PerformanceFlags::DISABLE_THEMING, profile.themes),
        (
            PerformanceFlags::DISABLE_CURSOR_SHADOW,
            profile.cursor_shadow,
        ),
        (
            PerformanceFlags::DISABLE_CURSORSETTINGS,
            profile.cursor_blinking,
        ),
    ] {
        performance_flags.set(flag, !enabled);
    }
    performance_flags.set(
        PerformanceFlags::ENABLE_DESKTOP_COMPOSITION,
        profile.desktop_composition,
    );
    use crate::store::connections::NetworkHint;
    let connection_type = match profile.network_hint {
        NetworkHint::Lan => ConnectionType::Lan,
        NetworkHint::Wan => ConnectionType::Wan,
        NetworkHint::BroadbandHigh => ConnectionType::BroadbandHigh,
        NetworkHint::BroadbandLow => ConnectionType::BroadbandLow,
        NetworkHint::Satellite => ConnectionType::Satellite,
        NetworkHint::Modem => ConnectionType::Modem,
    };
    // A client request for the next connection, independent of codec support.
    // Server policy can override this; it is not evidence of ClearType acceptance.
    performance_flags.set(
        PerformanceFlags::ENABLE_FONT_SMOOTHING,
        config.font_smoothing,
    );

    // connector 0.12's Config has no `Default` — all fields are pub and the
    // literal is constructed explicitly (task 4.1 probe C8a shape). Values for
    // fields that the old 0.8 connector hardcoded internally are preserved:
    // Empty `alternate_shell`/`work_dir` were 0.8
    // constants; `enable_standard_rdp_security: false` matches 0.8, which
    // rejected standard RDP security at negotiation. Graphics advertisement
    // comes from CodecPolicy (task 9.1, single source of truth):
    // `support_dyn_vc_gfx_protocol` sets the
    // `SUPPORT_DYN_VC_GFX_PROTOCOL` early capability flag (Req 9.1) and the
    // embedded fast-path bitmap config is the no-EGFX fallback (Req 9.3).
    // Multitransport stays off; bulk compression is an explicit per-host request.
    // `remote_application_mode` off.
    connector::Config {
        credentials: Credentials::UsernamePassword {
            username: config.username.clone(),
            password: password.to_string(),
        },
        domain,
        enable_tls: !legacy,
        enable_credssp: !legacy,
        enable_standard_rdp_security: legacy,
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        // The session resolves Automatic before it connects (keyboard::report);
        // here the setting is either a fixed layout or nothing.
        keyboard_layout: match config.host_profile.keyboard_layout {
            crate::store::connections::KeyboardLayoutChoice::Fixed(klid) => klid,
            crate::store::connections::KeyboardLayoutChoice::Named(_) => 0,
        },
        keyboard_functional_keys_count: 12,
        connection_type,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize {
            width: width as u16,
            height: height as u16,
        },
        // 0 leaves scaling to the server (100% on Windows). The device scale
        // (100, 140 or 180%) follows the desktop scale, as Display Control does.
        desktop_scale_factor: desktop_scale.unwrap_or(0),
        monitor_layout: None,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: desktop_scale.map(|scale| {
            ironrdp_displaycontrol::pdu::DeviceScaleFactor::from_desktop_scale_factor(scale).value()
        }),
        monitor_extended_layout: None,
        relative_mouse_input: false,
        bitmap: Some(connector::BitmapConfig {
            color_depth: profile.color_depth.bits(),
            ..crate::rdp::codec_policy::CodecPolicy::fastpath_bitmap()
        }),
        client_build: 0,
        client_name: "rdp-client".to_owned(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        #[cfg(target_os = "windows")]
        platform: MajorPlatformType::WINDOWS,
        #[cfg(target_os = "macos")]
        platform: MajorPlatformType::MACINTOSH,
        #[cfg(target_os = "linux")]
        platform: MajorPlatformType::UNIX,
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        platform: MajorPlatformType::UNSPECIFIED,
        enable_server_pointer: true,
        request_data: None,
        // With NLA the sign-in happens before the session. Without it the
        // host signs in with the sent password only when asked to; a host
        // set to always prompt shows its sign-in screen all the same.
        autologon: legacy && !password.is_empty(),
        enable_audio_playback: false,
        enable_audio_capture: false,
        // The pointer is the local cursor (rdp/remote_pointer.rs), not part of frames.
        pointer_software_rendering: false,
        performance_flags,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        compression_type: {
            use crate::store::connections::BulkCompression;
            use ironrdp_pdu::rdp::client_info::CompressionType;
            match profile.bulk_compression {
                BulkCompression::Off => None,
                BulkCompression::Rdp4 => Some(CompressionType::K8),
                BulkCompression::Rdp5 => Some(CompressionType::K64),
                BulkCompression::Rdp6 => Some(CompressionType::Rdp6),
                BulkCompression::Rdp61 => Some(CompressionType::Rdp61),
            }
        },
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol:
            crate::rdp::codec_policy::CodecPolicy::support_dyn_vc_gfx_protocol(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: RailSupportLevel::SUPPORTED,
    }
}

/// Build a session-layer `ActiveStage` from a connector `ConnectionResult`.
///
/// Adapter for the 0.13 session API: `ActiveStage::new(ConnectionResult)` was
/// replaced upstream by `ActiveStageBuilder` with pub fields taken straight
/// from the connection result (upstream's documented pattern). Shared by the
/// session actor and the baseline harnesses so both construct the stage
/// identically.
pub(crate) fn active_stage_from_connection_result(
    result: ConnectionResult,
) -> ironrdp_session::ActiveStage {
    ironrdp_session::ActiveStageBuilder {
        static_channels: result.static_channels,
        user_channel_id: result.user_channel_id,
        io_channel_id: result.io_channel_id,
        message_channel_id: result.message_channel_id,
        share_id: result.share_id,
        compression_type: result.compression_type,
        enable_server_pointer: result.enable_server_pointer,
        pointer_software_rendering: result.pointer_software_rendering,
    }
    .build()
}

/// Attempt to establish an IronRDP session.
///
/// Follows the exact same pattern as the official IronRDP client:
/// 1. TCP connect
/// 2. connect_begin (X.224 negotiation)
/// 3. TLS upgrade and peer identity check on this socket
/// 4. connect_finalize (CredSSP/NLA + MCS/licensing/capabilities)
pub async fn attempt_rdp_connection(
    config: &ConnectionConfig,
    password: Option<&str>,
    width: u32,
    height: u32,
) -> ConnectionOutcome {
    attempt_rdp_connection_with_share(config, password, width, height, Vec::new()).await
}

/// What the user reads when X.224 negotiation fails. A server that refuses
/// TLS and NLA is named with the setting that reaches it.
fn negotiation_failure_text(error: &connector::ConnectorError) -> String {
    use ironrdp_pdu::nego::FailureCode;

    let refuses_tls = matches!(
        error.kind(),
        connector::ConnectorErrorKind::Negotiation(failure)
            if failure.code() == FailureCode::SSL_NOT_ALLOWED_BY_SERVER
    );
    if refuses_tls {
        "This host accepts only legacy RDP security, without TLS or NLA. To connect anyway, set \
         Security to Legacy in the host's settings: the encryption is weak and the host is not \
         authenticated."
            .to_owned()
    } else {
        format!("RDP negotiation begin failed: {error}")
    }
}

/// The rest of a connection under Standard RDP Security: no TLS upgrade, the
/// RC4 layer of `rdp/legacy_security.rs` between the library and the socket.
async fn finish_legacy_connection(
    config: &ConnectionConfig,
    mut connector: ClientConnector,
    tcp_stream: TcpStream,
    network_counters: std::sync::Arc<NetworkCounters>,
    codec_policy: std::sync::Arc<crate::rdp::codec_policy::CodecErrorPolicy>,
    share: Vec<crate::store::share_grants::ShareGrant>,
) -> ConnectionOutcome {
    log::warn!(
        "Connecting to {} with legacy RDP security: weak encryption, host not authenticated",
        config.host
    );
    let stream = crate::rdp::legacy_security::LegacyStream::new(MeasuredStream::new(
        tcp_stream,
        network_counters.clone(),
    ));
    let mut framed = TokioFramed::new(stream);
    let should_upgrade = match ironrdp_tokio::connect_begin(&mut framed, &mut connector).await {
        Ok(su) => su,
        Err(e) => {
            log::warn!("RDP negotiation begin failed: {:?}", e);
            return ConnectionOutcome::Failed(format!("RDP negotiation begin failed: {e}"));
        }
    };
    // Nothing to upgrade: the security exchange happens inside the stream.
    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
    let mut network_client = ReqwestNetworkClient::new();
    let connection_result = match ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut network_client,
        ServerName::new(config.host.clone()),
        Vec::new(),
        None,
    )
    .await
    .and_then(|outcome| outcome.into_connected())
    {
        Ok(result) => result,
        Err(e) => {
            log::warn!("RDP connection finalize failed: {:?}", e);
            return ConnectionOutcome::Failed(format!("RDP connection finalize failed: {e}"));
        }
    };
    log::info!(
        "RDP connection established with legacy security. Desktop size: {}x{}",
        connection_result.desktop_size.width,
        connection_result.desktop_size.height
    );
    // A folder goes only to a server whose identity was checked, and this
    // mode has none to check.
    let share = share
        .iter()
        .map(|_| {
            let status = crate::rdp::folder_share::status::ShareStatusCell::default();
            status.set(crate::rdp::folder_share::status::ShareState::Unverified {
                reason: "Folders are not shared over legacy RDP security: the host is not \
                         authenticated."
                    .into(),
            });
            status
        })
        .collect();
    ConnectionOutcome::RealSessionLegacy {
        connection_result: Box::new(connection_result),
        framed: Box::new(framed),
        network_counters,
        codec_policy,
        share,
    }
}

/// [`attempt_rdp_connection`], offering each folder in `share` as a drive when
/// the server and folder still match it (rdp-folder-sharing task 5.2).
pub(crate) async fn attempt_rdp_connection_with_share(
    config: &ConnectionConfig,
    password: Option<&str>,
    width: u32,
    height: u32,
    share: Vec<crate::store::share_grants::ShareGrant>,
) -> ConnectionOutcome {
    let password = password.unwrap_or_default();

    // Use the same DNS/IP destination for dialing and TLS name validation.
    let target = match ConnectionTarget::new(&config.host, config.port) {
        Ok(target) => target,
        Err(e) => return ConnectionOutcome::Failed(format!("Invalid RDP endpoint: {e}")),
    };
    let tcp_stream = match tcp_connect(&target).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("TCP connection failed: {}", e);
            return ConnectionOutcome::Failed(e.0);
        }
    };

    // With a shared folder, file data competes with input for this socket.
    // Send small packets at once and cap what the system queues for sending.
    if !share.is_empty() {
        let _ = tcp_stream.set_nodelay(true);
        if let Some(bytes) = config.host_profile.transfer_pace.send_buffer_bytes() {
            match socket2::SockRef::from(&tcp_stream).set_send_buffer_size(bytes) {
                Ok(()) => log::info!("Folder share: send buffer capped at {} KiB", bytes / 1024),
                Err(error) => log::warn!("Folder share: could not cap the send buffer: {error}"),
            }
        }
    }

    // Step 2: Create ClientConnector and begin negotiation
    let connector_config = build_connector_config(config, password, width, height);
    let client_addr = tcp_stream
        .local_addr()
        .unwrap_or_else(|_| std::net::SocketAddr::from(([0, 0, 0, 0], 0)));
    let mut connector = ClientConnector::new(connector_config, client_addr);
    connector.attach_static_channel(ironrdp_cliprdr::CliprdrClient::new(Box::new(
        SessionClipboardBackend::with_direction(config.host_profile.clipboard_direction),
    )));
    // Task 9.3 (Req 9.6/9.7): the session's shared codec-error policy —
    // the EGFX handler constructed by the attach below reports
    // unsupported-codec arrivals and decoded deliveries into it, and the
    // returned handle carries it to the session loop, which samples the
    // upstream decode-failure counter.
    let codec_policy = std::sync::Arc::new(crate::rdp::codec_policy::CodecErrorPolicy::new());
    // Task 9.2 (Req 9.2/9.4): register the graphics pipeline DVC on the
    // DRDYNVC static channel — the EGFX adapter (no H.264 decoder). Without
    // this, a real server's EGFX channel creation meets an unregistered
    // processor even though 9.1 advertises
    // `support_dyn_vc_gfx_protocol`.
    crate::rdp::egfx::attach_to_connector(&mut connector, &codec_policy);

    let network_counters = std::sync::Arc::new(NetworkCounters::default());
    if config.host_profile.security == crate::store::connections::SecurityLayer::Legacy {
        return finish_legacy_connection(
            config,
            connector,
            tcp_stream,
            network_counters,
            codec_policy,
            share,
        )
        .await;
    }
    let mut framed = TokioFramed::new(MeasuredStream::new(tcp_stream, network_counters.clone()));

    let should_upgrade = match ironrdp_tokio::connect_begin(&mut framed, &mut connector).await {
        Ok(su) => su,
        Err(e) => {
            log::warn!("RDP negotiation begin failed: {:?}", e);
            return ConnectionOutcome::Failed(negotiation_failure_text(&e));
        }
    };

    log::info!("RDP X.224 negotiation complete, upgrading to TLS...");

    let stream = match peer_identity::take_upgrade_stream(framed) {
        Ok(stream) => stream,
        Err(e) => return ConnectionOutcome::Failed(format!("RDP negotiation failed: {e}")),
    };
    let tls = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        peer_identity::handshake(stream, &target.endpoint),
    )
    .await
    {
        Ok(Ok(tls)) => tls,
        Ok(Err(e)) => return ConnectionOutcome::Failed(format!("TLS upgrade failed: {e}")),
        Err(_) => return ConnectionOutcome::Failed("TLS upgrade timed out".to_string()),
    };
    // An ordinary desktop permits self-signed hosts. Consumers of authenticated
    // identity call SignedTls::verify before releasing credentials or a share.
    // The drive channel is decided here, before the channel list is sent.
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    let share_status = if share.is_empty() {
        Vec::new()
    } else {
        let (rdpdr, statuses) =
            crate::rdp::folder_share::attach::drive_channel(&tls, &target.endpoint, share).await;
        if let Some(rdpdr) = rdpdr {
            connector.attach_static_channel(rdpdr);
            // Windows starts drive redirection only when the audio channel is
            // registered as well (FreeRDP and IronRDP's own client do the same).
            // It offers no formats, so the server sends no sound.
            connector.attach_static_channel(ironrdp_rdpsnd::client::Rdpsnd::new(Box::new(
                ironrdp_rdpsnd::client::NoopRdpsndBackend,
            )));
        }
        statuses
    };
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    let share_status: Vec<_> = share
        .iter()
        .map(|_| {
            let status = crate::rdp::folder_share::status::ShareStatusCell::default();
            status.set(crate::rdp::folder_share::status::ShareState::Unsupported {
                reason: "Folder sharing is available in the Windows, Linux and macOS builds."
                    .into(),
            });
            status
        })
        .collect();
    let (tls_stream, server_public_key) = tls.into_parts();
    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
    let mut tls_framed = TokioFramed::new(tls_stream);

    let server_name = ServerName::new(config.host.clone());
    let mut network_client = ReqwestNetworkClient::new();

    log::info!("Starting CredSSP/NLA finalization...");

    // Step 5: Finalize connection (CredSSP/NLA + MCS + licensing + capabilities).
    // 0.12 transport API: connect_finalize returns a ConnectionOutcome enum —
    // `into_connected()` yields the ConnectionResult and turns a Server
    // Redirection PDU into an error, matching the old 0.8 stack, which had no
    // redirection handling at this seam.
    let connection_result = match ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut tls_framed,
        &mut network_client,
        server_name,
        server_public_key,
        None,
    )
    .await
    {
        Ok(outcome) => match outcome.into_connected() {
            Ok(result) => result,
            Err(e) => {
                log::warn!("RDP connection was redirected (unsupported): {:?}", e);
                return ConnectionOutcome::Failed(format!("RDP connection finalize failed: {}", e));
            }
        },
        Err(e) => {
            log::warn!("RDP connection finalize failed: {:?}", e);
            return ConnectionOutcome::Failed(format!("RDP connection finalize failed: {}", e));
        }
    };

    log::info!(
        "RDP connection established! Desktop size: {}x{}",
        connection_result.desktop_size.width,
        connection_result.desktop_size.height
    );
    // Which static channels the server joined, by MCS channel ID.
    log::info!(
        "Static channels: drive {:?}, clipboard {:?}, I/O {}",
        connection_result
            .static_channels
            .get_channel_id_by_type::<crate::rdp::folder_share::channel::DriveChannel>(),
        connection_result
            .static_channels
            .get_channel_id_by_type::<ironrdp_cliprdr::CliprdrClient>(),
        connection_result.io_channel_id
    );

    ConnectionOutcome::RealSession {
        connection_result: Box::new(connection_result),
        framed: Box::new(tls_framed),
        network_counters,
        codec_policy,
        share: share_status,
    }
}

/// Task 4.3 — builder-adapter seam characterization.
///
/// `active_stage_from_connection_result` is the whole "builder" half of the
/// session-adapter boundary (0.13 replaced `ActiveStage::new(ConnectionResult)`
/// with `ActiveStageBuilder` pub fields). The observables below pin the
/// mappings the session layer depends on, deterministically and without a
/// server: static channels survive onto the stage (the clipboard SVC the
/// connector attaches — Req 3.5 session plumbing), the typed DVC accessor
/// surface declines for DisplayControl (the connector attaches no DVC —
/// found behavior also recorded by the smoke `display_control_resize`
/// field), and a builder-produced stage still encodes fast-path input and
/// graceful shutdown (lifecycle behavior preserved end-to-end).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_stage_adapter_carries_channels_and_preserves_stage_behavior() {
        use ironrdp_pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};

        // Production-shaped ConnectionResult with the clipboard SVC attached
        // (the connector attaches exactly this channel today).
        let mut result = crate::rdp::baseline::synthetic_connection_result(64, 64);
        result
            .static_channels
            .insert(ironrdp_cliprdr::CliprdrClient::new(Box::new(
                SessionClipboardBackend::new(),
            )));

        let mut stage = active_stage_from_connection_result(result);

        // static_channels passthrough: the clipboard SVC stays reachable
        // through the stage (session.rs flush_clipboard_messages depends on it).
        assert!(
            stage
                .get_svc_processor_mut::<ironrdp_cliprdr::CliprdrClient>()
                .is_some(),
            "attached clipboard SVC must be reachable through the built stage"
        );

        // Typed DVC accessors (0.13 shape): no DisplayControlClient is
        // attached, so the typed lookup declines and resize declines with it.
        assert!(
            stage
                .get_dvc::<ironrdp_displaycontrol::client::DisplayControlClient>()
                .is_none(),
            "no DisplayControl DVC is attached by the connector"
        );
        assert!(
            stage.encode_resize(1280, 720, None, None).is_none(),
            "resize must decline while the DisplayControl DVC is absent"
        );

        // Graceful shutdown still encodes through the built stage.
        let outputs = stage
            .graceful_shutdown()
            .expect("graceful shutdown encodes");
        assert!(
            outputs.iter().any(|out| matches!(
                out,
                ironrdp_session::ActiveStageOutput::ResponseFrame(frame) if !frame.is_empty()
            )),
            "graceful shutdown must produce a response frame"
        );

        // Fast-path input still encodes through the rebuilt fast-path processor.
        use ironrdp_graphics::image_processing::PixelFormat;
        let mut image = ironrdp_session::image::DecodedImage::new(PixelFormat::RgbA32, 64, 64);
        let outputs = stage
            .process_fastpath_input(
                &mut image,
                &[FastPathInputEvent::KeyboardEvent(
                    KeyboardFlags::empty(),
                    0x1E,
                )],
            )
            .expect("fast-path input encodes");
        assert!(
            outputs.iter().any(|out| matches!(
                out,
                ironrdp_session::ActiveStageOutput::ResponseFrame(frame) if !frame.is_empty()
            )),
            "fast-path input must produce a response frame"
        );
    }

    // Req 8.3 (task 5.1): a Windows build must identify itself with Windows
    // platform capabilities in the Client Information PDU. The cfg-mapped
    // `platform` field is pinned per build OS, so a regression to
    // UNSPECIFIED (or the wrong platform) fails the rust tier on every OS.
    #[test]
    fn connector_config_advertises_native_platform_capabilities() {
        use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;

        let config = crate::store::connections::ConnectionConfig {
            id: "drift-task-5-1-platform-pin".to_string(),
            name: "platform capability pin".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3389,
            username: "user".to_string(),
            domain: String::new(),
            color_accent: None,
            tags: Vec::new(),
            display_width: None,
            display_height: None,
            font_smoothing: true,

            host_profile: Default::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_connected_at: None,
            mock: false,
            library_id: String::new(),
            folder_id: None,
        };

        let connector = build_connector_config(&config, "pw", 1280, 720);

        #[cfg(target_os = "windows")]
        let expected = MajorPlatformType::WINDOWS;
        #[cfg(target_os = "macos")]
        let expected = MajorPlatformType::MACINTOSH;
        #[cfg(target_os = "linux")]
        let expected = MajorPlatformType::UNIX;
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        let expected = MajorPlatformType::UNSPECIFIED;

        assert_eq!(
            connector.platform, expected,
            "connector must advertise the build OS as its platform capability (Req 8.3)"
        );

        // Without a measured scale the server keeps its own default.
        assert_eq!(connector.desktop_scale_factor, 0);
        assert_eq!(connector.device_scale_factor, None);
        // A Retina window (200%) requests 200% desktop and 180% device scale.
        let mut scaled = config.clone();
        for (measured, desktop, device) in [(200, 200, 180), (150, 150, 140), (125, 125, 100)] {
            scaled.host_profile.desktop_scale = Some(measured);
            let connector = build_connector_config(&scaled, "pw", 2880, 1800);
            assert_eq!(connector.desktop_scale_factor, desktop);
            assert_eq!(connector.device_scale_factor, Some(device));
        }
    }

    /// A legacy host answers a request that names TLS or NLA with a
    /// failure, so the request names neither; every other host keeps both.
    #[test]
    fn connector_config_requests_standard_security_only_for_a_legacy_host() {
        let mut saved = serde_json::json!({
            "id": "legacy", "name": "Legacy", "host": "127.0.0.1",
            "port": 3389, "username": "user", "domain": "", "tags": [],
            "createdAt": "2026-11-21T00:00:00Z", "updatedAt": "2026-11-21T00:00:00Z"
        });
        let automatic: ConnectionConfig = serde_json::from_value(saved.clone()).unwrap();
        saved["hostProfile"] = serde_json::json!({"security": "legacy"});
        let legacy: ConnectionConfig = serde_json::from_value(saved).unwrap();

        let automatic = build_connector_config(&automatic, "pw", 1280, 720);
        assert!(automatic.enable_tls && automatic.enable_credssp);
        assert!(!automatic.enable_standard_rdp_security);
        assert!(!automatic.autologon, "NLA signs in before the session");
        let asked = build_connector_config(&legacy, "", 1280, 720);
        assert!(!asked.autologon, "no password, nothing to sign in with");
        let legacy = build_connector_config(&legacy, "pw", 1280, 720);
        assert!(!legacy.enable_tls && !legacy.enable_credssp);
        assert!(legacy.enable_standard_rdp_security);
        assert!(legacy.autologon);
    }

    #[test]
    fn a_host_that_refuses_tls_is_named_with_the_setting_that_reaches_it() {
        use ironrdp_pdu::nego::FailureCode;

        let refused = connector::ConnectorError::new(
            "negotiation failure",
            connector::ConnectorErrorKind::Negotiation(
                FailureCode::SSL_NOT_ALLOWED_BY_SERVER.into(),
            ),
        );
        let text = negotiation_failure_text(&refused);
        assert!(text.contains("Security to Legacy"), "{text}");
        let other = connector::ConnectorError::new(
            "negotiation failure",
            connector::ConnectorErrorKind::Negotiation(
                FailureCode::HYBRID_REQUIRED_BY_SERVER.into(),
            ),
        );
        let text = negotiation_failure_text(&other);
        assert!(text.starts_with("RDP negotiation begin failed"), "{text}");
    }

    #[test]
    fn connector_config_requests_saved_font_smoothing_only() {
        use ironrdp_pdu::rdp::client_info::PerformanceFlags;

        // Build from a persisted preference, through the real outgoing-config path.
        let mut saved = serde_json::json!({
            "id": "smoothing", "name": "Smoothing", "host": "127.0.0.1",
            "port": 3389, "username": "user", "domain": "", "tags": [],
            "createdAt": "2026-11-17T00:00:00Z", "updatedAt": "2026-11-17T00:00:00Z",
            "fontSmoothing": true
        });
        let enabled: ConnectionConfig = serde_json::from_value(saved.clone()).unwrap();
        saved["fontSmoothing"] = serde_json::json!(false);
        let disabled: ConnectionConfig = serde_json::from_value(saved).unwrap();
        let on = build_connector_config(&enabled, "pw", 1280, 720);
        let off = build_connector_config(&disabled, "pw", 1280, 720);
        assert!(
            on.performance_flags
                .contains(PerformanceFlags::ENABLE_FONT_SMOOTHING)
        );
        assert!(
            !off.performance_flags
                .contains(PerformanceFlags::ENABLE_FONT_SMOOTHING)
        );
        assert_eq!(
            on.performance_flags ^ off.performance_flags,
            PerformanceFlags::ENABLE_FONT_SMOOTHING,
            "changing smoothing must change only its upstream performance bit"
        );
        assert_eq!(
            off.performance_flags,
            PerformanceFlags::DISABLE_WALLPAPER
                | PerformanceFlags::DISABLE_MENUANIMATIONS
                | PerformanceFlags::DISABLE_THEMING
                | PerformanceFlags::DISABLE_CURSOR_SHADOW
        );
        assert_eq!(
            on.support_dyn_vc_gfx_protocol,
            off.support_dyn_vc_gfx_protocol
        );
        let on_bitmap = on.bitmap.unwrap();
        let off_bitmap = off.bitmap.unwrap();
        assert_eq!(on_bitmap.color_depth, off_bitmap.color_depth);
        assert_eq!(on_bitmap.lossy_compression, off_bitmap.lossy_compression);
        assert_eq!(on_bitmap.codecs, off_bitmap.codecs);
        assert_eq!(on.enable_tls, off.enable_tls);
        assert_eq!(on.enable_credssp, off.enable_credssp);
        assert_eq!(on.desktop_size, off.desktop_size);
    }

    // Task 9.1 (Req 9.1/9.3): the connector consumes CodecPolicy as the
    // single source of truth for graphics advertisement — the EGFX
    // early-capability switch and the no-EGFX fast-path bitmap fallback
    // config both come from the policy, never from ad-hoc literals here.
    #[test]
    fn graphics_preferences_map_to_connector_without_changing_security_or_codecs() {
        use ironrdp_pdu::rdp::client_info::CompressionType;
        for depth in [15, 16, 24, 32] {
            for (name, expected) in [
                ("off", None),
                ("rdp4", Some(CompressionType::K8)),
                ("rdp5", Some(CompressionType::K64)),
                ("rdp6", Some(CompressionType::Rdp6)),
                ("rdp61", Some(CompressionType::Rdp61)),
            ] {
                let value = serde_json::json!({"id":"graphics", "name":"Graphics", "host":"graphics.invalid", "port":3389,
                    "createdAt":"2026-11-17T00:00:00Z", "updatedAt":"2026-11-17T00:00:00Z",
                    "hostProfile":{"colorDepth":depth.to_string(),"bulkCompression":name}});
                let config = serde_json::from_value(value).unwrap();
                let actual = build_connector_config(&config, "", 800, 600);
                assert_eq!(actual.bitmap.as_ref().unwrap().color_depth, depth);
                assert_eq!(actual.compression_type, expected);
                assert!(
                    actual.enable_tls
                        && actual.enable_credssp
                        && !actual.enable_standard_rdp_security
                );
                assert!(actual.support_dyn_vc_gfx_protocol);
                assert!(actual.bitmap.as_ref().unwrap().codecs.0.is_empty());
                assert!(!actual.bitmap.as_ref().unwrap().lossy_compression);
            }
        }
    }

    #[test]
    fn host_profile_controls_protocol_flags_and_network_hint() {
        use ironrdp_pdu::rdp::client_info::PerformanceFlags;
        let saved = serde_json::json!({"id":"profile", "name":"profile", "host":"profile.invalid", "port":3389,
            "createdAt":"2026-11-17T00:00:00Z", "updatedAt":"2026-11-17T00:00:00Z",
            "hostProfile":{"wallpaper":true,"fullWindowDrag":true,"menuAnimations":true,"themes":true,
                "cursorShadow":true,"cursorBlinking":false,"desktopComposition":true,"networkHint":"wan"}});
        let config = serde_json::from_value(saved).unwrap();
        let actual = build_connector_config(&config, "", 800, 600);
        assert_eq!(
            actual.performance_flags,
            PerformanceFlags::ENABLE_FONT_SMOOTHING
                | PerformanceFlags::DISABLE_CURSORSETTINGS
                | PerformanceFlags::ENABLE_DESKTOP_COMPOSITION
        );
        assert_eq!(
            actual.connection_type,
            ironrdp_pdu::gcc::ConnectionType::Wan
        );
    }

    /// Spec rdp-native-renderer 6.3: the pointer is the local cursor. If the
    /// session composited it again, every mouse move would be a frame and,
    /// on Linux, a full-window repaint.
    #[test]
    fn the_remote_pointer_is_never_composited_into_frames() {
        let config: ConnectionConfig = serde_json::from_value(serde_json::json!({
            "id": "pointer", "name": "Pointer", "host": "pointer.invalid", "port": 3389,
            "createdAt": "2026-11-21T00:00:00Z", "updatedAt": "2026-11-21T00:00:00Z"
        }))
        .unwrap();
        let built = build_connector_config(&config, "test", 800, 600);
        assert!(
            built.enable_server_pointer,
            "the server's pointer shapes are wanted"
        );
        assert!(
            !built.pointer_software_rendering,
            "but not drawn into the frames"
        );
    }

    /// Spec rdp-keyboard-layout 1.1: the connection reports the layout the
    /// session resolved; Automatic is resolved before this point.
    #[test]
    fn the_resolved_keyboard_layout_is_reported() {
        use crate::store::connections::{KeyboardLayoutChoice, KeyboardLayoutName};
        let mut config: ConnectionConfig = serde_json::from_value(serde_json::json!({
            "id": "kb", "name": "Kb", "host": "kb.invalid", "port": 3389,
            "createdAt": "2026-11-21T00:00:00Z", "updatedAt": "2026-11-21T00:00:00Z"
        }))
        .unwrap();
        for (choice, reported) in [
            (KeyboardLayoutChoice::Fixed(0x407), 0x407),
            (
                KeyboardLayoutChoice::Named(KeyboardLayoutName::ServerDefault),
                0,
            ),
            (KeyboardLayoutChoice::AUTOMATIC, 0),
        ] {
            config.host_profile.keyboard_layout = choice;
            assert_eq!(
                build_connector_config(&config, "", 800, 600).keyboard_layout,
                reported
            );
        }
    }

    #[test]
    fn host_profile_each_preference_changes_only_its_protocol_bit() {
        use ironrdp_pdu::rdp::client_info::PerformanceFlags as F;
        let base = serde_json::json!({"id":"profile", "name":"Profile", "host":"profile.invalid", "port":3389,
            "createdAt":"2026-11-17T00:00:00Z", "updatedAt":"2026-11-17T00:00:00Z"});
        for (field, flag, default, positive) in [
            ("wallpaper", F::DISABLE_WALLPAPER, false, false),
            ("fullWindowDrag", F::DISABLE_FULLWINDOWDRAG, true, false),
            ("menuAnimations", F::DISABLE_MENUANIMATIONS, false, false),
            ("themes", F::DISABLE_THEMING, false, false),
            ("cursorShadow", F::DISABLE_CURSOR_SHADOW, false, false),
            ("cursorBlinking", F::DISABLE_CURSORSETTINGS, true, false),
            (
                "desktopComposition",
                F::ENABLE_DESKTOP_COMPOSITION,
                false,
                true,
            ),
        ] {
            let original: ConnectionConfig = serde_json::from_value(base.clone()).unwrap();
            let before = build_connector_config(&original, "test", 800, 600);
            assert_eq!(
                before.performance_flags.contains(flag),
                default == positive,
                "{field} default"
            );
            let mut changed = base.clone();
            changed["hostProfile"] = serde_json::json!({field: !default});
            let after =
                build_connector_config(&serde_json::from_value(changed).unwrap(), "test", 800, 600);
            assert_eq!(
                before.performance_flags ^ after.performance_flags,
                flag,
                "{field}"
            );
            assert_eq!(
                before.support_dyn_vc_gfx_protocol,
                after.support_dyn_vc_gfx_protocol
            );
            assert!(
                after.enable_tls && after.enable_credssp && !after.enable_standard_rdp_security
            );
        }
        use ironrdp_pdu::gcc::ConnectionType as C;
        for (hint, expected) in [
            ("lan", C::Lan),
            ("wan", C::Wan),
            ("broadbandHigh", C::BroadbandHigh),
            ("broadbandLow", C::BroadbandLow),
            ("satellite", C::Satellite),
            ("modem", C::Modem),
        ] {
            let mut value = base.clone();
            value["hostProfile"] = serde_json::json!({"networkHint":hint});
            let config =
                build_connector_config(&serde_json::from_value(value).unwrap(), "test", 800, 600);
            assert_eq!(config.connection_type, expected);
        }
    }

    #[test]
    fn connector_config_consumes_codec_policy() {
        use crate::rdp::codec_policy::CodecPolicy;

        let config = crate::store::connections::ConnectionConfig {
            id: "drift-task-9-1-codec-policy".to_string(),
            name: "codec policy consumption".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3389,
            username: "user".to_string(),
            domain: String::new(),
            color_accent: None,
            tags: Vec::new(),
            display_width: None,
            display_height: None,
            font_smoothing: true,

            host_profile: Default::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_connected_at: None,
            mock: false,
            library_id: String::new(),
            folder_id: None,
        };

        let connector = build_connector_config(&config, "pw", 1280, 720);

        assert!(
            connector.support_dyn_vc_gfx_protocol == CodecPolicy::support_dyn_vc_gfx_protocol()
                && connector.support_dyn_vc_gfx_protocol,
            "the GFX early-capability switch must come from CodecPolicy and be ON (Req 9.1)"
        );

        let fallback = CodecPolicy::fastpath_bitmap();
        let bitmap = connector
            .bitmap
            .as_ref()
            .expect("fast-path bitmap fallback config must be embedded in the connector config");
        assert_eq!(
            bitmap.color_depth, fallback.color_depth,
            "fallback color depth must come from CodecPolicy (Req 9.3)"
        );
        assert_eq!(
            bitmap.lossy_compression, fallback.lossy_compression,
            "fallback lossy-compression switch must come from CodecPolicy"
        );
        assert_eq!(
            bitmap.codecs.0.len(),
            fallback.codecs.0.len(),
            "fallback codec advertisement must come from CodecPolicy"
        );
        assert!(
            bitmap.codecs.0.is_empty(),
            "no-EGFX fallback must advertise no bitmap codec: standard/uncompressed bitmap updates only (Req 9.3)"
        );
    }
}
