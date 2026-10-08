//! Deterministic hasher for ABI calculations.

/// Deterministic 64-bit FNV-1a hasher.
///
/// Guaranteed to yield identical results across platforms, architectures, and compiler runs.
#[derive(Debug, Clone)]
pub struct DeterministicHasher {
    state: u64,
}

impl Default for DeterministicHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl DeterministicHasher {
    pub const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    pub const PRIME: u64 = 0x100000001b3;

    pub fn new() -> Self {
        Self {
            state: Self::OFFSET_BASIS,
        }
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= byte as u64;
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    pub fn write_str(&mut self, s: &str) {
        self.write_bytes(s.as_bytes());
        self.write_bytes(&[0xFF]); // Separator
    }

    pub fn write_u8(&mut self, val: u8) {
        self.write_bytes(&[val]);
    }

    pub fn write_u64(&mut self, val: u64) {
        self.write_bytes(&val.to_le_bytes());
    }

    pub fn finish(&self) -> u64 {
        self.state
    }
}
