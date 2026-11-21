//! Standard RDP Security with RC4 ([MS-RDPBCGR] 5.3), for hosts that accept
//! neither TLS nor NLA.
//!
//! The RDP library speaks standard security only in its unencrypted variant
//! (`ENCRYPTION_LEVEL_NONE`). This module sits between it and the socket and
//! rewrites every PDU, so the library sees that variant while the wire
//! carries the encrypted one:
//!
//! - the client's GCC security block offers 40-, 56- and 128-bit RC4;
//! - the server's answer (method, random, certificate) is taken out and shown
//!   to the library as "no encryption";
//! - the Security Exchange PDU is sent before the Client Info PDU;
//! - from then on slow-path and fast-path data is signed and encrypted on
//!   the way out, and checked, decrypted and stripped of its security
//!   header on the way in.
//!
//! This mode is weak by design of the protocol, not of this module: RC4, an
//! MD5/SHA-1 MAC, and no authentication of the server (its key is not
//! certified by anything a client can check), so a machine in the network
//! path can read and change the session, the password included. It is used
//! only for a host whose Security setting says Legacy.

mod crypto;
mod stream;
#[cfg(test)]
mod tests;

pub use stream::LegacyStream;

use crypto::{Method, SessionKeys};

/// Basic security header flags ([MS-RDPBCGR] 2.2.8.1.1.2.1).
mod sec {
    pub const EXCHANGE_PKT: u16 = 0x0001;
    pub const TRANSPORT_REQ: u16 = 0x0002;
    pub const ENCRYPT: u16 = 0x0008;
    pub const LICENSE_PKT: u16 = 0x0080;
    pub const LICENSE_ENCRYPT_CS: u16 = 0x0200;
    pub const REDIRECTION_PKT: u16 = 0x0400;
    pub const SECURE_CHECKSUM: u16 = 0x0800;
    pub const AUTODETECT_REQ: u16 = 0x1000;
    pub const HEARTBEAT: u16 = 0x4000;
    /// Server PDUs that keep a security header without encryption too.
    pub const KEEPS_HEADER: u16 =
        LICENSE_PKT | TRANSPORT_REQ | REDIRECTION_PKT | AUTODETECT_REQ | HEARTBEAT;
}

/// Fast-path header flags, in the two high bits of the first byte.
const FASTPATH_SECURE_CHECKSUM: u8 = 0x40;
const FASTPATH_ENCRYPTED: u8 = 0x80;

const TPKT_VERSION: u8 = 3;
/// TPKT header and the X.224 data header (`02 F0 80`).
const X224_DATA: usize = 7;
const SIGNATURE: usize = 8;
const SERVER_RANDOM: usize = 32;

/// MCS domain PDU choices this module looks into.
const SEND_DATA_REQUEST: u8 = 25;
const SEND_DATA_INDICATION: u8 = 26;

/// GCC user data blocks.
const CS_SECURITY: u16 = 0xC002;
const SC_SECURITY: u16 = 0x0C02;
const SC_MESSAGE_CHANNEL: u16 = 0x0C04;

type Result<T> = std::result::Result<T, String>;

/// The length of the PDU at the start of `bytes`, once enough of it arrived.
pub(super) fn pdu_length(bytes: &[u8]) -> Result<Option<usize>> {
    let Some(&first) = bytes.first() else {
        return Ok(None);
    };
    let length = if first == TPKT_VERSION {
        if bytes.len() < 4 {
            return Ok(None);
        }
        usize::from(u16::from_be_bytes([bytes[2], bytes[3]]))
    } else {
        match fast_path_length(bytes) {
            Some((length, _)) => length,
            None => return Ok(None),
        }
    };
    if length < 4 {
        return Err(format!("a PDU of {length} bytes"));
    }
    Ok(Some(length))
}

/// A fast-path PDU's length field and where its body starts.
fn fast_path_length(bytes: &[u8]) -> Option<(usize, usize)> {
    let first = *bytes.get(1)?;
    if first & 0x80 == 0 {
        return Some((usize::from(first), 2));
    }
    let second = *bytes.get(2)?;
    Some(((usize::from(first & 0x7F) << 8) | usize::from(second), 3))
}

/// A slow-path PDU that carries data on an MCS channel.
struct SendData<'a> {
    /// TPKT, X.224 and MCS headers up to the length of the user data.
    header: &'a [u8],
    channel: u16,
    data: &'a [u8],
}

impl<'a> SendData<'a> {
    fn parse(pdu: &'a [u8], choice: u8) -> Option<Self> {
        if pdu.len() < X224_DATA + 7 || pdu[4..7] != [0x02, 0xF0, 0x80] {
            return None;
        }
        let mcs = &pdu[X224_DATA..];
        if mcs[0] >> 2 != choice {
            return None;
        }
        let channel = u16::from_be_bytes([mcs[3], mcs[4]]);
        // Initiator (2), channel (2), priority and segmentation (1), length.
        let length_bytes = if mcs[6] & 0x80 == 0 { 1 } else { 2 };
        let start = X224_DATA + 6 + length_bytes;
        Some(Self {
            header: &pdu[..X224_DATA + 6],
            channel,
            data: pdu.get(start..)?,
        })
    }

    /// Writes the PDU again with other user data.
    fn rebuild(&self, parts: &[&[u8]], out: &mut Vec<u8>) -> Result<()> {
        let data_len: usize = parts.iter().map(|part| part.len()).sum();
        let length_bytes = if data_len < 0x80 { 1 } else { 2 };
        let total = self.header.len() + length_bytes + data_len;
        if data_len > 0x7FFF || total > usize::from(u16::MAX) {
            return Err(format!("{data_len} bytes do not fit one MCS PDU"));
        }
        let start = out.len();
        out.extend_from_slice(self.header);
        out[start + 2..start + 4].copy_from_slice(&(total as u16).to_be_bytes());
        if length_bytes == 1 {
            out.push(data_len as u8);
        } else {
            out.extend_from_slice(&(data_len as u16 | 0x8000).to_be_bytes());
        }
        for part in parts {
            out.extend_from_slice(part);
        }
        Ok(())
    }
}

/// The blocks of GCC user data in a Connect Initial or Connect Response:
/// `(type, body offset, body length)`, found behind the H.221 key.
fn gcc_blocks(pdu: &[u8], key: &[u8; 4]) -> Vec<(u16, usize, usize)> {
    let mut blocks = Vec::new();
    let Some(key_at) = pdu.windows(4).position(|window| window == key) else {
        return blocks;
    };
    // The key is followed by the PER length of the user data.
    let mut at = key_at + 4;
    at += match pdu.get(at) {
        Some(first) if first & 0x80 != 0 => 2,
        _ => 1,
    };
    while let Some(header) = pdu.get(at..at + 4) {
        let kind = u16::from_le_bytes([header[0], header[1]]);
        let length = usize::from(u16::from_le_bytes([header[2], header[3]]));
        if length < 4 || at + length > pdu.len() {
            break;
        }
        blocks.push((kind, at + 4, length - 4));
        at += length;
    }
    blocks
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The server's RSA key from its certificate ([MS-RDPBCGR] 2.2.1.4.3.1).
struct ServerKey {
    /// Little-endian, without the padding the proprietary format appends.
    modulus: Vec<u8>,
    exponent: u32,
}

impl ServerKey {
    fn parse(certificate: &[u8]) -> Result<Self> {
        let version = u32_at(certificate, 0).ok_or("the server certificate is cut short")?;
        // The high bit marks a temporary certificate.
        match version & 0x7FFF_FFFF {
            1 => Self::proprietary(&certificate[4..]),
            2 => Self::x509_chain(&certificate[4..]),
            other => Err(format!("server certificate version {other}")),
        }
        .and_then(|key| {
            if key.modulus.len() < 64 {
                return Err(format!("a server key of {} bits", key.modulus.len() * 8));
            }
            Ok(key)
        })
    }

    fn proprietary(body: &[u8]) -> Result<Self> {
        let cut = "the server certificate is cut short";
        // dwSigAlgId, dwKeyAlgId, wPublicKeyBlobType, wPublicKeyBlobLen.
        let blob = body.get(12..).ok_or(cut)?;
        if blob.get(..4) != Some(b"RSA1") {
            return Err("the server certificate holds no RSA key".into());
        }
        let key_len = u32_at(blob, 4).ok_or(cut)? as usize;
        let bits = u32_at(blob, 8).ok_or(cut)? as usize;
        let exponent = u32_at(blob, 16).ok_or(cut)?;
        let modulus = blob.get(20..20 + key_len).ok_or(cut)?;
        let modulus = modulus.get(..bits.div_ceil(8)).ok_or(cut)?;
        Ok(Self {
            modulus: modulus.to_vec(),
            exponent,
        })
    }

    /// The last certificate of the chain is the server's.
    fn x509_chain(body: &[u8]) -> Result<Self> {
        use x509_cert::der::{Decode as _, Reader as _, SliceReader, asn1::UintRef};

        let cut = "the server certificate chain is cut short";
        let count = u32_at(body, 0).ok_or(cut)?;
        let mut at = 4;
        let mut leaf = None;
        for _ in 0..count {
            let length = u32_at(body, at).ok_or(cut)? as usize;
            leaf = Some(body.get(at + 4..at + 4 + length).ok_or(cut)?);
            at += 4 + length;
        }
        let leaf = leaf.ok_or("the server certificate chain is empty")?;
        let certificate = x509_cert::Certificate::from_der(leaf)
            .map_err(|error| format!("server certificate: {error}"))?;
        let key = ironrdp_tls::extract_tls_server_public_key(&certificate)
            .ok_or("the server certificate holds no key")?;
        // RSAPublicKey ::= SEQUENCE { modulus INTEGER, publicExponent INTEGER }
        let mut reader = SliceReader::new(key).map_err(|error| error.to_string())?;
        let (modulus, exponent) = reader
            .sequence(|reader| Ok((UintRef::decode(reader)?, UintRef::decode(reader)?)))
            .map_err(|error: x509_cert::der::Error| format!("server RSA key: {error}"))?;
        let exponent = exponent
            .as_bytes()
            .iter()
            .try_fold(0u32, |value, byte| {
                value.checked_mul(256).map(|v| v | u32::from(*byte))
            })
            .ok_or("the server's RSA exponent is too large")?;
        let mut modulus = modulus.as_bytes().to_vec();
        modulus.reverse();
        Ok(Self { modulus, exponent })
    }
}

/// What the server chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Negotiated {
    /// RC4 key strength in bits.
    pub bits: u32,
    /// `encryptionLevel`: 1 low, 2 client compatible, 3 high.
    pub level: u32,
}

struct Active {
    keys: SessionKeys,
    /// The Security Exchange PDU's body, sent with the Client Info PDU.
    exchange: Option<Vec<u8>>,
    io_channel: Option<u16>,
    message_channel: Option<u16>,
    /// The server sent something other than licensing on the I/O channel.
    licensed: bool,
    /// The server asked for encrypted client licensing PDUs.
    encrypt_licensing: bool,
}

/// Rewrites PDUs between the library's unencrypted standard security and
/// the server's encrypted one. One PDU at a time, in wire order per
/// direction.
#[derive(Default)]
pub struct Transcoder {
    /// `None` until the server selects a method, and for a server that
    /// selects none: PDUs then pass unchanged.
    active: Option<Active>,
    negotiated: Option<Negotiated>,
    /// For tests: the client random instead of a fresh one.
    client_random: Option<[u8; 32]>,
}

impl Transcoder {
    #[cfg(test)]
    pub fn negotiated(&self) -> Option<Negotiated> {
        self.negotiated
    }

    /// A PDU from the library, as it goes to the server.
    pub fn outgoing(&mut self, pdu: &[u8], out: &mut Vec<u8>) -> Result<()> {
        if pdu[0] != TPKT_VERSION {
            return self.fast_path_out(pdu, out);
        }
        if self.active.is_none() {
            out.extend_from_slice(pdu);
            // MCS Connect Initial: offer the RC4 methods.
            if pdu.get(X224_DATA..X224_DATA + 2) == Some(&[0x7F, 0x65][..]) {
                let start = out.len() - pdu.len();
                for (kind, at, length) in gcc_blocks(pdu, b"Duca") {
                    if kind == CS_SECURITY && length >= 4 {
                        out[start + at..start + at + 4]
                            .copy_from_slice(&Method::OFFERED.to_le_bytes());
                    }
                }
            }
            return Ok(());
        }
        let Some(send) = SendData::parse(pdu, SEND_DATA_REQUEST) else {
            out.extend_from_slice(pdu);
            return Ok(());
        };
        let active = self.active.as_mut().expect("checked above");

        // The first data the client sends is the Client Info PDU: the
        // Security Exchange PDU goes right before it.
        if let Some(exchange) = active.exchange.take() {
            active.io_channel = Some(send.channel);
            let header = [sec::EXCHANGE_PKT.to_le_bytes(), [0, 0]].concat();
            let length = (exchange.len() as u32).to_le_bytes();
            send.rebuild(&[&header, &length, &exchange], out)?;
            return Self::encrypt_with_header(active, &send, out);
        }
        let on_io = Some(send.channel) == active.io_channel;
        if Some(send.channel) == active.message_channel || (on_io && !active.licensed) {
            let flags = u16::from_le_bytes([send.data[0], send.data[1]]);
            if flags & sec::LICENSE_PKT != 0 && !active.encrypt_licensing {
                out.extend_from_slice(pdu);
                return Ok(());
            }
            return Self::encrypt_with_header(active, &send, out);
        }
        let mut body = send.data.to_vec();
        let signature = active.keys.mac.sign(&body, None);
        active.keys.encrypt.apply(&mut body);
        let header = [sec::ENCRYPT.to_le_bytes(), [0, 0]].concat();
        send.rebuild(&[&header, &signature, &body], out)
    }

    /// Encrypts a PDU that carries its own basic security header.
    fn encrypt_with_header(
        active: &mut Active,
        send: &SendData<'_>,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let (header, body) = send
            .data
            .split_at_checked(4)
            .ok_or("a PDU without its security header")?;
        let flags = u16::from_le_bytes([header[0], header[1]]) | sec::ENCRYPT;
        let mut body = body.to_vec();
        let signature = active.keys.mac.sign(&body, None);
        active.keys.encrypt.apply(&mut body);
        send.rebuild(
            &[&flags.to_le_bytes(), &header[2..], &signature, &body],
            out,
        )
    }

    fn fast_path_out(&mut self, pdu: &[u8], out: &mut Vec<u8>) -> Result<()> {
        let Some(active) = self.active.as_mut() else {
            out.extend_from_slice(pdu);
            return Ok(());
        };
        let (_, start) = fast_path_length(pdu).ok_or("a fast-path PDU without length")?;
        let mut body = pdu[start..].to_vec();
        let signature = active.keys.mac.sign(&body, None);
        active.keys.encrypt.apply(&mut body);
        let total = 3 + SIGNATURE + body.len();
        if total > 0x7FFF {
            return Err(format!("{total} bytes do not fit one fast-path PDU"));
        }
        out.push(pdu[0] | FASTPATH_ENCRYPTED);
        out.extend_from_slice(&(total as u16 | 0x8000).to_be_bytes());
        out.extend_from_slice(&signature);
        out.extend_from_slice(&body);
        Ok(())
    }

    /// A PDU from the server, as the library gets it.
    pub fn incoming(&mut self, pdu: &[u8], out: &mut Vec<u8>) -> Result<()> {
        if pdu[0] != TPKT_VERSION {
            return self.fast_path_in(pdu, out);
        }
        if self.active.is_none() {
            let start = out.len();
            out.extend_from_slice(pdu);
            // MCS Connect Response: the server's method, random and key.
            if self.negotiated.is_none()
                && pdu.get(X224_DATA..X224_DATA + 2) == Some(&[0x7F, 0x66][..])
            {
                self.begin(pdu, &mut out[start..])?;
            }
            return Ok(());
        }
        let Some(send) = SendData::parse(pdu, SEND_DATA_INDICATION) else {
            out.extend_from_slice(pdu);
            return Ok(());
        };
        let active = self.active.as_mut().expect("checked above");
        let (header, rest) = send
            .data
            .split_at_checked(4)
            .ok_or("a server PDU without its security header")?;
        let flags = u16::from_le_bytes([header[0], header[1]]);
        let body = if flags & sec::ENCRYPT != 0 {
            let (signature, cipher) = rest
                .split_at_checked(SIGNATURE)
                .ok_or("an encrypted server PDU without its signature")?;
            let mut body = cipher.to_vec();
            let index = active.keys.decrypt.apply(&mut body);
            let salted = (flags & sec::SECURE_CHECKSUM != 0).then_some(index);
            if active.keys.mac.sign(&body, salted) != signature {
                return Err("a server PDU failed its integrity check".into());
            }
            body
        } else {
            rest.to_vec()
        };

        if flags & sec::LICENSE_PKT != 0 {
            active.encrypt_licensing |= flags & sec::LICENSE_ENCRYPT_CS != 0;
        } else if Some(send.channel) == active.io_channel {
            active.licensed = true;
        }
        if flags & sec::KEEPS_HEADER != 0 {
            let flags = flags & !(sec::ENCRYPT | sec::SECURE_CHECKSUM);
            send.rebuild(&[&flags.to_le_bytes(), &header[2..], &body], out)
        } else {
            send.rebuild(&[&body], out)
        }
    }

    fn fast_path_in(&mut self, pdu: &[u8], out: &mut Vec<u8>) -> Result<()> {
        let active = match self.active.as_mut() {
            Some(active) if pdu[0] & FASTPATH_ENCRYPTED != 0 => active,
            _ => {
                out.extend_from_slice(pdu);
                return Ok(());
            }
        };
        let (_, start) = fast_path_length(pdu).ok_or("a fast-path PDU without length")?;
        let (signature, cipher) = pdu[start..]
            .split_at_checked(SIGNATURE)
            .ok_or("an encrypted fast-path PDU without its signature")?;
        let mut body = cipher.to_vec();
        let index = active.keys.decrypt.apply(&mut body);
        let salted = (pdu[0] & FASTPATH_SECURE_CHECKSUM != 0).then_some(index);
        if active.keys.mac.sign(&body, salted) != signature {
            return Err("a server update failed its integrity check".into());
        }
        out.push(pdu[0] & !(FASTPATH_ENCRYPTED | FASTPATH_SECURE_CHECKSUM));
        // The two-byte length form holds every size the server can send.
        out.extend_from_slice(&((3 + body.len()) as u16 | 0x8000).to_be_bytes());
        out.extend_from_slice(&body);
        Ok(())
    }

    /// Reads the server's security data out of the Connect Response and
    /// rewrites `copy`, the bytes the library will see, to "no encryption".
    fn begin(&mut self, pdu: &[u8], copy: &mut [u8]) -> Result<()> {
        let blocks = gcc_blocks(pdu, b"McDn");
        let message_channel = blocks
            .iter()
            .find(|(kind, _, length)| *kind == SC_MESSAGE_CHANNEL && *length >= 2)
            .map(|(_, at, _)| u16::from_le_bytes([pdu[*at], pdu[*at + 1]]));
        let Some(&(_, at, length)) = blocks.iter().find(|(kind, ..)| *kind == SC_SECURITY) else {
            return Ok(());
        };
        let block = &pdu[at..at + length];
        let cut = "the server's security data is cut short";
        let method = u32_at(block, 0).ok_or(cut)?;
        let level = u32_at(block, 4).ok_or(cut)?;
        if method == 0 && level == 0 {
            return Ok(());
        }
        let method = Method::from_wire(method).ok_or_else(|| {
            format!("the server requires an encryption method 1-1 lacks ({method:#x}, FIPS?)")
        })?;
        let random_len = u32_at(block, 8).ok_or(cut)? as usize;
        let certificate_len = u32_at(block, 12).ok_or(cut)? as usize;
        if random_len != SERVER_RANDOM {
            return Err(format!("a server random of {random_len} bytes"));
        }
        let server_random: [u8; 32] = block
            .get(16..16 + SERVER_RANDOM)
            .and_then(|random| random.try_into().ok())
            .ok_or(cut)?;
        let certificate = block
            .get(16 + SERVER_RANDOM..16 + SERVER_RANDOM + certificate_len)
            .ok_or(cut)?;
        let key = ServerKey::parse(certificate)?;

        let client_random = match self.client_random {
            Some(random) => random,
            None => {
                let mut random = [0u8; 32];
                getrandom::fill(&mut random)
                    .map_err(|error| format!("no random bytes: {error}"))?;
                random
            }
        };
        let mut exchange =
            crypto::encrypt_client_random(&client_random, &key.modulus, key.exponent);
        // Eight bytes of padding follow the encrypted random.
        exchange.resize(exchange.len() + 8, 0);

        let negotiated = Negotiated {
            bits: method.bits(),
            level,
        };
        log::info!(
            "Legacy RDP security: {}-bit RC4, encryption level {}, server key {} bits \
             (not authenticated)",
            negotiated.bits,
            negotiated.level,
            key.modulus.len() * 8,
        );
        // The library accepts only "no encryption" here; the rest of the
        // block stays and is not read.
        copy[at..at + 8].fill(0);
        self.negotiated = Some(negotiated);
        self.active = Some(Active {
            keys: SessionKeys::derive(&client_random, &server_random, method),
            exchange: Some(exchange),
            io_channel: None,
            message_channel,
            licensed: false,
            encrypt_licensing: false,
        });
        Ok(())
    }
}
