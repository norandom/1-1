use std::sync::Arc;

use super::*;
use rustls::crypto::ring;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::sign::{CertifiedKey, SingleCertAndKey};
use rustls::{ServerConfig, SupportedProtocolVersion, version};
use tokio::io::duplex;
use tokio::io::{AsyncReadExt as _, DuplexStream};
use tokio_rustls::TlsAcceptor;

const SERVER: &[u8] = include_bytes!("fixtures/server.der");
const SERVER_KEY: &[u8] = include_bytes!("fixtures/server-key.der");
const OTHER_KEY: &[u8] = include_bytes!("fixtures/self-signed-key.der");
const SELF_SIGNED: &[u8] = include_bytes!("fixtures/self-signed.der");
const CA: &[u8] = include_bytes!("fixtures/ca.der");

fn endpoint(host: &str, port: u16) -> Endpoint {
    ConnectionTarget::new(host, port).unwrap().endpoint
}

#[test]
fn transport_target_preserves_absolute_and_relative_dns_names() {
    for (configured_host, dial_host) in [
        ("build.", "build."),
        ("build", "build"),
        ("[2001:db8::10]", "2001:db8::10"),
        ("2001:db8::10", "2001:db8::10"),
    ] {
        let target = ConnectionTarget::new(configured_host, 3391).unwrap();
        assert_eq!(target.socket_target(), (dial_host, 3391));
    }
    assert_eq!(endpoint("build.", 3391), endpoint("build", 3391));
}

fn fixture_roots() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(CA)).unwrap();
    roots
}

async fn fixture(
    endpoint: &Endpoint,
    cert: &'static [u8],
    key: &'static [u8],
    version: &'static SupportedProtocolVersion,
) -> io::Result<SignedTls<DuplexStream>> {
    let (client_io, server_io) = duplex(65536);
    let acceptor = server(version, cert, key);
    let (client, _) = tokio::join!(handshake(client_io, endpoint), acceptor.accept(server_io));
    client
}

fn server(
    version: &'static SupportedProtocolVersion,
    cert: &'static [u8],
    key: &'static [u8],
) -> TlsAcceptor {
    let provider = Arc::new(ring::default_provider());
    let key = provider
        .key_provider
        .load_private_key(PrivatePkcs8KeyDer::from(key).into())
        .unwrap();
    // Choosing the signing key independently lets the fixture exercise a real
    // handshake whose CertificateVerify signature disagrees with the leaf.
    let resolver = SingleCertAndKey::from(CertifiedKey::new(vec![CertificateDer::from(cert)], key));
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[version])
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(resolver));
    config.send_tls13_tickets = 0;
    TlsAcceptor::from(Arc::new(config))
}

#[tokio::test]
async fn rejects_bad_server_signature_on_tls12_and_tls13() {
    for version in [&version::TLS12, &version::TLS13] {
        let client = fixture(
            &endpoint("desktop.example.test", 3389),
            SERVER,
            OTHER_KEY,
            version,
        )
        .await;
        assert!(
            client.is_err(),
            "accepted an invalid {:?} server signature",
            version.version
        );
    }
}

#[tokio::test]
async fn trusted_chain_checks_dns_and_ip_sans_and_returns_der_spki() {
    const HASH: [u8; 32] = [
        0xbc, 0x1b, 0x18, 0xe4, 0xfc, 0xc7, 0xcf, 0xb4, 0xc2, 0xbf, 0xf5, 0x23, 0xb3, 0x8f, 0x33,
        0x2e, 0x4c, 0xeb, 0xcd, 0x31, 0x17, 0x87, 0x63, 0xf8, 0xb9, 0x00, 0xee, 0x35, 0x91, 0xc7,
        0xe8, 0x06,
    ];
    for host in ["Desktop.Example.Test.", "192.0.2.10", "[2001:db8::10]"] {
        let destination = endpoint(host, 3391);
        let tls = fixture(&destination, SERVER, SERVER_KEY, &version::TLS12)
            .await
            .unwrap();
        let verified = tls.verify_chain(&fixture_roots()).unwrap();
        assert_eq!(verified.identity().spki_sha256, HASH);
        assert_eq!(verified.identity().endpoint, destination);
    }
    assert_eq!(
        endpoint("DESKTOP.Example.Test.", 3389),
        endpoint("desktop.example.test", 3389)
    );
    assert_eq!(
        endpoint("[2001:db8:0::10]", 3389),
        endpoint("2001:db8::10", 3389)
    );
    assert!(ConnectionTarget::new("[desktop.example.test]", 3389).is_err());
    assert!(ConnectionTarget::new("desktop.example.test..", 3389).is_err());
    for host in ["wrong.example.test", "192.0.2.11", "2001:db8::11"] {
        let tls = fixture(&endpoint(host, 3389), SERVER, SERVER_KEY, &version::TLS13)
            .await
            .unwrap();
        assert!(tls.verify_chain(&fixture_roots()).is_err());
    }
}

#[test]
fn upgrade_rejects_surplus_plaintext_after_x224() {
    let (client_io, _server_io) = duplex(1024);
    let framed = TokioFramed::new_with_leftover(client_io, (&b"extra"[..]).into());
    assert_eq!(
        take_upgrade_stream(framed).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[tokio::test]
async fn confirmed_pin_requires_same_host_port_and_signed_key_on_each_socket() {
    let destination = endpoint("desktop.example.test", 3389);
    let first = fixture(&destination, SELF_SIGNED, OTHER_KEY, &version::TLS13)
        .await
        .unwrap();
    assert!(first.verify_chain(&fixture_roots()).is_err());
    let pin = ConfirmedPin::from_operator_confirmation(first.observed);
    let next = fixture(&destination, SELF_SIGNED, OTHER_KEY, &version::TLS12)
        .await
        .unwrap();
    assert!(next.verify(Some(&pin)).await.is_ok());

    for other in [
        endpoint("other.example.test", 3389),
        endpoint("desktop.example.test", 3390),
    ] {
        let tls = fixture(&other, SELF_SIGNED, OTHER_KEY, &version::TLS13)
            .await
            .unwrap();
        assert!(tls.verify(Some(&pin)).await.is_err());
    }
    let rotated = fixture(&destination, SERVER, SERVER_KEY, &version::TLS13)
        .await
        .unwrap();
    assert!(rotated.verify(Some(&pin)).await.is_err());
}

#[tokio::test]
async fn discovery_negotiates_without_identity_then_closes_without_application_data() {
    let (client_io, server_io) = duplex(65536);
    let destination = endpoint("desktop.example.test", 3389);
    let responder = respond_to_review(server_io);
    let (discovered, ()) =
        tokio::join!(discover_on_stream(client_io, &destination, None), responder);
    let candidate: ObservedPeer = discovered.unwrap();
    assert_eq!(candidate.endpoint, destination);
    let next = fixture(&destination, SELF_SIGNED, OTHER_KEY, &version::TLS13)
        .await
        .unwrap();
    assert!(next.verify_chain(&fixture_roots()).is_err());
}

#[tokio::test]
async fn trusted_discovery_needs_a_valid_chain_or_the_confirmed_pin() {
    async fn observe(
        destination: &Endpoint,
        cert: &'static [u8],
        key: &'static [u8],
    ) -> ConfirmedPin {
        let tls = fixture(destination, cert, key, &version::TLS13)
            .await
            .unwrap();
        ConfirmedPin::from_operator_confirmation(tls.observed)
    }
    let destination = endpoint("desktop.example.test", 3389);
    let pinned = observe(&destination, SELF_SIGNED, OTHER_KEY).await;
    let rotated = observe(&destination, SERVER, SERVER_KEY).await;
    for (pin, expected) in [
        (None, Err(io::ErrorKind::Other)),
        (Some(&pinned), Ok(())),
        (Some(&rotated), Err(io::ErrorKind::PermissionDenied)),
    ] {
        let (client_io, server_io) = duplex(65536);
        let (result, ()) = tokio::join!(
            discover_on_stream(client_io, &destination, Some(pin)),
            respond_to_review(server_io)
        );
        // The self-signed responder is trusted only through its own pin.
        assert_eq!(result.map(|_| ()).map_err(|e| e.kind()), expected);
    }
}

async fn respond_to_review<S: AsyncRead + AsyncWrite + Unpin + Send + Sync>(server_io: S) {
    use ironrdp_pdu::nego::ResponseFlags;
    let mut server_framed = TokioFramed::new(server_io);
    let (_, request) = server_framed.read_pdu().await.unwrap();
    let X224(req) = ironrdp_core::decode::<X224<ConnectionRequest>>(&request).unwrap();
    assert_eq!(req.nego_data, None);
    let (mut stream, leftover) = server_framed.into_inner();
    assert!(leftover.is_empty());
    let response = X224(ConnectionConfirm::Response {
        flags: ResponseFlags::empty(),
        protocol: SecurityProtocol::HYBRID,
    });
    stream
        .write_all(&ironrdp_core::encode_vec(&response).unwrap())
        .await
        .unwrap();
    let mut encrypted = server(&version::TLS13, SELF_SIGNED, OTHER_KEY)
        .accept(stream)
        .await
        .unwrap();
    let mut buffer = [0u8; 1];
    assert_eq!(encrypted.read(&mut buffer).await.unwrap(), 0);
}

#[tokio::test]
async fn review_confirms_self_signed_key_for_a_fresh_socket_and_rejects_rotation() {
    use super::pins::PinStore;
    use crate::store::library::LibraryStore;

    let dir = tempfile::tempdir().unwrap();
    let library = LibraryStore::open(dir.path().join("connections.json"));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let connection = library
        .create_connection(
            serde_json::from_value(serde_json::json!({
                "name": "Synthetic host", "host": "127.0.0.1", "port": port,
            }))
            .unwrap(),
        )
        .unwrap();
    let path = dir.path().join("server-pins.json");
    let pins = PinStore::new(path.clone());
    let (review, ()) = tokio::join!(pins.review(&library, &connection.id), async {
        respond_to_review(listener.accept().await.unwrap().0).await
    });
    let review = review.unwrap();
    assert_eq!(review.endpoint, format!("127.0.0.1:{port}"));
    assert!(review.previous_fingerprint.is_none());
    let destination = endpoint("127.0.0.1", port);
    assert!(pins.lookup(&destination).unwrap().is_none());
    pins.confirm(&library, review.selection).unwrap();
    assert!(pins.confirm(&library, review.selection).is_err());
    let reopened = PinStore::new(path);
    let pin = reopened.lookup(&destination).unwrap().unwrap();
    let next = fixture(&destination, SELF_SIGNED, OTHER_KEY, &version::TLS13)
        .await
        .unwrap();
    next.verify(Some(&pin)).await.unwrap();
    let rotated = fixture(&destination, SERVER, SERVER_KEY, &version::TLS13)
        .await
        .unwrap();
    assert!(rotated.verify(Some(&pin)).await.is_err());
    assert!(
        reopened
            .lookup(&endpoint("another.test", port))
            .unwrap()
            .is_none()
    );
    assert!(
        reopened
            .lookup(&endpoint("127.0.0.1", port - 1))
            .unwrap()
            .is_none()
    );
    let profile = std::fs::read_to_string(dir.path().join("connections.json")).unwrap();
    assert!(!profile.contains(&review.observed_fingerprint));
    assert!(!profile.contains("spki"));
}
