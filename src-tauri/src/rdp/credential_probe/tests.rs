use super::*;
use crate::rdp::peer_identity::{ConnectionTarget, ObservedPeer};
use ironrdp_pdu::nego::{ConnectionConfirm, ConnectionRequest, ResponseFlags, SecurityProtocol};
use ironrdp_pdu::x224::X224;
use ironrdp_tokio::{FramedWrite as _, TokioFramed};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use sha2::{Digest as _, Sha256};
use sspi::credssp::{
    CredSspServer, CredentialsProxy, EarlyUserAuthResult, ServerMode, ServerState, TsRequest,
};
use sspi::{AuthIdentity, Username};
use std::{io, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use x509_cert::der::{Decode as _, Encode as _};

const CERT: &[u8] = include_bytes!("../peer_identity/fixtures/self-signed.der");
const KEY: &[u8] = include_bytes!("../peer_identity/fixtures/self-signed-key.der");
const PASSWORD: &str = "fixture-only-sentinel";

async fn fixture() -> (TcpListener, ConnectionConfig, ConfirmedPin) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let host = serde_json::from_value(serde_json::json!({
        "id": "fixture", "name": "Selected host", "host": "127.0.0.1", "port": port,
        "username": "operator", "domain": "FIXTURE",
        "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    let cert = x509_cert::Certificate::from_der(CERT).unwrap();
    let spki = cert
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .unwrap();
    let pin = ConfirmedPin::from_operator_confirmation(ObservedPeer {
        endpoint: ConnectionTarget::new("127.0.0.1", port).unwrap().endpoint,
        spki_sha256: Sha256::digest(spki).into(),
    });
    (listener, host, pin)
}

async fn negotiate(listener: &TcpListener, selected: SecurityProtocol) -> TcpStream {
    let (socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut stream = TokioFramed::new(socket);
    let (_, bytes) = stream.read_pdu().await.unwrap();
    let X224(request) = ironrdp_core::decode::<X224<ConnectionRequest>>(&bytes).unwrap();
    assert_eq!(
        request.protocol,
        SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX
    );
    assert_eq!(
        request.nego_data, None,
        "saved identity was sent before TLS verification"
    );
    let reply = X224(ConnectionConfirm::Response {
        flags: ResponseFlags::empty(),
        protocol: selected,
    });
    stream
        .write_all(&ironrdp_core::encode_vec(&reply).unwrap())
        .await
        .unwrap();
    stream.into_inner_no_leftover()
}

async fn encrypted(stream: TcpStream) -> TlsStream<TcpStream> {
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(CERT)],
        PrivatePkcs8KeyDer::from(KEY).into(),
    )
    .unwrap();
    TlsAcceptor::from(Arc::new(config))
        .accept(stream)
        .await
        .unwrap()
}

async fn assert_closed(stream: &mut TlsStream<TcpStream>) {
    match stream.read(&mut [0; 1]).await {
        Ok(0) => {}
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
            ) => {}
        result => panic!("expected the probe to close without further protocol bytes: {result:?}"),
    }
}

fn assert_one_connection(listener: TcpListener) {
    assert_eq!(
        listener.into_std().unwrap().accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

struct FixtureIdentity(AuthIdentity);

impl CredentialsProxy for FixtureIdentity {
    type AuthenticationData = AuthIdentity;
    fn auth_data_by_user(&mut self, username: &Username) -> io::Result<AuthIdentity> {
        assert_eq!(username, &self.0.username);
        Ok(self.0.clone())
    }
    fn auth_data(&mut self) -> io::Result<Vec<AuthIdentity>> {
        Ok(vec![self.0.clone()])
    }
}

async fn read_request(stream: &mut TlsStream<TcpStream>) -> TsRequest {
    let mut bytes = Vec::new();
    let len = loop {
        bytes.push(stream.read_u8().await.unwrap());
        if let Ok(len) = TsRequest::read_length(bytes.as_slice()) {
            break len;
        }
        assert!(bytes.len() < 5, "not a CredSSP frame");
    };
    let header_len = bytes.len();
    bytes.resize(len, 0);
    stream.read_exact(&mut bytes[header_len..]).await.unwrap();
    TsRequest::from_buffer(&bytes).unwrap()
}

#[derive(Clone, Copy)]
enum Reply {
    NoConfirmation,
    Authorization(EarlyUserAuthResult),
    Status { after_proof: bool },
}

async fn authentication_server(listener: &TcpListener, reply: Reply) {
    let protocol = match reply {
        Reply::NoConfirmation => SecurityProtocol::HYBRID,
        _ => SecurityProtocol::HYBRID_EX,
    };
    let mut stream = encrypted(negotiate(listener, protocol).await).await;
    let key = x509_cert::Certificate::from_der(CERT).unwrap();
    let identity = AuthIdentity {
        username: Username::new("operator", Some("FIXTURE")).unwrap(),
        password: PASSWORD.to_owned().into(),
    };
    let mut server = CredSspServer::new(
        ironrdp_tls::extract_tls_server_public_key(&key)
            .unwrap()
            .to_vec(),
        FixtureIdentity(identity.clone()),
        ServerMode::Ntlm(Default::default()),
    )
    .unwrap();
    let mut requests = 0;
    loop {
        let request = read_request(&mut stream).await;
        requests += 1;
        if let Reply::Status { after_proof } = reply
            && request.pub_key_auth.is_some() == after_proof
        {
            assert_eq!(requests, if after_proof { 2 } else { 1 });
            let response = TsRequest {
                error_code: Some(NStatusCode::LOGON_FAILURE),
                ..Default::default()
            };
            let mut bytes = Vec::new();
            response.encode_ts_request(&mut bytes).unwrap();
            stream.write_all(&bytes[..1]).await.unwrap();
            stream.write_all(&bytes[1..]).await.unwrap();
            assert_closed(&mut stream).await;
            return;
        }
        match server.process(request).resolve_to_result().unwrap() {
            ServerState::ReplyNeeded(reply) => {
                let mut bytes = Vec::new();
                reply.encode_ts_request(&mut bytes).unwrap();
                stream.write_all(&bytes[..1]).await.unwrap();
                stream.write_all(&bytes[1..]).await.unwrap();
            }
            ServerState::Finished(received) => {
                assert!(
                    received == identity,
                    "saved identity and credential must match"
                );
                break;
            }
        }
    }
    assert_eq!(requests, 3, "one NTLM exchange");
    if let Reply::Authorization(result) = reply {
        let mut bytes = Vec::new();
        result.to_buffer(&mut bytes).unwrap();
        stream.write_all(&bytes).await.unwrap();
    }
    assert_closed(&mut stream).await;
}

#[tokio::test]
async fn same_socket_saved_credential_stops_before_desktop() {
    for (reply, expected) in [
        (
            Reply::Authorization(EarlyUserAuthResult::Success),
            Evidence::EarlyAuthorizationGranted,
        ),
        (
            Reply::Authorization(EarlyUserAuthResult::AccessDenied),
            Evidence::EarlyAuthorizationDenied,
        ),
        (Reply::NoConfirmation, Evidence::NoServerConfirmation),
        (
            Reply::Status { after_proof: false },
            Evidence::ServerStatus {
                code: NStatusCode::LOGON_FAILURE,
                credential_proof_sent: false,
            },
        ),
        (
            Reply::Status { after_proof: true },
            Evidence::ServerStatus {
                code: NStatusCode::LOGON_FAILURE,
                credential_proof_sent: true,
            },
        ),
    ] {
        let (listener, host, pin) = fixture().await;
        let responder = tokio::spawn(async move {
            authentication_server(&listener, reply).await;
            listener
        });
        assert_eq!(
            probe_saved_host(&host, PASSWORD, Some(&pin)).await,
            Ok(expected)
        );
        assert_one_connection(responder.await.unwrap());
    }
}

#[tokio::test]
async fn unknown_or_changed_identity_receives_no_authentication() {
    for changed in [false, true] {
        let (listener, host, _) = fixture().await;
        let wrong = ConfirmedPin::from_operator_confirmation(ObservedPeer {
            endpoint: ConnectionTarget::new(&host.host, host.port)
                .unwrap()
                .endpoint,
            spki_sha256: [0; 32],
        });
        let responder = tokio::spawn(async move {
            let mut stream =
                encrypted(negotiate(&listener, SecurityProtocol::HYBRID_EX).await).await;
            assert_closed(&mut stream).await;
            listener
        });
        assert_eq!(
            probe_saved_host(&host, PASSWORD, changed.then_some(&wrong)).await,
            Err(ProbeError::UntrustedPeer)
        );
        assert_one_connection(responder.await.unwrap());
    }
}

#[tokio::test]
async fn tls_only_selection_ends_without_upgrade_or_retry() {
    let (listener, host, pin) = fixture().await;
    let responder = tokio::spawn(async move {
        let mut stream = negotiate(&listener, SecurityProtocol::SSL).await;
        assert_eq!(stream.read(&mut [0; 1]).await.unwrap(), 0);
        listener
    });
    assert_eq!(
        probe_saved_host(&host, PASSWORD, Some(&pin)).await,
        Err(ProbeError::Negotiation)
    );
    assert_one_connection(responder.await.unwrap());
}

#[tokio::test]
async fn unsupported_frame_size_is_rejected_from_its_header() {
    use x509_cert::der::{Header, Length, Tag};

    // Its body fits in u16 but its complete frame does not. No body is supplied.
    let header = Header::new(Tag::Sequence, Length::from(u16::MAX))
        .to_der()
        .unwrap();
    assert_eq!(
        read_ts_request(&mut header.as_slice()).await,
        Err(ProbeError::MalformedResponse)
    );
}

#[tokio::test]
async fn unexpected_response_closes_while_the_peer_is_still_open() {
    let (listener, host, pin) = fixture().await;
    let responder = tokio::spawn(async move {
        let mut stream = encrypted(negotiate(&listener, SecurityProtocol::HYBRID_EX).await).await;
        read_request(&mut stream).await;
        // A valid early authorization PDU is out of order during the token exchange.
        let mut reply = Vec::new();
        EarlyUserAuthResult::Success.to_buffer(&mut reply).unwrap();
        stream.write_all(&reply).await.unwrap();
        assert_closed(&mut stream).await;
        listener
    });
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        probe_saved_host(&host, PASSWORD, Some(&pin)),
    )
    .await
    .expect("invalid framing must finish without waiting for EOF");
    assert_eq!(result, Err(ProbeError::MalformedResponse));
    let listener = tokio::time::timeout(Duration::from_secs(3), responder)
        .await
        .expect("the peer must observe the probe closing its socket")
        .unwrap();
    assert_one_connection(listener);
}

#[tokio::test]
async fn cancelling_pending_authentication_drops_the_only_socket() {
    let (listener, host, pin) = fixture().await;
    let (received, ready) = tokio::sync::oneshot::channel();
    let responder = tokio::spawn(async move {
        let mut stream = encrypted(negotiate(&listener, SecurityProtocol::HYBRID_EX).await).await;
        read_request(&mut stream).await;
        received.send(()).unwrap();
        assert_closed(&mut stream).await;
        listener
    });
    let probe = tokio::spawn(async move { probe_saved_host(&host, PASSWORD, Some(&pin)).await });
    tokio::time::timeout(Duration::from_secs(5), ready)
        .await
        .unwrap()
        .unwrap();
    probe.abort();
    assert!(probe.await.unwrap_err().is_cancelled());
    assert_one_connection(responder.await.unwrap());
}
