// Split from src/object/core.rs — FxHasher / FxBuildHasher.

#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}

const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl std::hash::Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut hash = self.hash;
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            let word = u64::from_ne_bytes(chunk.try_into().unwrap());
            hash = (hash.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rem.len()].copy_from_slice(rem);
            let word = u64::from_ne_bytes(buf);
            hash = (hash.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
        }
        self.hash = hash;
    }
    fn write_u8(&mut self, i: u8) { self.write_u64(i as u64); }
    fn write_u16(&mut self, i: u16) { self.write_u64(i as u64); }
    fn write_u32(&mut self, i: u32) { self.write_u64(i as u64); }
    fn write_u64(&mut self, i: u64) { self.hash = (self.hash.rotate_left(5) ^ i).wrapping_mul(FX_SEED); }
    fn write_usize(&mut self, i: usize) { self.write_u64(i as u64); }
    fn finish(&self) -> u64 { self.hash }
}

pub type FxBuildHasher = std::hash::BuildHasherDefault<FxHasher>;
