//! Explicit host-side generator for the retained wavetable prototype.

use std::{env, f64::consts::PI, fs, path::PathBuf};
use synth_core::wavetable::{WAVETABLE_HARMONIC_LIMITS, WAVETABLE_LENGTHS, WAVETABLE_WAVE_SAMPLES};

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/wavetable-prototype"));
    fs::create_dir_all(&output).expect("create output directory");

    let (saw, triangle) = generate();
    let mut f32_bytes = Vec::with_capacity((saw.len() + triangle.len()) * 4);
    let mut q15_bytes = Vec::with_capacity((saw.len() + triangle.len()) * 2);
    for sample in saw.iter().chain(&triangle) {
        f32_bytes.extend_from_slice(&sample.to_le_bytes());
        let quantized = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        q15_bytes.extend_from_slice(&quantized.to_le_bytes());
    }
    fs::write(output.join("wavetable-f32.bin"), f32_bytes).expect("write f32 bank");
    fs::write(output.join("wavetable-q15.bin"), q15_bytes).expect("write Q15 comparison bank");
}

fn generate() -> (Vec<f32>, Vec<f32>) {
    let mut saw_bank = Vec::with_capacity(WAVETABLE_WAVE_SAMPLES);
    let mut triangle_bank = Vec::with_capacity(WAVETABLE_WAVE_SAMPLES);
    for (&limit, &length) in WAVETABLE_HARMONIC_LIMITS.iter().zip(&WAVETABLE_LENGTHS) {
        let mut saw = vec![0.0_f64; length];
        let mut triangle = vec![0.0_f64; length];
        for harmonic in 1..=usize::from(limit) {
            let angle = 2.0 * PI * harmonic as f64 / length as f64;
            let (step_sin, step_cos) = angle.sin_cos();
            let (mut sin, mut cos) = (0.0_f64, 1.0_f64);
            let saw_gain = -2.0 / (PI * harmonic as f64);
            let triangle_gain = if harmonic & 1 == 1 {
                -8.0 / (PI * PI * (harmonic * harmonic) as f64)
            } else {
                0.0
            };
            for index in 0..length {
                saw[index] += saw_gain * sin;
                triangle[index] += triangle_gain * cos;
                let next_sin = sin * step_cos + cos * step_sin;
                cos = cos * step_cos - sin * step_sin;
                sin = next_sin;
            }
        }
        saw_bank.extend(saw.into_iter().map(|sample| sample as f32));
        triangle_bank.extend(triangle.into_iter().map(|sample| sample as f32));
    }
    (saw_bank, triangle_bank)
}
