use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

use ironrdp_connector::{
    self as connector, ClientConnector, ConnectionResult, Credentials, ServerName,
};
use ironrdp_tokio::TokioFramed;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;

use crate::rdp::clipboard::SessionClipboardBackend;
use crate::store::connections::ConnectionConfig;

/// Commands that can be sent to a running session actor.
#[derive(Debug)]
pub enum SessionCommand {
    SendKey {
        key_code: u32,
        is_down: bool,
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
    },
    ClipboardWrite {
        text: String,
    },
    Disconnect,
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
pub type TlsFramed = TokioFramed<ironrdp_tls::TlsStream<TcpStream>>;

/// Outcome of the RDP connection attempt.
pub enum ConnectionOutcome {
    /// Real IronRDP connection established.
    RealSession {
        connection_result: Box<ConnectionResult>,
        framed: Box<TlsFramed>,
    },
    /// Connection failed; reason provided.
    Failed(String),
}

/// Attempt a TCP connection to the RDP server.
pub async fn tcp_connect(host: &str, port: u16) -> Result<TcpStream, RdpError> {
    let addr = format!("{}:{}", host, port);
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        TcpStream::connect(&addr),
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

    let performance_flags = PerformanceFlags::DISABLE_WALLPAPER
        | PerformanceFlags::DISABLE_FULLWINDOWDRAG
        | PerformanceFlags::DISABLE_MENUANIMATIONS
        | PerformanceFlags::DISABLE_THEMING
        | PerformanceFlags::DISABLE_CURSOR_SHADOW;

    // connector 0.12's Config has no `Default` — all fields are pub and the
    // literal is constructed explicitly (task 4.1 probe C8a shape). Values for
    // fields that the old 0.8 connector hardcoded internally are preserved:
    // `connection_type: Lan` and empty `alternate_shell`/`work_dir` were 0.8
    // constants; `enable_standard_rdp_security: false` matches 0.8, which
    // rejected standard RDP security at negotiation. New 0.12 switches stay in
    // their off position: `support_dyn_vc_gfx_protocol` waits for the EGFX
    // graphics pipeline (group 9), `multitransport_flags`/`compression_type`
    // off, `remote_application_mode` off.
    connector::Config {
        credentials: Credentials::UsernamePassword {
            username: config.username.clone(),
            password: password.to_string(),
        },
        domain,
        enable_tls: true,
        enable_credssp: true,
        enable_standard_rdp_security: false,
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        connection_type: ConnectionType::Lan,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize {
            width: width as u16,
            height: height as u16,
        },
        desktop_scale_factor: 0,
        monitor_layout: None,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        bitmap: None,
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
        autologon: false,
        enable_audio_playback: false,
        enable_audio_capture: false,
        pointer_software_rendering: true,
        performance_flags,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        compression_type: None,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: false,
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
/// 3. TLS upgrade via ironrdp_tls::upgrade (preserving leftover bytes)
/// 4. connect_finalize (CredSSP/NLA + MCS/licensing/capabilities)
pub async fn attempt_rdp_connection(
    config: &ConnectionConfig,
    password: Option<&str>,
    width: u32,
    height: u32,
) -> ConnectionOutcome {
    let password = password.unwrap_or_default();

    // Step 1: TCP connect
    let tcp_stream = match tcp_connect(&config.host, config.port).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("TCP connection failed: {}", e);
            return ConnectionOutcome::Failed(e.0);
        }
    };

    // Step 2: Create ClientConnector and begin negotiation
    let connector_config = build_connector_config(config, password, width, height);
    let client_addr = tcp_stream
        .local_addr()
        .unwrap_or_else(|_| std::net::SocketAddr::from(([0, 0, 0, 0], 0)));
    let mut connector = ClientConnector::new(connector_config, client_addr);
    connector.attach_static_channel(ironrdp_cliprdr::CliprdrClient::new(Box::new(
        SessionClipboardBackend::new(),
    )));

    let mut framed = TokioFramed::new(tcp_stream);

    let should_upgrade = match ironrdp_tokio::connect_begin(&mut framed, &mut connector).await {
        Ok(su) => su,
        Err(e) => {
            log::warn!("RDP negotiation begin failed: {:?}", e);
            return ConnectionOutcome::Failed(format!("RDP negotiation begin failed: {}", e));
        }
    };

    log::info!("RDP X.224 negotiation complete, upgrading to TLS...");

    // Step 3: TLS upgrade — following the official IronRDP client pattern exactly:
    // 1) Extract stream AND leftover bytes from framed
    // 2) Pass stream to ironrdp_tls::upgrade
    // 3) Reconstruct framed with leftover bytes preserved
    let (initial_stream, leftover_bytes) = framed.into_inner();
    log::info!(
        "Starting TLS handshake (leftover={} bytes, server_name='{}')...",
        leftover_bytes.len(),
        config.host
    );

    // Use a dummy DNS name for TLS SNI when connecting via IP address,
    // since rustls ServerName::try_from may fail or cause issues with raw IPs.
    // Certificate verification is disabled anyway (NoCertificateVerification).
    let tls_server_name = if config.host.parse::<std::net::IpAddr>().is_ok() {
        "rdp-server"
    } else {
        &config.host
    };
    log::info!(
        "Using TLS server name: '{}' (original host: '{}')",
        tls_server_name,
        config.host
    );

    // Run TLS in a separate spawned task so timeout can actually cancel it.
    // The TLS handshake can block the executor, preventing tokio::time::timeout from working.
    let tls_name = tls_server_name.to_string();
    let tls_handle = tokio::spawn(async move { do_tls_upgrade(initial_stream, &tls_name).await });

    let tls_result = tokio::time::timeout(std::time::Duration::from_secs(15), tls_handle).await;

    let (tls_stream, tls_cert) = match tls_result {
        Ok(Ok(Ok(result))) => {
            log::info!("TLS handshake completed successfully");
            result
        }
        Ok(Ok(Err(e))) => {
            log::warn!("TLS upgrade failed: {}", e);
            return ConnectionOutcome::Failed(format!("TLS upgrade failed: {}", e));
        }
        Ok(Err(e)) => {
            log::warn!("TLS task panicked: {}", e);
            return ConnectionOutcome::Failed(format!("TLS task panicked: {}", e));
        }
        Err(_) => {
            log::warn!("TLS upgrade timed out after 15 seconds");
            return ConnectionOutcome::Failed("TLS upgrade timed out".to_string());
        }
    };

    // Extract server public key from certificate (needed for CredSSP)
    let server_public_key = match ironrdp_tls::extract_tls_server_public_key(&tls_cert) {
        Some(key) => key.to_vec(),
        None => {
            log::warn!("Failed to extract server public key from TLS certificate");
            return ConnectionOutcome::Failed(
                "Failed to extract server public key from TLS certificate".to_string(),
            );
        }
    };

    log::info!(
        "Server public key extracted ({} bytes)",
        server_public_key.len()
    );

    // Step 4: Mark as upgraded, create type-erased framed with leftover bytes
    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);

    // Create framed over TLS stream, preserving any leftover bytes
    let mut tls_framed = TokioFramed::new_with_leftover(tls_stream, leftover_bytes);

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

    ConnectionOutcome::RealSession {
        connection_result: Box::new(connection_result),
        framed: Box::new(tls_framed),
    }
}

/// Manual TLS upgrade with step-by-step logging to diagnose where it hangs.
async fn do_tls_upgrade<S>(
    stream: S,
    server_name: &str,
) -> std::io::Result<(ironrdp_tls::TlsStream<S>, x509_cert::Certificate)>
where
    S: Unpin + tokio::io::AsyncRead + tokio::io::AsyncWrite,
{
    use tokio::io::AsyncWriteExt;
    use tokio_rustls::rustls;
    use tokio_rustls::rustls::pki_types::ServerName as RustlsServerName;

    log::info!("[TLS] Step 1: Building rustls config...");
    // The ring CryptoProvider is pinned on this ClientConfig instead of
    // relying on a process-wide install (B3-15): the connector path is
    // standalone-usable headless — the Tauri entrypoint's install_default is
    // neither required nor disturbed by this (same provider either way).
    let mut config = rustls::client::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring CryptoProvider with safe default protocol versions")
    .dangerous()
    .with_custom_certificate_verifier(std::sync::Arc::new(NoCertVerifier))
    .with_no_client_auth();
    config.key_log = std::sync::Arc::new(rustls::KeyLogFile::new());
    config.resumption = rustls::client::Resumption::disabled();
    let config = std::sync::Arc::new(config);

    log::info!("[TLS] Step 2: Parsing server name '{}'...", server_name);
    let domain =
        RustlsServerName::try_from(server_name.to_owned()).map_err(std::io::Error::other)?;
    log::info!("[TLS] Step 3: ServerName parsed OK: {:?}", domain);

    log::info!("[TLS] Step 4: Starting TLS handshake (TlsConnector::connect)...");
    let mut tls_stream = tokio_rustls::TlsConnector::from(config)
        .connect(domain, stream)
        .await?;
    log::info!("[TLS] Step 5: TLS handshake COMPLETED! Flushing...");

    tls_stream.flush().await?;
    log::info!("[TLS] Step 6: Flush done. Extracting certificate...");

    let tls_cert = {
        use x509_cert::der::Decode as _;
        let cert = tls_stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certs| certs.first())
            .ok_or_else(|| std::io::Error::other("peer certificate is missing"))?;
        x509_cert::Certificate::from_der(cert).map_err(std::io::Error::other)?
    };
    log::info!("[TLS] Step 7: Certificate extracted OK");

    Ok((tls_stream, tls_cert))
}

/// NoCertificateVerification for RDP self-signed certs.
#[derive(Debug)]
struct NoCertVerifier;

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        use tokio_rustls::rustls::SignatureScheme::*;
        vec![
            RSA_PKCS1_SHA1,
            ECDSA_SHA1_Legacy,
            RSA_PKCS1_SHA256,
            ECDSA_NISTP256_SHA256,
            RSA_PKCS1_SHA384,
            ECDSA_NISTP384_SHA384,
            RSA_PKCS1_SHA512,
            ECDSA_NISTP521_SHA512,
            RSA_PSS_SHA256,
            RSA_PSS_SHA384,
            RSA_PSS_SHA512,
            ED25519,
            ED448,
        ]
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
}
