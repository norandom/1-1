//! One saved-host authentication check. Its caller owns the snapshot and deadline.

use super::client::build_connector_config;
use super::peer_identity::{self, ConfirmedPin, ConnectionTarget};
use crate::store::connections::ConnectionConfig;
use ironrdp_connector::credssp::CredsspSequence;
use ironrdp_connector::{
    ClientConnector, ClientConnectorState, ConnectorErrorKind, Credentials, ServerName,
};
use ironrdp_core::WriteBuf;
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_tokio::TokioFramed;
use sspi::credssp::{ClientState, EARLY_USER_AUTH_RESULT_PDU_SIZE, NStatusCode, TsRequest};
use sspi::generator::GeneratorState;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

#[derive(Debug, PartialEq, Eq)]
enum Evidence {
    EarlyAuthorizationGranted,
    EarlyAuthorizationDenied,
    ServerStatus {
        code: NStatusCode,
        credential_proof_sent: bool,
    },
    NoServerConfirmation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ProbeReason {
    Timeout,
    UnknownHost,
    HostChanged,
    StoreUnavailable,
    KeyringUnavailable,
    Unreachable,
    Negotiation,
    UntrustedPeer,
    MissingIdentity,
    MissingCredential,
    AuthorizationDenied,
    NoServerConfirmation,
    ServerStatus,
    InteractiveSignInRequired,
    Authentication,
    UnsupportedAuthentication,
    MalformedResponse,
    TransportClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "outcome", content = "reason", rename_all = "camelCase")]
pub(crate) enum CredentialOutcome {
    Accepted,
    Rejected,
    UnableToVerify(ProbeReason),
}

fn classify(result: Result<Evidence, ProbeReason>) -> CredentialOutcome {
    let reason = match result {
        Ok(Evidence::EarlyAuthorizationGranted) => return CredentialOutcome::Accepted,
        Ok(Evidence::ServerStatus {
            code: NStatusCode::WRONG_PASSWORD | NStatusCode::LOGON_FAILURE,
            credential_proof_sent,
        }) => {
            if credential_proof_sent {
                return CredentialOutcome::Rejected;
            }
            ProbeReason::NoServerConfirmation
        }
        Ok(Evidence::EarlyAuthorizationDenied) => ProbeReason::AuthorizationDenied,
        Ok(Evidence::NoServerConfirmation) => ProbeReason::NoServerConfirmation,
        Ok(Evidence::ServerStatus {
            code: NStatusCode::SMARTCARD_LOGON_REQUIRED,
            ..
        }) => ProbeReason::InteractiveSignInRequired,
        Ok(Evidence::ServerStatus { .. }) => ProbeReason::ServerStatus,
        Err(reason) => reason,
    };
    CredentialOutcome::UnableToVerify(reason)
}

/// These dependencies can log decoded credential tokens at debug or trace level.
pub(crate) fn allow_diagnostic(metadata: &log::Metadata<'_>) -> bool {
    let target = metadata.target();
    !["sspi", "ironrdp_connector::credssp"].iter().any(|prefix| {
        target == *prefix
            || target
                .strip_prefix(prefix)
                .is_some_and(|tail| tail.starts_with("::"))
    })
}

/// The command supplies one saved revision and its resolved keyring value.
/// Cancellation drops this future and its socket, including during a read.
pub(crate) async fn probe_saved_host(
    host: &ConnectionConfig,
    password: &str,
    pin: Option<&ConfirmedPin>,
) -> CredentialOutcome {
    classify(collect_evidence(host, password, pin).await)
}

async fn collect_evidence(
    host: &ConnectionConfig,
    password: &str,
    pin: Option<&ConfirmedPin>,
) -> Result<Evidence, ProbeReason> {
    if password.is_empty() {
        return Err(ProbeReason::MissingCredential);
    }
    if host.username.is_empty() {
        return Err(ProbeReason::MissingIdentity);
    }
    let target =
        ConnectionTarget::new(&host.host, host.port).map_err(|_| ProbeReason::Unreachable)?;
    // TcpStream::connect((host, port)) tries subsequent addresses on failure.
    // A check gets a single dial, including when DNS returns multiple records.
    let address = tokio::net::lookup_host(target.socket_target())
        .await
        .map_err(|_| ProbeReason::Unreachable)?
        .next()
        .ok_or(ProbeReason::Unreachable)?;
    let socket = TcpStream::connect(address)
        .await
        .map_err(|_| ProbeReason::Unreachable)?;
    let local_addr = socket.local_addr().map_err(|_| ProbeReason::Unreachable)?;
    let mut framed = TokioFramed::new(socket);

    let mut config = build_connector_config(host, password, 0, 0);
    config.enable_tls = false; // Offer only TLS with NLA (HYBRID or HYBRID_EX).
    // An empty username suppresses IronRDP's plaintext routing cookie.
    let credentials = std::mem::replace(
        &mut config.credentials,
        Credentials::UsernamePassword {
            username: String::new(),
            password: String::new(),
        },
    );
    let mut connector = ClientConnector::new(config, local_addr);
    ironrdp_tokio::connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|_| ProbeReason::Negotiation)?;
    let protocol = match connector.state {
        ClientConnectorState::EnhancedSecurityUpgrade {
            selected_protocol: protocol @ (SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX),
        } => protocol,
        _ => return Err(ProbeReason::Negotiation),
    };
    let stream =
        peer_identity::take_upgrade_stream(framed).map_err(|_| ProbeReason::Negotiation)?;
    let tls = peer_identity::handshake_with_key_log(stream, &target.endpoint, None)
        .await
        .map_err(|_| ProbeReason::Negotiation)?;
    let proof = tls
        .verify(pin)
        .await
        .map_err(|_| ProbeReason::UntrustedPeer)?;
    let name = ServerName::new(proof.identity().endpoint.name.to_str().into_owned());
    let (tls_stream, public_key) = tls.into_parts();
    let (sequence, request) = CredsspSequence::init(
        credentials,
        connector.config.domain.as_deref(),
        protocol,
        name,
        public_key,
        None,
    )
    .map_err(|_| ProbeReason::Authentication)?;
    exchange(tls_stream, sequence, request).await
}

// SSPI 0.23 represents an entire TSRequest frame length as a u16. Inspect only
// its definite-length ASN.1 SEQUENCE header before reserving any body storage.
async fn read_ts_request(stream: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>, ProbeReason> {
    use x509_cert::der::{Decode as _, Header};

    let mut header = [0; 4];
    header[0] = stream
        .read_u8()
        .await
        .map_err(|_| ProbeReason::TransportClosed)?;
    if header[0] != 0x30 {
        return Err(ProbeReason::MalformedResponse);
    }
    header[1] = stream
        .read_u8()
        .await
        .map_err(|_| ProbeReason::TransportClosed)?;
    let header_len = match header[1] {
        0..=0x7f => 2,
        0x81 => 3,
        0x82 => 4,
        _ => return Err(ProbeReason::MalformedResponse),
    };
    stream
        .read_exact(&mut header[2..header_len])
        .await
        .map_err(|_| ProbeReason::TransportClosed)?;
    let prefix = &header[..header_len];
    let parsed = Header::from_der(prefix).map_err(|_| ProbeReason::MalformedResponse)?;
    let body_len = usize::try_from(parsed.length()).map_err(|_| ProbeReason::MalformedResponse)?;
    let frame_len = header_len + body_len;
    if frame_len > usize::from(u16::MAX) {
        return Err(ProbeReason::MalformedResponse);
    }
    let mut frame = vec![0; frame_len];
    frame[..header_len].copy_from_slice(prefix);
    stream
        .read_exact(&mut frame[header_len..])
        .await
        .map_err(|_| ProbeReason::TransportClosed)?;
    Ok(frame)
}

async fn exchange(
    mut stream: TlsStream<TcpStream>,
    mut sequence: CredsspSequence,
    mut request: TsRequest,
) -> Result<Evidence, ProbeReason> {
    let mut output = WriteBuf::new();
    loop {
        // This is the same explicit NTLM configuration as the desktop. A request
        // for an external provider cannot open a second connection for this check.
        let client_state = match sequence.process_ts_request(request).start() {
            GeneratorState::Completed(result) => result.map_err(|_| ProbeReason::Authentication)?,
            GeneratorState::Suspended(_) => return Err(ProbeReason::UnsupportedAuthentication),
        };
        // An error after the initial negotiation token can precede any proof
        // of the saved credential. Preserve that distinction for the UI mapper.
        let credential_proof_sent = match &client_state {
            ClientState::ReplyNeeded(message) => message.pub_key_auth.is_some(),
            ClientState::FinalMessage(_) => true,
        };
        let final_token = matches!(&client_state, ClientState::FinalMessage(_));
        output.clear();
        let written = sequence
            .handle_process_result(client_state, &mut output)
            .map_err(|_| ProbeReason::Authentication)?;
        if let Some(len) = written.size() {
            stream
                .write_all(&output[..len])
                .await
                .map_err(|_| ProbeReason::TransportClosed)?;
            stream
                .flush()
                .await
                .map_err(|_| ProbeReason::TransportClosed)?;
        }
        if sequence.next_pdu_hint().is_none() {
            return Ok(Evidence::NoServerConfirmation);
        }
        let pdu = if final_token {
            let mut authorization = vec![0; EARLY_USER_AUTH_RESULT_PDU_SIZE];
            stream
                .read_exact(&mut authorization)
                .await
                .map_err(|_| ProbeReason::TransportClosed)?;
            authorization
        } else {
            read_ts_request(&mut stream).await?
        };
        match sequence.decode_server_message(&pdu) {
            Ok(Some(message)) => {
                if let Some(status) = message.error_code
                    && status != NStatusCode::SUCCESS
                {
                    return Ok(Evidence::ServerStatus {
                        code: status,
                        credential_proof_sent,
                    });
                }
                request = message;
            }
            Ok(None) => return Ok(Evidence::EarlyAuthorizationGranted),
            Err(error) if matches!(error.kind(), ConnectorErrorKind::AccessDenied) => {
                return Ok(Evidence::EarlyAuthorizationDenied);
            }
            Err(_) => return Err(ProbeReason::MalformedResponse),
        }
    }
}

#[cfg(test)]
mod tests;
