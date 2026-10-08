//! Deterministic 128-bit hasher for ABI calculations.

/// Deterministic 128-bit FNV-1a hasher.
///
/// Guaranteed to yield identical results across platforms, architectures, and compiler runs.
/// The 128-bit state provides collision resistance with a birthday bound of ~2^64 items.
#[derive(Debug, Clone)]
pub struct DeterministicHasher {
    state: u128,
}

impl Default for DeterministicHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl DeterministicHasher {
    pub const OFFSET_BASIS: u128 = 0x6c62272e07bb014262b821756295c58d;
    pub const PRIME: u128 = 0x0000000001000000000000000000013b;

    pub fn new() -> Self {
        Self {
            state: Self::OFFSET_BASIS,
        }
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= byte as u128;
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    pub fn write_str(&mut self, s: &str) {
        self.write_bytes(s.as_bytes());
        self.write_bytes(&[0xFF]); // Separator tag
    }

    pub fn write_u8(&mut self, val: u8) {
        self.write_bytes(&[val]);
    }

    pub fn write_u64(&mut self, val: u64) {
        self.write_bytes(&val.to_le_bytes());
    }

    pub fn write_u128(&mut self, val: u128) {
        self.write_bytes(&val.to_le_bytes());
    }

    pub fn finish(&self) -> u128 {
        self.state
    }
}
