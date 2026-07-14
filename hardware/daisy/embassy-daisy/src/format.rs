//! Board-independent audio block and sample-format helpers.
#![cfg_attr(not(target_arch = "arm"), allow(dead_code))]

pub const SAMPLE_RATE_HZ: u32 = 48_000;
pub const BLOCK_LENGTH: usize = 32;
pub const CHANNELS: usize = 2;
pub const BLOCK_SAMPLES: usize = BLOCK_LENGTH * CHANNELS;

pub type Frame = (f32, f32);
pub type Block = [Frame; BLOCK_LENGTH];

pub(crate) fn encode_block(block: &Block, words: &mut [u32; BLOCK_SAMPLES]) {
    for (frame, pair) in block.iter().zip(words.chunks_exact_mut(2)) {
        // The Seed SAI wiring presents the right channel first in memory.
        pair[0] = f32_to_i24(frame.1);
        pair[1] = f32_to_i24(frame.0);
    }
}

pub(crate) fn decode_block(words: &[u32; BLOCK_SAMPLES], block: &mut Block) {
    for (pair, frame) in words.chunks_exact(2).zip(block.iter_mut()) {
        *frame = (i24_to_f32(pair[1]), i24_to_f32(pair[0]));
    }
}

fn f32_to_i24(sample: f32) -> u32 {
    let scaled = (sample.clamp(-1.0, 1.0) * 8_388_607.0) as i32;
    scaled as u32
}

fn i24_to_f32(word: u32) -> f32 {
    let signed = ((word << 8) as i32) >> 8;
    signed as f32 / 8_388_608.0
}

#[cfg(test)]
mod tests {
    use super::{BLOCK_LENGTH, decode_block, encode_block};

    #[test]
    fn sample_conversion_preserves_channel_order_and_bounds() {
        let mut source = [(0.0, 0.0); BLOCK_LENGTH];
        source[0] = (-1.0, 1.0);
        source[1] = (0.25, -0.5);
        let mut words = [0; BLOCK_LENGTH * 2];
        encode_block(&source, &mut words);
        let mut decoded = [(0.0, 0.0); BLOCK_LENGTH];
        decode_block(&words, &mut decoded);
        assert!((decoded[0].0 + 1.0).abs() < 0.000_001);
        assert!((decoded[0].1 - 1.0).abs() < 0.000_001);
        assert!((decoded[1].0 - 0.25).abs() < 0.000_001);
        assert!((decoded[1].1 + 0.5).abs() < 0.000_001);
    }
}
