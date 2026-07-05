use wide::{f32x4, i32x4};

/// Scales a signed 32-bit lane to the bipolar `[-1, 1)` range.
const NOISE_SCALE: f32 = 2.0 / 0xffff_ffffu32 as f32;

/// A 4-lane (SIMD) white-noise generator.
///
/// Each lane runs an independent copy of the fast xorshift generator from
/// Will Pirkle's SynthLab `NoiseGenerator::doWhiteNoise`, seeded distinctly so
/// the lanes produce decorrelated noise.
pub struct WhiteNoise {
    x1: i32x4,
    x2: i32x4,
}

impl Default for WhiteNoise {
    fn default() -> Self {
        Self {
            x1: i32x4::new([
                0x67452301u32 as i32,
                0x98badcfeu32 as i32,
                0x70f4f854u32 as i32,
                0x1f83d9abu32 as i32,
            ]),
            x2: i32x4::new([
                0xefcdab89u32 as i32,
                0x10325476u32 as i32,
                0xe1e9f0a7u32 as i32,
                0x5be0cd19u32 as i32,
            ]),
        }
    }
}

impl WhiteNoise {
    /// Advances all lanes and returns 4 lanes of uniform white noise in
    /// `[-1, 1)`.
    pub fn next(&mut self) -> f32x4 {
        white_noise(&mut self.x1, &mut self.x2)
    }
}

/// Advances the per-lane xorshift state and returns the next bipolar white
/// sample for each lane.
///
/// Integer XOR/add wrap on overflow, as the generator requires.
fn white_noise(x1: &mut i32x4, x2: &mut i32x4) -> f32x4 {
    *x1 = *x1 ^ *x2;
    let output = x2.round_float() * f32x4::splat(NOISE_SCALE);
    *x2 = *x2 + *x1;
    output
}

/// Filters white noise into pink noise per lane using Paul Kellet's 3-pole
/// approximation, scaled to keep the output within `[-1, 1]`.
///
/// `bn` holds the three per-lane filter poles and persists across calls.
#[expect(dead_code)]
fn pink_filter(bn: &mut [f32x4; 3], white: f32x4) -> f32x4 {
    bn[0] = f32x4::splat(0.99765) * bn[0] + white * f32x4::splat(0.0990460);
    bn[1] = f32x4::splat(0.96300) * bn[1] + white * f32x4::splat(0.2965164);
    bn[2] = f32x4::splat(0.57000) * bn[2] + white * f32x4::splat(1.0526913);
    (bn[0] + bn[1] + bn[2] + white * f32x4::splat(0.1848)) * f32x4::splat(0.25)
}
