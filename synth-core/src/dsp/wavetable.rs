//! Wavetable oscillator data and lookup kernel.
//!
//! This module owns no global bank and performs no platform selection. A host,
//! benchmark, or future wavetable engine prepares immutable samples and passes
//! a [`WavetableBank`] to the oscillator explicitly. Consequently, retaining
//! this code does not add table data to the analog/BLEP production firmware.

use crate::{LANES, TAU, f32x4};

pub const WAVETABLE_MIP_LEVELS: usize = 12;
pub const WAVETABLE_HARMONIC_LIMITS: [u16; WAVETABLE_MIP_LEVELS] =
    [4095, 2047, 1023, 511, 255, 127, 63, 31, 15, 7, 3, 1];
pub const WAVETABLE_LENGTHS: [usize; WAVETABLE_MIP_LEVELS] =
    [8192, 4096, 2048, 1024, 512, 256, 256, 256, 256, 128, 64, 64];
pub const WAVETABLE_OFFSETS: [usize; WAVETABLE_MIP_LEVELS] = [
    0, 8192, 12288, 14336, 15360, 15872, 16128, 16384, 16640, 16896, 17024, 17088,
];
pub const WAVETABLE_WAVE_SAMPLES: usize = 17_152;
pub const WAVETABLE_BANK_SAMPLES: usize = WAVETABLE_WAVE_SAMPLES * 2;
const CROSSFADE_SAMPLES: u8 = 8;

/// Failure while validating or generating an immutable wavetable bank.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WavetableBankError {
    WrongSampleCount { expected: usize, actual: usize },
    NonFinite,
}

/// Stable diagnostics for a prepared bank.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WavetableBankReport {
    pub samples: u32,
    pub bytes: u32,
    pub checksum: u32,
}

/// An immutable, validated `f32` saw/triangle mip bank.
///
/// The caller owns storage and may place it in compiled read-only data, D2 RAM,
/// or another nonblocking memory region. The samples must remain alive for the
/// duration of every oscillator using the bank.
#[derive(Clone, Copy)]
pub struct WavetableBank {
    samples: &'static [f32],
}

impl WavetableBank {
    pub fn new(samples: &'static [f32]) -> Result<Self, WavetableBankError> {
        if samples.len() != WAVETABLE_BANK_SAMPLES {
            return Err(WavetableBankError::WrongSampleCount {
                expected: WAVETABLE_BANK_SAMPLES,
                actual: samples.len(),
            });
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(WavetableBankError::NonFinite);
        }
        Ok(Self { samples })
    }

    pub fn report(self) -> WavetableBankReport {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                self.samples.as_ptr().cast::<u8>(),
                core::mem::size_of_val(self.samples),
            )
        };
        WavetableBankReport {
            samples: self.samples.len() as u32,
            bytes: bytes.len() as u32,
            checksum: fnv1a(bytes),
        }
    }

    #[inline(always)]
    fn sample(self, waveform: WaveTable, level: usize, phase: f32) -> f32 {
        let length = WAVETABLE_LENGTHS[level];
        let wave_offset = if matches!(waveform, WaveTable::Saw) {
            0
        } else {
            WAVETABLE_WAVE_SAMPLES
        };
        let position = phase * length as f32;
        let unwrapped_index = position as usize;
        let index = unwrapped_index & (length - 1);
        let next = (index + 1) & (length - 1);
        let fraction = position - unwrapped_index as f32;
        let offset = wave_offset + WAVETABLE_OFFSETS[level];
        let first = self.samples[offset + index];
        let second = self.samples[offset + next];
        first + (second - first) * fraction
    }
}

/// Generates the retained prototype's saw/triangle mip bank into caller-owned
/// storage. This is suitable for offline tools and explicit boot-time research;
/// production audio must never invoke it.
pub fn generate_wavetable_bank(
    samples: &mut [f32],
) -> Result<WavetableBankReport, WavetableBankError> {
    if samples.len() != WAVETABLE_BANK_SAMPLES {
        return Err(WavetableBankError::WrongSampleCount {
            expected: WAVETABLE_BANK_SAMPLES,
            actual: samples.len(),
        });
    }
    samples.fill(0.0);
    for level in 0..WAVETABLE_MIP_LEVELS {
        let length = WAVETABLE_LENGTHS[level];
        let offset = WAVETABLE_OFFSETS[level];
        let limit = WAVETABLE_HARMONIC_LIMITS[level] as usize;
        let (saw_bank, triangle_bank) = samples.split_at_mut(WAVETABLE_WAVE_SAMPLES);
        let saw = &mut saw_bank[offset..offset + length];
        let triangle = &mut triangle_bank[offset..offset + length];
        for harmonic in 1..=limit {
            let angle = TAU * harmonic as f32 / length as f32;
            let (step_sin, step_cos) = sin_cos(angle);
            let (mut sin, mut cos) = (0.0_f32, 1.0_f32);
            let saw_gain = -2.0 / (core::f32::consts::PI * harmonic as f32);
            let triangle_gain = if harmonic & 1 == 1 {
                -8.0 / (core::f32::consts::PI
                    * core::f32::consts::PI
                    * (harmonic * harmonic) as f32)
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
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(WavetableBankError::NonFinite);
    }
    Ok(report_samples(samples))
}

fn sin_cos(angle: f32) -> (f32, f32) {
    #[cfg(feature = "embedded-math")]
    {
        ::micromath::F32Ext::sin_cos(angle)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        (libm::sinf(angle), libm::cosf(angle))
    }
}

fn report_samples(samples: &[f32]) -> WavetableBankReport {
    let bytes = unsafe {
        core::slice::from_raw_parts(
            samples.as_ptr().cast::<u8>(),
            core::mem::size_of_val(samples),
        )
    };
    WavetableBankReport {
        samples: samples.len() as u32,
        bytes: bytes.len() as u32,
        checksum: fnv1a(bytes),
    }
}

fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ *byte as u32).wrapping_mul(0x0100_0193)
    })
}

#[derive(Clone, Copy)]
enum WaveTable {
    Saw,
    Triangle,
}

/// Internal lookup state used by [`crate::dsp::WavetableOscillator`].
#[doc(hidden)]
pub struct WavetableOscillatorKernel {
    bank: WavetableBank,
    current: [u8; LANES],
    previous: [u8; LANES],
    fade_remaining: [u8; LANES],
    prepared_increment_bits: [u32; LANES],
}

impl WavetableOscillatorKernel {
    pub(crate) fn new(bank: WavetableBank) -> Self {
        Self {
            bank,
            current: [0; LANES],
            previous: [0; LANES],
            fade_remaining: [0; LANES],
            prepared_increment_bits: [u32::MAX; LANES],
        }
    }

    pub(crate) fn prepare(&mut self, phase_inc: f32x4) {
        for (lane, increment) in phase_inc.to_array().into_iter().enumerate() {
            let increment_bits = increment.to_bits();
            if increment_bits == self.prepared_increment_bits[lane] {
                continue;
            }
            self.prepared_increment_bits[lane] = increment_bits;
            let target = select_mip(increment);
            if target != self.current[lane] as usize {
                let dominant = if self.fade_remaining[lane] > CROSSFADE_SAMPLES / 2 {
                    self.previous[lane]
                } else {
                    self.current[lane]
                };
                // The richer old mip is unsafe during an upward pitch change.
                // Downward changes can crossfade from the older, leaner mip.
                self.previous[lane] = dominant.max(target as u8);
                self.current[lane] = target as u8;
                self.fade_remaining[lane] = CROSSFADE_SAMPLES;
            }
        }
    }

    pub(crate) fn finish(&mut self) {
        for remaining in &mut self.fade_remaining {
            *remaining = remaining.saturating_sub(1);
        }
    }

    pub(crate) fn saw(&self, phase: f32x4) -> f32x4 {
        self.sample(WaveTable::Saw, phase)
    }

    pub(crate) fn triangle(&self, phase: f32x4) -> f32x4 {
        self.sample(WaveTable::Triangle, phase)
    }

    fn sample(&self, waveform: WaveTable, phase: f32x4) -> f32x4 {
        let phase = phase.to_array();
        let mut output = [0.0; LANES];
        for lane in 0..LANES {
            let current = self
                .bank
                .sample(waveform, self.current[lane] as usize, phase[lane]);
            let remaining = self.fade_remaining[lane];
            output[lane] = if remaining == 0 {
                current
            } else {
                let previous =
                    self.bank
                        .sample(waveform, self.previous[lane] as usize, phase[lane]);
                let amount = (CROSSFADE_SAMPLES - remaining) as f32 / CROSSFADE_SAMPLES as f32;
                previous + (current - previous) * amount
            };
        }
        f32x4::new(output)
    }
}

fn select_mip(phase_inc: f32) -> usize {
    if !phase_inc.is_finite() || phase_inc <= 0.0 {
        return 0;
    }
    let safe_harmonics = (0.45 / phase_inc) as u32;
    let exponent =
        (31 - safe_harmonics.saturating_add(1).leading_zeros()).min(WAVETABLE_MIP_LEVELS as u32);
    (WAVETABLE_MIP_LEVELS - exponent as usize).min(WAVETABLE_MIP_LEVELS - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;

    fn reference_bank() -> WavetableBank {
        static BANK: std::sync::OnceLock<WavetableBank> = std::sync::OnceLock::new();
        *BANK.get_or_init(|| {
            let mut samples = std::vec![0.0; WAVETABLE_BANK_SAMPLES];
            let (saw_bank, triangle_bank) = samples.split_at_mut(WAVETABLE_WAVE_SAMPLES);
            for level in 0..WAVETABLE_MIP_LEVELS {
                let length = WAVETABLE_LENGTHS[level];
                let offset = WAVETABLE_OFFSETS[level];
                for index in 0..length {
                    let phase = index as f32 / length as f32;
                    saw_bank[offset + index] = phase * 2.0 - 1.0;
                    triangle_bank[offset + index] = if phase < 0.5 {
                        phase * 4.0 - 1.0
                    } else {
                        3.0 - phase * 4.0
                    };
                }
            }
            WavetableBank::new(std::boxed::Box::leak(samples.into_boxed_slice())).unwrap()
        })
    }

    #[test]
    fn table_layout_is_contiguous_and_complete() {
        for level in 0..WAVETABLE_MIP_LEVELS - 1 {
            assert_eq!(
                WAVETABLE_OFFSETS[level] + WAVETABLE_LENGTHS[level],
                WAVETABLE_OFFSETS[level + 1]
            );
        }
        assert_eq!(
            WAVETABLE_OFFSETS[WAVETABLE_MIP_LEVELS - 1]
                + WAVETABLE_LENGTHS[WAVETABLE_MIP_LEVELS - 1],
            WAVETABLE_WAVE_SAMPLES
        );
    }

    #[test]
    fn bank_validation_and_report_are_stable() {
        let bank = reference_bank();
        assert_eq!(bank.report().samples, WAVETABLE_BANK_SAMPLES as u32);
        assert_eq!(bank.report(), bank.report());
        assert!(matches!(
            WavetableBank::new(&[]),
            Err(WavetableBankError::WrongSampleCount { .. })
        ));
    }

    #[test]
    fn mip_selection_keeps_selected_harmonics_inside_guard() {
        for index in 1..100_000 {
            let increment = index as f32 * 0.499 / 100_000.0;
            let level = select_mip(increment);
            assert!(
                WAVETABLE_HARMONIC_LIMITS[level] as f32 * increment <= 0.45 + f32::EPSILON
                    || level == WAVETABLE_MIP_LEVELS - 1
            );
        }
    }

    #[test]
    fn constant_time_mip_selection_matches_ordered_search() {
        for index in 1..100_000 {
            let increment = index as f32 * 0.499 / 100_000.0;
            let safe_harmonics = (0.45 / increment) as u32;
            let reference = WAVETABLE_HARMONIC_LIMITS
                .iter()
                .position(|limit| u32::from(*limit) <= safe_harmonics)
                .unwrap_or(WAVETABLE_MIP_LEVELS - 1);
            assert_eq!(select_mip(increment), reference);
        }
    }

    #[test]
    fn interpolation_wrap_is_finite_and_periodic() {
        let bank = reference_bank();
        for waveform in [WaveTable::Saw, WaveTable::Triangle] {
            for level in 0..WAVETABLE_MIP_LEVELS {
                let below_wrap = bank.sample(waveform, level, 1.0 - f32::EPSILON);
                let at_wrap = bank.sample(waveform, level, 0.0);
                assert!(below_wrap.is_finite() && at_wrap.is_finite());
                assert!((at_wrap - bank.sample(waveform, level, 1.0)).abs() < f32::EPSILON);
            }
        }
    }

    #[test]
    fn mip_state_is_per_lane_and_crossfade_restarts_safely() {
        let mut kernel = WavetableOscillatorKernel::new(reference_bank());
        kernel.prepare(f32x4::new([0.000_1, 0.001, 0.01, 0.1]));
        assert!(
            kernel
                .current
                .windows(2)
                .all(|levels| levels[0] <= levels[1])
        );
        assert!(
            kernel
                .current
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                > 1
        );
        assert!(
            kernel
                .sample(WaveTable::Saw, f32x4::splat(0.237))
                .to_array()
                .into_iter()
                .all(f32::is_finite)
        );
        kernel.finish();
        kernel.prepare(f32x4::new([0.1, 0.01, 0.001, 0.000_1]));
        assert_eq!(kernel.fade_remaining, [CROSSFADE_SAMPLES; LANES]);
    }

    #[test]
    #[ignore = "full harmonic generation is intentionally an offline/boot benchmark"]
    fn runtime_generation_matches_host_reference() {
        let mut generated = std::vec![0.0; WAVETABLE_BANK_SAMPLES];
        let report = generate_wavetable_bank(&mut generated).unwrap();
        assert_eq!(report.samples, WAVETABLE_BANK_SAMPLES as u32);
        assert!(generated.iter().all(|sample| sample.is_finite()));
        assert!(generated.iter().any(|sample| sample.abs() > 0.5));
    }
}
