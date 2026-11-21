//! Keys, RC4 and MACs of Standard RDP Security ([MS-RDPBCGR] 5.3.4–5.3.7).

use md5::{Digest as _, Md5};
use num_bigint::BigUint;
use sha1::{Digest as _, Sha1};

/// The salt that reduces a 128-bit key to 40 bits; its first byte alone
/// reduces one to 56 bits ([MS-RDPBCGR] 5.3.5.1).
const SALT_40: [u8; 3] = [0xD1, 0x26, 0x9E];
/// A key is replaced after this many packets (5.3.7).
const PACKETS_PER_KEY: u32 = 4096;
const PAD_36: [u8; 40] = [0x36; 40];
const PAD_5C: [u8; 48] = [0x5C; 48];

/// The RC4 strength the server selected (`encryptionMethod`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Bit40,
    Bit56,
    Bit128,
}

impl Method {
    /// The methods 1-1 offers: `40BIT | 128BIT | 56BIT`. FIPS (3DES) is not.
    pub const OFFERED: u32 = 0x01 | 0x02 | 0x08;

    pub fn from_wire(value: u32) -> Option<Self> {
        match value {
            0x01 => Some(Self::Bit40),
            0x02 => Some(Self::Bit128),
            0x08 => Some(Self::Bit56),
            _ => None,
        }
    }

    pub fn bits(self) -> u32 {
        match self {
            Self::Bit40 => 40,
            Self::Bit56 => 56,
            Self::Bit128 => 128,
        }
    }

    /// Bytes of key material RC4 and the MAC use.
    fn key_len(self) -> usize {
        match self {
            Self::Bit128 => 16,
            Self::Bit40 | Self::Bit56 => 8,
        }
    }

    /// Weakens a derived key to the method's strength.
    fn salt(self, key: &mut [u8; 16]) {
        match self {
            Self::Bit40 => key[..3].copy_from_slice(&SALT_40),
            Self::Bit56 => key[0] = SALT_40[0],
            Self::Bit128 => {}
        }
    }
}

pub struct Rc4 {
    state: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    pub fn new(key: &[u8]) -> Self {
        let mut state = [0u8; 256];
        for (index, slot) in state.iter_mut().enumerate() {
            *slot = index as u8;
        }
        let mut j = 0u8;
        for i in 0..256 {
            j = j.wrapping_add(state[i]).wrapping_add(key[i % key.len()]);
            state.swap(i, usize::from(j));
        }
        Self { state, i: 0, j: 0 }
    }

    pub fn apply(&mut self, data: &mut [u8]) {
        for byte in data {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.state[usize::from(self.i)]);
            self.state.swap(usize::from(self.i), usize::from(self.j));
            let index =
                self.state[usize::from(self.i)].wrapping_add(self.state[usize::from(self.j)]);
            *byte ^= self.state[usize::from(index)];
        }
    }
}

fn md5(parts: &[&[u8]]) -> [u8; 16] {
    let mut hash = Md5::new();
    for part in parts {
        hash.update(part);
    }
    hash.finalize().into()
}

fn sha1(parts: &[&[u8]]) -> [u8; 20] {
    let mut hash = Sha1::new();
    for part in parts {
        hash.update(part);
    }
    hash.finalize().into()
}

/// `SaltedHash(S, I) = MD5(S + SHA(I + S + ClientRandom + ServerRandom))`.
fn salted_hash(secret: &[u8; 48], label: &[u8], client: &[u8; 32], server: &[u8; 32]) -> [u8; 16] {
    md5(&[secret, &sha1(&[label, secret, client, server])])
}

/// Three salted hashes with the labels of the master secret or the key blob.
fn expand(secret: &[u8; 48], labels: [&[u8]; 3], client: &[u8; 32], server: &[u8; 32]) -> [u8; 48] {
    let mut out = [0u8; 48];
    for (chunk, label) in out.as_chunks_mut::<16>().0.iter_mut().zip(labels) {
        chunk.copy_from_slice(&salted_hash(secret, label, client, server));
    }
    out
}

/// The session's keys as the client uses them.
pub struct SessionKeys {
    pub mac: MacKey,
    pub encrypt: Cipher,
    pub decrypt: Cipher,
}

impl SessionKeys {
    /// Non-FIPS key derivation ([MS-RDPBCGR] 5.3.5.1).
    pub fn derive(client: &[u8; 32], server: &[u8; 32], method: Method) -> Self {
        let mut pre_master = [0u8; 48];
        pre_master[..24].copy_from_slice(&client[..24]);
        pre_master[24..].copy_from_slice(&server[..24]);
        let master = expand(&pre_master, [b"A", b"BB", b"CCC"], client, server);
        let blob = expand(&master, [b"X", b"YY", b"ZZZ"], client, server);

        let mut mac = [0u8; 16];
        mac.copy_from_slice(&blob[..16]);
        let mut decrypt = md5(&[&blob[16..32], client, server]);
        let mut encrypt = md5(&[&blob[32..48], client, server]);
        method.salt(&mut mac);
        method.salt(&mut decrypt);
        method.salt(&mut encrypt);
        Self {
            mac: MacKey { key: mac, method },
            encrypt: Cipher::new(encrypt, method),
            decrypt: Cipher::new(decrypt, method),
        }
    }
}

/// One direction's RC4 stream with its key updates.
pub struct Cipher {
    method: Method,
    initial: [u8; 16],
    current: [u8; 16],
    rc4: Rc4,
    /// Packets under the current key.
    uses: u32,
    /// Packets since the connection began (the salted MAC's counter).
    total: u32,
}

impl Cipher {
    fn new(key: [u8; 16], method: Method) -> Self {
        Self {
            method,
            initial: key,
            current: key,
            rc4: Rc4::new(&key[..method.key_len()]),
            uses: 0,
            total: 0,
        }
    }

    /// Encrypts or decrypts one packet's data in place. Returns how many
    /// packets went before it, which the salted MAC includes.
    pub fn apply(&mut self, data: &mut [u8]) -> u32 {
        if self.uses >= PACKETS_PER_KEY {
            self.update_key();
        }
        self.rc4.apply(data);
        self.uses += 1;
        let index = self.total;
        self.total = self.total.wrapping_add(1);
        index
    }

    /// The session key update of [MS-RDPBCGR] 5.3.7.
    fn update_key(&mut self) {
        let len = self.method.key_len();
        let inner = sha1(&[&self.initial[..len], &PAD_36, &self.current[..len]]);
        let mut next = md5(&[&self.initial[..len], &PAD_5C, &inner]);
        Rc4::new(&next[..len]).apply(&mut next[..len]);
        self.method.salt(&mut next);
        self.current = next;
        self.rc4 = Rc4::new(&next[..len]);
        self.uses = 0;
    }
}

pub struct MacKey {
    key: [u8; 16],
    method: Method,
}

impl MacKey {
    /// The 8-byte data signature over plaintext ([MS-RDPBCGR] 5.3.6.1);
    /// `packets_before` makes it the salted variant (5.3.6.1.1).
    pub fn sign(&self, data: &[u8], packets_before: Option<u32>) -> [u8; 8] {
        let key = &self.key[..self.method.key_len()];
        let length = (data.len() as u32).to_le_bytes();
        let counter = packets_before.map(u32::to_le_bytes);
        let inner = match &counter {
            Some(counter) => sha1(&[key, &PAD_36, &length, data, counter]),
            None => sha1(&[key, &PAD_36, &length, data]),
        };
        let outer = md5(&[key, &PAD_5C, &inner]);
        let mut signature = [0u8; 8];
        signature.copy_from_slice(&outer[..8]);
        signature
    }
}

/// The client random under the server's RSA key, as the Security Exchange
/// PDU carries it: little-endian, the modulus's length, without padding
/// scheme ([MS-RDPBCGR] 5.3.4.1).
pub fn encrypt_client_random(random: &[u8; 32], modulus_le: &[u8], exponent: u32) -> Vec<u8> {
    let cipher = BigUint::from_bytes_le(random).modpow(
        &BigUint::from(exponent),
        &BigUint::from_bytes_le(modulus_le),
    );
    let mut out = cipher.to_bytes_le();
    out.resize(modulus_le.len(), 0);
    out
}
