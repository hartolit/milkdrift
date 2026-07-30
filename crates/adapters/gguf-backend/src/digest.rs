//! SHA-256 content identity for GGUF artifacts.

use std::fmt::{self, Debug, Display, Formatter};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::str::FromStr;

const SHA256_BLOCK_BYTES: usize = 64;
const SHA256_DIGEST_BYTES: usize = 32;
const FILE_BUFFER_BYTES: usize = 64 * 1024;

const INITIAL_STATE: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

const ROUND_CONSTANTS: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// A complete SHA-256 content digest.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Digest([u8; SHA256_DIGEST_BYTES]);

impl Sha256Digest {
    /// Number of bytes in a SHA-256 digest.
    pub const BYTE_LENGTH: usize = SHA256_DIGEST_BYTES;
    /// Number of lowercase hexadecimal characters in a formatted digest.
    pub const HEX_LENGTH: usize = SHA256_DIGEST_BYTES * 2;

    /// Constructs a digest from its canonical 32-byte representation.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; SHA256_DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the canonical 32-byte representation.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SHA256_DIGEST_BYTES] {
        &self.0
    }

    /// Consumes the digest and returns its canonical bytes.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; SHA256_DIGEST_BYTES] {
        self.0
    }

    /// Parses exactly 64 ASCII hexadecimal characters.
    ///
    /// # Errors
    ///
    /// Returns [`Sha256DigestParseError`] when the length or a character is invalid.
    pub fn from_hex(value: &str) -> Result<Self, Sha256DigestParseError> {
        value.parse()
    }
}

impl Display for Sha256Digest {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Debug for Sha256Digest {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Sha256Digest")
            .field(&format_args!("{self}"))
            .finish()
    }
}

impl FromStr for Sha256Digest {
    type Err = Sha256DigestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let encoded = value.as_bytes();
        if encoded.len() != Self::HEX_LENGTH {
            return Err(Sha256DigestParseError::InvalidLength {
                actual: encoded.len(),
            });
        }

        let mut digest = [0_u8; SHA256_DIGEST_BYTES];
        for (index, byte) in digest.iter_mut().enumerate() {
            let high_index = index * 2;
            let low_index = high_index + 1;
            let high_byte =
                encoded
                    .get(high_index)
                    .copied()
                    .ok_or(Sha256DigestParseError::InvalidLength {
                        actual: encoded.len(),
                    })?;
            let low_byte =
                encoded
                    .get(low_index)
                    .copied()
                    .ok_or(Sha256DigestParseError::InvalidLength {
                        actual: encoded.len(),
                    })?;
            let high = decode_hex(high_byte).ok_or(Sha256DigestParseError::InvalidCharacter {
                index: high_index,
                byte: high_byte,
            })?;
            let low = decode_hex(low_byte).ok_or(Sha256DigestParseError::InvalidCharacter {
                index: low_index,
                byte: low_byte,
            })?;
            *byte = (high << 4) | low;
        }
        Ok(Self(digest))
    }
}

/// Failure while parsing a hexadecimal SHA-256 digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sha256DigestParseError {
    /// The input did not contain exactly 64 bytes.
    InvalidLength {
        /// Actual input length in bytes.
        actual: usize,
    },
    /// One input byte was not an ASCII hexadecimal digit.
    InvalidCharacter {
        /// Byte offset of the invalid character.
        index: usize,
        /// Invalid byte value.
        byte: u8,
    },
}

impl Display for Sha256DigestParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { actual } => write!(
                formatter,
                "SHA-256 digest must contain 64 hexadecimal characters, found {actual}"
            ),
            Self::InvalidCharacter { index, byte } => write!(
                formatter,
                "SHA-256 digest contains invalid byte 0x{byte:02x} at offset {index}"
            ),
        }
    }
}

impl std::error::Error for Sha256DigestParseError {}

/// Computes the SHA-256 digest of an in-memory byte slice.
#[must_use]
pub fn sha256_digest(bytes: &[u8]) -> Sha256Digest {
    let mut state = Sha256State::new();
    state.update(bytes);
    state.finish()
}

/// Computes the SHA-256 digest of a file using bounded streaming I/O.
///
/// # Errors
///
/// Returns the underlying I/O error when the file cannot be opened or read.
pub fn sha256_file(path: impl AsRef<Path>) -> io::Result<Sha256Digest> {
    let mut file = File::open(path)?;
    let mut state = Sha256State::new();
    let mut buffer = vec![0_u8; FILE_BUFFER_BYTES].into_boxed_slice();
    loop {
        let read = file.read(buffer.as_mut())?;
        if read == 0 {
            break;
        }
        let Some(bytes) = buffer.get(..read) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file reader returned an invalid byte count",
            ));
        };
        state.update(bytes);
    }
    Ok(state.finish())
}

const fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

struct Sha256State {
    words: [u32; 8],
    block: [u8; SHA256_BLOCK_BYTES],
    block_length: usize,
    message_length: u64,
}

impl Sha256State {
    const fn new() -> Self {
        Self {
            words: INITIAL_STATE,
            block: [0; SHA256_BLOCK_BYTES],
            block_length: 0,
            message_length: 0,
        }
    }

    fn update(&mut self, mut bytes: &[u8]) {
        self.message_length = self
            .message_length
            .wrapping_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));

        if self.block_length != 0 {
            let copied = (SHA256_BLOCK_BYTES - self.block_length).min(bytes.len());
            let end = self.block_length + copied;
            if let (Some(target), Some(source)) = (
                self.block.get_mut(self.block_length..end),
                bytes.get(..copied),
            ) {
                target.copy_from_slice(source);
            }
            self.block_length = end;
            bytes = bytes.get(copied..).unwrap_or_default();
            if self.block_length == SHA256_BLOCK_BYTES {
                let block = self.block;
                self.compress(&block);
                self.block_length = 0;
            }
        }

        while bytes.len() >= SHA256_BLOCK_BYTES {
            let Some(source) = bytes.get(..SHA256_BLOCK_BYTES) else {
                break;
            };
            let mut block = [0_u8; SHA256_BLOCK_BYTES];
            block.copy_from_slice(source);
            self.compress(&block);
            bytes = bytes.get(SHA256_BLOCK_BYTES..).unwrap_or_default();
        }

        if !bytes.is_empty()
            && let Some(target) = self.block.get_mut(..bytes.len())
        {
            target.copy_from_slice(bytes);
            self.block_length = bytes.len();
        }
    }

    fn finish(mut self) -> Sha256Digest {
        let bit_length = self.message_length.wrapping_mul(8);
        if let Some(marker) = self.block.get_mut(self.block_length) {
            *marker = 0x80;
        }
        self.block_length = self.block_length.saturating_add(1);

        if self.block_length > 56 {
            if let Some(padding) = self.block.get_mut(self.block_length..) {
                padding.fill(0);
            }
            let block = self.block;
            self.compress(&block);
            self.block = [0; SHA256_BLOCK_BYTES];
        } else if let Some(padding) = self.block.get_mut(self.block_length..56) {
            padding.fill(0);
        }

        if let Some(length_bytes) = self.block.get_mut(56..) {
            length_bytes.copy_from_slice(&bit_length.to_be_bytes());
        }
        let block = self.block;
        self.compress(&block);

        let mut digest = [0_u8; SHA256_DIGEST_BYTES];
        for (target, word) in digest.chunks_exact_mut(4).zip(self.words) {
            target.copy_from_slice(&word.to_be_bytes());
        }
        Sha256Digest(digest)
    }

    #[allow(clippy::many_single_char_names)]
    #[expect(
        clippy::indexing_slicing,
        reason = "the SHA-256 16..64 schedule range proves every relative index is in bounds"
    )]
    fn compress(&mut self, block: &[u8; SHA256_BLOCK_BYTES]) {
        let mut schedule = [0_u32; 64];
        for (word, bytes) in schedule.iter_mut().take(16).zip(block.chunks_exact(4)) {
            let mut encoded = [0_u8; 4];
            encoded.copy_from_slice(bytes);
            *word = u32::from_be_bytes(encoded);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.words;
        for (constant, scheduled) in ROUND_CONSTANTS.into_iter().zip(schedule) {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temporary1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(constant)
                .wrapping_add(scheduled);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary2 = sum0.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary1);
            d = c;
            c = b;
            b = a;
            a = temporary1.wrapping_add(temporary2);
        }

        self.words[0] = self.words[0].wrapping_add(a);
        self.words[1] = self.words[1].wrapping_add(b);
        self.words[2] = self.words[2].wrapping_add(c);
        self.words[3] = self.words[3].wrapping_add(d);
        self.words[4] = self.words[4].wrapping_add(e);
        self.words[5] = self.words[5].wrapping_add(f);
        self.words[6] = self.words[6].wrapping_add(g);
        self.words[7] = self.words[7].wrapping_add(h);
    }
}
