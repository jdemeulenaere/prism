//! Deterministic 256-bit BLAKE3 hashing for ABI and cache keys (D1 of `abi-cache`).

use std::fmt;

/// A 256-bit BLAKE3 hash: an ABI, interface or Monomorphization Hash, or a cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    /// The BLAKE3 hash of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// The hash as 64 lowercase hexadecimal digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{:02x}", byte)).collect()
    }

    /// Parses 64 hexadecimal digits, as [`ContentHash::to_hex`] writes them.
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 64 || !text.is_ascii() {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[2 * index..2 * index + 2], 16).ok()?;
        }
        Some(Self(bytes))
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// A deterministic hasher over a stream of tags, lengths and strings, producing a BLAKE3
/// [`ContentHash`]. Identical across platforms, architectures and compiler runs.
#[derive(Debug, Clone, Default)]
pub struct DeterministicHasher {
    state: blake3::Hasher,
}

impl DeterministicHasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.state.update(bytes);
    }

    /// Writes `s` followed by a `0xFF` separator, a byte that UTF-8 text never contains.
    pub fn write_str(&mut self, s: &str) {
        self.write_bytes(s.as_bytes());
        self.write_bytes(&[0xFF]);
    }

    pub fn write_u8(&mut self, val: u8) {
        self.write_bytes(&[val]);
    }

    pub fn write_u64(&mut self, val: u64) {
        self.write_bytes(&val.to_le_bytes());
    }

    pub fn write_hash(&mut self, hash: &ContentHash) {
        self.write_bytes(&hash.0);
    }

    pub fn finish(&self) -> ContentHash {
        ContentHash(*self.state.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let hash = ContentHash::of(b"prism");
        assert_eq!(hash.to_hex().len(), 64);
        assert_eq!(ContentHash::from_hex(&hash.to_hex()), Some(hash));
        assert_eq!(ContentHash::from_hex("zz"), None);
        assert_eq!(ContentHash::from_hex(&"g".repeat(64)), None);
    }

    #[test]
    fn hasher_matches_blake3_of_its_stream() {
        let mut hasher = DeterministicHasher::new();
        hasher.write_str("ab");
        hasher.write_u8(7);
        assert_eq!(hasher.finish(), ContentHash::of(&[b'a', b'b', 0xFF, 7]));
    }
}
