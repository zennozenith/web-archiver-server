use std::convert::TryFrom;
use std::fmt;
use std::fmt::Write as _;
use std::ops::Deref;
use std::str::FromStr;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Blake3Hash([u8; 32]);

impl Blake3Hash {
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(self.0.len().saturating_mul(2));
        for byte in self.0 {
            write!(out, "{byte:02x}").expect("writing to a String cannot fail");
        }
        out
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_bytes<T: AsRef<[u8]>>(bytes: T) -> Result<Self, HashParseError> {
        let slice = bytes.as_ref();
        let arr: [u8; 32] = slice
            .try_into()
            .map_err(|_| HashParseError::InvalidLength(slice.len()))?;
        Ok(Self(arr))
    }
}

impl fmt::Debug for Blake3Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Blake3Hash({})", self.to_hex())
    }
}

// // Display: hex string
// impl fmt::Display for Blake3Hash {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         f.write_str(&self.to_hex())
//     }
// }

impl From<&[u8; 32]> for Blake3Hash {
    fn from(value: &[u8; 32]) -> Self {
        Self(*value)
    }
}

impl TryFrom<&[u8]> for Blake3Hash {
    type Error = HashParseError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let arr: [u8; 32] = value
            .try_into()
            .map_err(|_| HashParseError::InvalidLength(value.len()))?;
        Ok(Self(arr))
    }
}

impl FromStr for Blake3Hash {
    type Err = HashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 64 {
            return Err(HashParseError::InvalidLength(s.len()));
        }
        let mut out = [0u8; 32];
        #[expect(clippy::indexing_slicing, reason = "Varified 32 bytes")]
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hi = hex_nibble(chunk[0])?;
            let lo = hex_nibble(chunk[1])?;
            out[i] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }
}

impl Deref for Blake3Hash {
    type Target = [u8; 32];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

const fn hex_nibble(b: u8) -> Result<u8, HashParseError> {
    match b {
        b'0'..=b'9' => Ok(b.wrapping_sub(b'0')),
        b'a'..=b'f' => Ok(b.wrapping_sub(b'a').wrapping_add(10)),
        b'A'..=b'F' => Ok(b.wrapping_sub(b'A').wrapping_add(10)),
        _ => Err(HashParseError::InvalidHexChar(b)),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HashParseError {
    #[error("invalid hash length: {0} (expected 32 bytes / 64 hex chars)")]
    InvalidLength(usize),
    #[error("invalid hex character: {0:#04x}")]
    InvalidHexChar(u8),
}
