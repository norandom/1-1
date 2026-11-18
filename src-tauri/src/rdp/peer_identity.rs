//! Same-connection TLS proof for the desktop, credential tests and folder grants.

pub(crate) mod pins;

use std::{io, net::Ipv6Addr, sync::Arc, time::Duration};

use ironrdp_pdu::nego::{ConnectionConfirm, ConnectionRequest, RequestFlags, SecurityProtocol};
use ironrdp_pdu::x224::X224;
use ironrdp_tokio::TokioFramed;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{
    WebPkiSupportedAlgorithms, ring, verify_tls12_signature, verify_tls13_signature,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_rustls::{TlsConnector, client::TlsStream};
use x509_cert::der::{Decode, Encode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub(crate) name: ServerName<'static>,
    pub(crate) port: u16,
}

/// The resolver needs the configured DNS name, including its optional root dot.
/// TLS and endpoint pins use the canonical name independently of that spelling.
pub(crate) struct ConnectionTarget<'a> {
    resolver_host: &'a str,
    pub(crate) endpoint: Endpoint,
}

impl<'a> ConnectionTarget<'a> {
    pub(crate) fn new(host: &'a str, port: u16) -> io::Result<Self> {
        let (resolver_host, name) =
            if let Some(ip) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                (ip, ip.parse::<Ipv6Addr>().map_err(io::Error::other)?.into())
            } else {
                let canonical = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
                if canonical.ends_with('.') {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "invalid DNS name",
                    ));
                }
                (
                    host,
                    ServerName::try_from(canonical).map_err(io::Error::other)?,
                )
            };
        Ok(Self {
            resolver_host,
            endpoint: Endpoint { name, port },
        })
    }

    pub(crate) fn socket_target(&self) -> (&str, u16) {
        (self.resolver_host, self.endpoint.port)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedPeer {
    pub(crate) endpoint: Endpoint,
    pub(crate) spki_sha256: [u8; 32],
}

pub(crate) struct ConfirmedPin(ObservedPeer);

impl ConfirmedPin {
    /// Used by the native confirmation/pin store, after operator authorization.
    pub(crate) fn from_operator_confirmation(peer: ObservedPeer) -> Self {
        Self(peer)
    }
}

/// This proof borrows the observed identity from a completed TLS connection.
pub(crate) struct VerifiedPeer<'a>(&'a ObservedPeer);

impl VerifiedPeer<'_> {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "used when a share or credential test requires trust"
        )
    )]
    pub(crate) fn identity(&self) -> &ObservedPeer {
        self.0
    }
}

pub(crate) struct SignedTls<S> {
    stream: TlsStream<S>,
    server_public_key: Vec<u8>,
    observed: ObservedPeer,
}

impl<S> SignedTls<S> {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "used when a share or credential test requires trust"
        )
    )]
    pub(crate) async fn verify(&self, pin: Option<&ConfirmedPin>) -> io::Result<VerifiedPeer<'_>> {
        if let Some(pin) = pin {
            if self.observed != pin.0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "server identity changed",
                ));
            }
            return Ok(VerifiedPeer(&self.observed));
        }
        let roots = tokio::task::spawn_blocking(|| {
            let mut roots = RootCertStore::empty();
            roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
            roots
        })
        .await
        .map_err(io::Error::other)?;
        self.verify_chain(&roots)
    }

    fn verify_chain(&self, roots: &RootCertStore) -> io::Result<VerifiedPeer<'_>> {
        let (leaf, intermediates) = self
            .stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certs| certs.split_first())
            .ok_or_else(|| io::Error::other("missing server certificate"))?;
        let certificate =
            rustls::server::ParsedCertificate::try_from(leaf).map_err(io::Error::other)?;
        rustls::client::verify_server_cert_signed_by_trust_anchor(
            &certificate,
            roots,
            intermediates,
            UnixTime::now(),
            ring::default_provider()
                .signature_verification_algorithms
                .all,
        )
        .map_err(io::Error::other)?;
        rustls::client::verify_server_name(&certificate, &self.observed.endpoint.name)
            .map_err(io::Error::other)?;
        Ok(VerifiedPeer(&self.observed))
    }

    pub(crate) fn into_parts(self) -> (TlsStream<S>, Vec<u8>) {
        (self.stream, self.server_public_key)
    }
}

/// Establish possession of the leaf key before deciding whether the endpoint is
/// trusted. This permits an ordinary desktop on a self-signed host and a
/// credential-free review of that host. Only `SignedTls::verify` issues proof.
#[derive(Debug)]
struct SignatureVerifier(WebPkiSupportedAlgorithms);

impl ServerCertVerifier for SignatureVerifier {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        sig: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, sig, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        sig: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, sig, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

pub(crate) async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    endpoint: &Endpoint,
) -> io::Result<SignedTls<S>> {
    handshake_with_key_log(stream, endpoint, Some(rustls::KeyLogFile::new())).await
}

/// Credential checks always pass None, even when SSLKEYLOGFILE is configured.
pub(super) async fn handshake_with_key_log<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    endpoint: &Endpoint,
    key_log: Option<rustls::KeyLogFile>,
) -> io::Result<SignedTls<S>> {
    let provider = Arc::new(ring::default_provider());
    let verifier = SignatureVerifier(provider.signature_verification_algorithms);
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    if let Some(key_log) = key_log {
        config.key_log = Arc::new(key_log);
    }
    config.resumption = rustls::client::Resumption::disabled();
    let mut stream = TlsConnector::from(Arc::new(config))
        .connect(endpoint.name.clone(), stream)
        .await?;
    stream.flush().await?;
    let leaf = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
        .ok_or_else(|| io::Error::other("missing server certificate"))?;
    let certificate = x509_cert::Certificate::from_der(leaf).map_err(io::Error::other)?;
    let spki = certificate
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .map_err(io::Error::other)?;
    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&certificate)
        .ok_or_else(|| io::Error::other("missing CredSSP public key"))?
        .to_vec();
    Ok(SignedTls {
        stream,
        server_public_key,
        observed: ObservedPeer {
            endpoint: endpoint.clone(),
            spki_sha256: Sha256::digest(spki).into(),
        },
    })
}

/// X.224 precedes the ClientHello. Extra plaintext at that boundary cannot
/// become application data on the authenticated TLS stream.
pub(super) fn take_upgrade_stream<S>(framed: TokioFramed<S>) -> io::Result<S> {
    let (stream, leftover) = framed.into_inner();
    if !leftover.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected data after X.224 negotiation",
        ));
    }
    Ok(stream)
}

/// Review obtains a candidate only. The signature is checked, then this
/// connection closes without credentials, static channels or application data.
pub(crate) async fn discover(target: &ConnectionTarget<'_>) -> io::Result<ObservedPeer> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let stream = tokio::net::TcpStream::connect(target.socket_target()).await?;
        discover_on_stream(stream, &target.endpoint).await
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "server identity discovery timed out",
        )
    })?
}

async fn discover_on_stream<S: AsyncRead + AsyncWrite + Unpin + Send + Sync>(
    mut stream: S,
    endpoint: &Endpoint,
) -> io::Result<ObservedPeer> {
    let request = X224(ConnectionRequest {
        nego_data: None,
        flags: RequestFlags::empty(),
        protocol: SecurityProtocol::SSL | SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX,
        correlation_info: None,
    });
    stream
        .write_all(&ironrdp_core::encode_vec(&request).map_err(io::Error::other)?)
        .await?;
    let mut framed = TokioFramed::new(stream);
    let (_, reply) = framed.read_pdu().await?;
    match ironrdp_core::decode::<X224<ConnectionConfirm>>(&reply)
        .map_err(io::Error::other)?
        .0
    {
        ConnectionConfirm::Response {
            protocol: SecurityProtocol::SSL | SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX,
            ..
        } => (),
        _ => return Err(io::Error::other("server did not select TLS")),
    }
    let mut tls = handshake(take_upgrade_stream(framed)?, endpoint).await?;
    tls.stream.shutdown().await?;
    Ok(tls.observed)
}

#[cfg(test)]
mod tests;
