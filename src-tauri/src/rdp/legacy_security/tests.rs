//! The transcoder against a scripted server that does its own half of
//! Standard RDP Security: it recovers the client random with the private
//! key, derives the keys, and encrypts and decrypts as a server would.

use num_bigint::BigUint;

use super::crypto::{Cipher, MacKey, Method, Rc4, SessionKeys};
use super::*;

/// A throwaway 512-bit RSA key (little-endian modulus and private exponent).
const MODULUS: &str = "e345b4bb90f6459aba5cfeb2c2351281bec8e653da97db22c4551eb56f928f2f\
                       da1645f60fca2a01667f1d180306391f5b65a8795819a74f4eb69e6c58e053a3";
const PRIVATE: &str = "91d049145d1132fb5e638df47a32c2ed85795d687e257ac6765da8c24e5b9ca1\
                       e462ca215c97217584a30f19809676623070065bdb0abd4448844be842f6b642";
const SERVER_RANDOM: [u8; 32] = [0x5A; 32];
const CLIENT_RANDOM: [u8; 32] = [0xC3; 32];
const IO_CHANNEL: u16 = 1003;
const MESSAGE_CHANNEL: u16 = 1006;
const DRIVE_CHANNEL: u16 = 1004;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn x224_data(mcs: &[u8]) -> Vec<u8> {
    let mut pdu = vec![3, 0, 0, 0, 0x02, 0xF0, 0x80];
    pdu.extend_from_slice(mcs);
    let length = (pdu.len() as u16).to_be_bytes();
    pdu[2..4].copy_from_slice(&length);
    pdu
}

fn send_data(choice: u8, channel: u16, data: &[u8]) -> Vec<u8> {
    let mut mcs = vec![choice << 2, 0x00, 0x03];
    mcs.extend_from_slice(&channel.to_be_bytes());
    mcs.push(0x70);
    if data.len() < 0x80 {
        mcs.push(data.len() as u8);
    } else {
        mcs.extend_from_slice(&(data.len() as u16 | 0x8000).to_be_bytes());
    }
    mcs.extend_from_slice(data);
    x224_data(&mcs)
}

fn from_client(channel: u16, data: &[u8]) -> Vec<u8> {
    send_data(SEND_DATA_REQUEST, channel, data)
}

fn from_server(channel: u16, data: &[u8]) -> Vec<u8> {
    send_data(SEND_DATA_INDICATION, channel, data)
}

fn block(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut block = kind.to_le_bytes().to_vec();
    block.extend_from_slice(&(body.len() as u16 + 4).to_le_bytes());
    block.extend_from_slice(body);
    block
}

fn connect_initial() -> Vec<u8> {
    let blocks = [block(0xC001, &[7; 20]), block(CS_SECURITY, &[0; 8])].concat();
    let mut mcs = vec![0x7F, 0x65, 0x82, 0x01, 0x00];
    mcs.extend_from_slice(b"Duca");
    mcs.push(blocks.len() as u8);
    mcs.extend_from_slice(&blocks);
    x224_data(&mcs)
}

fn proprietary_certificate() -> Vec<u8> {
    let mut modulus = hex(MODULUS);
    modulus.resize(72, 0);
    let mut key = b"RSA1".to_vec();
    for value in [72u32, 512, 63, 65537] {
        key.extend_from_slice(&value.to_le_bytes());
    }
    key.extend_from_slice(&modulus);
    let mut certificate = Vec::new();
    for value in [1u32, 1, 1] {
        certificate.extend_from_slice(&value.to_le_bytes());
    }
    certificate.extend_from_slice(&6u16.to_le_bytes());
    certificate.extend_from_slice(&(key.len() as u16).to_le_bytes());
    certificate.extend_from_slice(&key);
    certificate.extend_from_slice(&8u16.to_le_bytes());
    certificate.extend_from_slice(&72u16.to_le_bytes());
    certificate.extend_from_slice(&[0xEE; 72]);
    certificate
}

fn security_block(method: u32, level: u32) -> Vec<u8> {
    let mut body = [method.to_le_bytes(), level.to_le_bytes()].concat();
    if method != 0 {
        let certificate = proprietary_certificate();
        body.extend_from_slice(&32u32.to_le_bytes());
        body.extend_from_slice(&(certificate.len() as u32).to_le_bytes());
        body.extend_from_slice(&SERVER_RANDOM);
        body.extend_from_slice(&certificate);
    }
    block(SC_SECURITY, &body)
}

fn connect_response(method: u32, level: u32) -> Vec<u8> {
    let blocks = [
        block(0x0C01, &[9; 12]),
        security_block(method, level),
        block(SC_MESSAGE_CHANNEL, &MESSAGE_CHANNEL.to_le_bytes()),
    ]
    .concat();
    let mut mcs = vec![0x7F, 0x66, 0x82, 0x01, 0x00];
    mcs.extend_from_slice(b"McDn");
    mcs.extend_from_slice(&(blocks.len() as u16 | 0x8000).to_be_bytes());
    mcs.extend_from_slice(&blocks);
    x224_data(&mcs)
}

/// The server's half of the session.
struct Server {
    mac: MacKey,
    encrypt: Cipher,
    decrypt: Cipher,
}

impl Server {
    /// Recovers the client random from the Security Exchange PDU.
    fn from_exchange(pdu: &[u8], method: Method) -> Self {
        let send = SendData::parse(pdu, SEND_DATA_REQUEST).expect("a send data request");
        assert_eq!(send.channel, IO_CHANNEL);
        assert_eq!(&send.data[..4], &[0x01, 0, 0, 0], "SEC_EXCHANGE_PKT");
        assert_eq!(u32_at(send.data, 4), Some(72), "key length and padding");
        assert_eq!(&send.data[8 + 64..], &[0; 8]);
        let random = BigUint::from_bytes_le(&send.data[8..8 + 64]).modpow(
            &BigUint::from_bytes_le(&hex(PRIVATE)),
            &BigUint::from_bytes_le(&hex(MODULUS)),
        );
        let mut client_random = random.to_bytes_le();
        client_random.resize(32, 0);
        assert_eq!(client_random, CLIENT_RANDOM);
        let keys = SessionKeys::derive(&CLIENT_RANDOM, &SERVER_RANDOM, method);
        // What the client encrypts with, the server decrypts with.
        Self {
            mac: keys.mac,
            encrypt: keys.decrypt,
            decrypt: keys.encrypt,
        }
    }

    /// A slow-path PDU as the server sends it encrypted.
    fn slow_path(&mut self, channel: u16, flags: u16, plain: &[u8], salted: bool) -> Vec<u8> {
        let mut body = plain.to_vec();
        let index = self.encrypt.apply(&mut body);
        let mut flags = flags | sec::ENCRYPT;
        if salted {
            flags |= sec::SECURE_CHECKSUM;
        }
        let signature = self.mac.sign(plain, salted.then_some(index));
        let data = [&flags.to_le_bytes()[..], &[0, 0], &signature, &body].concat();
        from_server(channel, &data)
    }

    fn fast_path(&mut self, plain: &[u8], salted: bool) -> Vec<u8> {
        let mut body = plain.to_vec();
        let index = self.encrypt.apply(&mut body);
        let signature = self.mac.sign(plain, salted.then_some(index));
        let flags = FASTPATH_ENCRYPTED | if salted { FASTPATH_SECURE_CHECKSUM } else { 0 };
        let total = 2 + SIGNATURE + body.len();
        assert!(total < 0x80, "the short length form");
        [&[flags, total as u8][..], &signature, &body].concat()
    }

    /// Checks and decrypts a client PDU's user data behind its header.
    fn open(&mut self, data: &[u8]) -> (u16, Vec<u8>) {
        let flags = u16::from_le_bytes([data[0], data[1]]);
        assert_ne!(flags & sec::ENCRYPT, 0, "the client always encrypts");
        let mut body = data[12..].to_vec();
        self.decrypt.apply(&mut body);
        assert_eq!(&data[4..12], &self.mac.sign(&body, None), "data signature");
        (flags, body)
    }

    fn open_slow_path(&mut self, pdu: &[u8], channel: u16) -> (u16, Vec<u8>) {
        let send = SendData::parse(pdu, SEND_DATA_REQUEST).expect("a send data request");
        assert_eq!(send.channel, channel);
        assert_eq!(pdu_length(pdu), Ok(Some(pdu.len())), "TPKT length");
        self.open(send.data)
    }
}

fn transcoder() -> Transcoder {
    Transcoder {
        client_random: Some(CLIENT_RANDOM),
        ..Transcoder::default()
    }
}

/// Runs the handshake up to the Client Info PDU; returns the scripted server.
fn connect(transcoder: &mut Transcoder, method: u32, level: u32) -> Server {
    let mut wire = Vec::new();
    transcoder.outgoing(&connect_initial(), &mut wire).unwrap();
    let mut seen = Vec::new();
    transcoder
        .incoming(&connect_response(method, level), &mut seen)
        .unwrap();

    // Client Info: INFO_PKT header, then the body.
    let info = from_client(IO_CHANNEL, &[&[0x40, 0, 0, 0][..], b"client info"].concat());
    let mut wire = Vec::new();
    transcoder.outgoing(&info, &mut wire).unwrap();
    let exchange_len = pdu_length(&wire).unwrap().unwrap();
    let mut server =
        Server::from_exchange(&wire[..exchange_len], Method::from_wire(method).unwrap());
    let (flags, body) = server.open_slow_path(&wire[exchange_len..], IO_CHANNEL);
    assert_eq!(flags, 0x40 | sec::ENCRYPT);
    assert_eq!(body, b"client info");
    server
}

fn incoming(transcoder: &mut Transcoder, pdu: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    transcoder.incoming(pdu, &mut out).unwrap();
    out
}

fn outgoing(transcoder: &mut Transcoder, pdu: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    transcoder.outgoing(pdu, &mut out).unwrap();
    out
}

#[test]
fn rc4_matches_the_published_vector() {
    let mut data = *b"Plaintext";
    Rc4::new(b"Key").apply(&mut data);
    assert_eq!(data, hex("bbf316e8d940af0ad3")[..]);
}

#[test]
fn the_client_offers_rc4_and_the_library_sees_no_encryption() {
    let mut transcoder = transcoder();
    let initial = connect_initial();
    let sent = outgoing(&mut transcoder, &initial);
    assert_eq!(sent.len(), initial.len());
    let (_, at, _) = gcc_blocks(&sent, b"Duca")
        .into_iter()
        .find(|(kind, ..)| *kind == CS_SECURITY)
        .unwrap();
    assert_eq!(u32_at(&sent, at), Some(0x0B), "40-, 56- and 128-bit RC4");
    assert_eq!(u32_at(&sent, at + 4), Some(0), "no French-locale methods");

    let response = connect_response(0x02, 2);
    let seen = incoming(&mut transcoder, &response);
    assert_eq!(seen.len(), response.len(), "rewritten in place");
    assert_eq!(
        transcoder.negotiated(),
        Some(Negotiated {
            bits: 128,
            level: 2
        })
    );
    let (_, at, length) = gcc_blocks(&seen, b"McDn")
        .into_iter()
        .find(|(kind, ..)| *kind == SC_SECURITY)
        .unwrap();
    // The library's own decoder takes the rewritten block for "no security".
    let decoded: ironrdp_pdu::gcc::ServerSecurityData =
        ironrdp_core::decode(&seen[at..at + length]).unwrap();
    assert_eq!(decoded, ironrdp_pdu::gcc::ServerSecurityData::no_security());
}

#[test]
fn a_session_encrypts_and_decrypts_in_both_directions() {
    for (method, level) in [(0x02, 2), (0x01, 3), (0x08, 2)] {
        let mut transcoder = transcoder();
        let mut server = connect(&mut transcoder, method, level);

        // Licensing: the header stays, without the encryption flag.
        let license = server.slow_path(IO_CHANNEL, sec::LICENSE_PKT, b"valid client", false);
        assert_eq!(
            incoming(&mut transcoder, &license),
            from_server(
                IO_CHANNEL,
                &[&[0x80, 0, 0, 0][..], b"valid client"].concat()
            )
        );
        // Everything else arrives without a security header.
        let demand = server.slow_path(IO_CHANNEL, 0, b"demand active", true);
        assert_eq!(
            incoming(&mut transcoder, &demand),
            from_server(IO_CHANNEL, b"demand active")
        );
        let update = server.fast_path(b"bitmap update", true);
        assert_eq!(
            incoming(&mut transcoder, &update),
            [&[0x00, 0x80, 16][..], b"bitmap update"].concat()
        );
        let heartbeat = server.slow_path(MESSAGE_CHANNEL, sec::HEARTBEAT, b"beat", false);
        assert_eq!(
            incoming(&mut transcoder, &heartbeat),
            from_server(
                MESSAGE_CHANNEL,
                &[&[0x00, 0x40, 0, 0][..], b"beat"].concat()
            )
        );

        // The client's PDUs get a header, a signature and RC4.
        let confirm = outgoing(&mut transcoder, &from_client(IO_CHANNEL, b"confirm active"));
        assert_eq!(
            server.open_slow_path(&confirm, IO_CHANNEL),
            (sec::ENCRYPT, b"confirm active".to_vec())
        );
        let large = vec![0xAB; 1600];
        let chunk = outgoing(&mut transcoder, &from_client(DRIVE_CHANNEL, &large));
        assert_eq!(server.open_slow_path(&chunk, DRIVE_CHANNEL).1, large);
        // Fast-path input: two events, the short length form.
        let input = outgoing(&mut transcoder, &[0x08, 6, 1, 2, 3, 4]);
        assert_eq!(input[0], 0x08 | FASTPATH_ENCRYPTED);
        assert_eq!(pdu_length(&input), Ok(Some(input.len())));
        let mut events = input[11..].to_vec();
        server.decrypt.apply(&mut events);
        assert_eq!(events, [1, 2, 3, 4]);
        assert_eq!(&input[3..11], &server.mac.sign(&events, None));
    }
}

#[test]
fn a_low_encryption_level_leaves_server_data_readable() {
    let mut transcoder = transcoder();
    let mut server = connect(&mut transcoder, 0x02, 1);
    // Level low: the server sends a header without encryption.
    let plain = from_server(IO_CHANNEL, &[&[0, 0, 0, 0][..], b"demand active"].concat());
    assert_eq!(
        incoming(&mut transcoder, &plain),
        from_server(IO_CHANNEL, b"demand active")
    );
    assert_eq!(
        incoming(&mut transcoder, &[0x00, 5, 1, 2, 3]),
        [0x00, 5, 1, 2, 3],
        "an unencrypted fast-path update passes"
    );
    let confirm = outgoing(&mut transcoder, &from_client(IO_CHANNEL, b"confirm"));
    assert_eq!(server.open_slow_path(&confirm, IO_CHANNEL).1, b"confirm");
}

#[test]
fn client_licensing_is_encrypted_only_when_the_server_asks() {
    let license = from_client(IO_CHANNEL, &[&[0x80, 0, 0, 0][..], b"new license"].concat());

    let mut transcoder = transcoder();
    let mut server = connect(&mut transcoder, 0x02, 2);
    let request = server.slow_path(IO_CHANNEL, sec::LICENSE_PKT, b"request", false);
    incoming(&mut transcoder, &request);
    assert_eq!(outgoing(&mut transcoder, &license), license);

    let mut transcoder = self::transcoder();
    let mut server = connect(&mut transcoder, 0x02, 2);
    let flags = sec::LICENSE_PKT | sec::LICENSE_ENCRYPT_CS;
    let request = server.slow_path(IO_CHANNEL, flags, b"request", false);
    incoming(&mut transcoder, &request);
    let sent = outgoing(&mut transcoder, &license);
    assert_eq!(
        server.open_slow_path(&sent, IO_CHANNEL),
        (sec::LICENSE_PKT | sec::ENCRYPT, b"new license".to_vec())
    );
}

#[test]
fn keys_are_replaced_after_4096_packets_in_each_direction() {
    let mut transcoder = transcoder();
    let mut server = connect(&mut transcoder, 0x02, 2);
    incoming(
        &mut transcoder,
        &server.slow_path(IO_CHANNEL, 0, b"demand", false),
    );
    for packet in 0..4100u32 {
        let text = packet.to_le_bytes();
        let update = server.fast_path(&text, packet % 2 == 0);
        assert_eq!(incoming(&mut transcoder, &update)[3..], text, "{packet}");
        let sent = outgoing(&mut transcoder, &from_client(IO_CHANNEL, &text));
        assert_eq!(server.open_slow_path(&sent, IO_CHANNEL).1, text, "{packet}");
    }
}

#[test]
fn a_changed_packet_is_refused() {
    let mut transcoder = transcoder();
    let mut server = connect(&mut transcoder, 0x02, 2);
    let mut update = server.fast_path(b"bitmap update", false);
    *update.last_mut().unwrap() ^= 1;
    let error = transcoder.incoming(&update, &mut Vec::new()).unwrap_err();
    assert!(error.contains("integrity"), "{error}");
}

#[test]
fn a_server_without_encryption_is_passed_through() {
    let mut transcoder = transcoder();
    outgoing(&mut transcoder, &connect_initial());
    let response = connect_response(0, 0);
    assert_eq!(incoming(&mut transcoder, &response), response);
    assert_eq!(transcoder.negotiated(), None);
    let info = from_client(IO_CHANNEL, &[&[0x40, 0, 0, 0][..], b"client info"].concat());
    assert_eq!(outgoing(&mut transcoder, &info), info);
    assert_eq!(outgoing(&mut transcoder, &[0x04, 4, 1, 2]), [0x04, 4, 1, 2]);
}

#[test]
fn fips_is_refused_with_a_reason() {
    let mut transcoder = transcoder();
    let error = transcoder
        .incoming(&connect_response(0x10, 4), &mut Vec::new())
        .unwrap_err();
    assert!(error.contains("FIPS"), "{error}");
}

#[test]
fn negotiation_pdus_pass_unchanged() {
    let mut transcoder = transcoder();
    // X.224 Connection Request and Confirm are not data TPDUs.
    let request = [3, 0, 0, 11, 6, 0xE0, 0, 0, 0, 0, 0];
    assert_eq!(outgoing(&mut transcoder, &request), request);
    let confirm = [3, 0, 0, 11, 6, 0xD0, 0, 0, 0x12, 0x34, 0];
    assert_eq!(incoming(&mut transcoder, &confirm), confirm);
}

#[test]
fn pdu_lengths_wait_for_enough_bytes() {
    assert_eq!(pdu_length(&[]), Ok(None));
    assert_eq!(pdu_length(&[3, 0, 0]), Ok(None));
    assert_eq!(pdu_length(&[3, 0, 1, 2]), Ok(Some(258)));
    assert_eq!(pdu_length(&[0]), Ok(None));
    assert_eq!(pdu_length(&[0, 9]), Ok(Some(9)));
    assert_eq!(pdu_length(&[0, 0x81]), Ok(None));
    assert_eq!(pdu_length(&[0, 0x81, 2]), Ok(Some(258)));
    assert!(pdu_length(&[0, 1]).is_err());
}

#[tokio::test]
async fn the_stream_transcodes_pdus_that_arrive_in_pieces() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let (near, mut far) = tokio::io::duplex(64 * 1024);
    let mut stream = LegacyStream::new(near);
    // Same client random as the scripted server expects.
    stream.transcoder = transcoder();

    stream.write_all(&connect_initial()).await.unwrap();
    let mut initial = vec![0; connect_initial().len()];
    far.read_exact(&mut initial).await.unwrap();

    // The response and an early byte of nothing else, in three-byte pieces.
    let response = connect_response(0x02, 2);
    let writer = tokio::spawn(async move {
        for piece in response.chunks(3) {
            far.write_all(piece).await.unwrap();
            far.flush().await.unwrap();
        }
        far
    });
    let mut seen = vec![0; connect_response(0x02, 2).len()];
    stream.read_exact(&mut seen).await.unwrap();
    let mut far = writer.await.unwrap();
    assert_eq!(&seen[7..9], &[0x7F, 0x66]);

    // The client info, written in two halves, leaves as two PDUs.
    let info = from_client(IO_CHANNEL, &[&[0x40, 0, 0, 0][..], b"client info"].concat());
    let (first, second) = info.split_at(9);
    stream.write_all(first).await.unwrap();
    stream.write_all(second).await.unwrap();
    stream.flush().await.unwrap();
    let mut exchange = vec![0; 13 + 1 + 8 + 72];
    far.read_exact(&mut exchange).await.unwrap();
    let mut server = Server::from_exchange(&exchange, Method::Bit128);
    let mut encrypted = vec![0; info.len() + SIGNATURE];
    far.read_exact(&mut encrypted).await.unwrap();
    assert_eq!(
        server.open_slow_path(&encrypted, IO_CHANNEL).1,
        b"client info"
    );

    // Two server PDUs in one write come out as two plain ones.
    let both = [
        server.slow_path(IO_CHANNEL, 0, b"demand active", false),
        server.fast_path(b"update", false),
    ]
    .concat();
    far.write_all(&both).await.unwrap();
    let expected = [
        from_server(IO_CHANNEL, b"demand active"),
        [&[0x00, 0x80, 9][..], b"update"].concat(),
    ]
    .concat();
    let mut plain = vec![0; expected.len()];
    stream.read_exact(&mut plain).await.unwrap();
    assert_eq!(plain, expected);
}
