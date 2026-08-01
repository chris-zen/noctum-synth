//! External pitch-conditioned measured wavetable bank and scalar research oscillator.

use super::Waveform;
use super::measured_wavetable_profile::{
    MEASURED_BANK_FNV1A32, MEASURED_BANK_SAMPLE_COUNT, MEASURED_BANK_TABLE_LENGTH,
    MEASURED_PULSE_FREQUENCIES_HZ, MEASURED_PULSE_MAXIMUM_HZ, MEASURED_SAW_FREQUENCIES_HZ,
    MEASURED_SAW_MAXIMUM_HZ, MEASURED_TRIANGLE_FREQUENCIES_HZ, MEASURED_TRIANGLE_MAXIMUM_HZ,
};
use crate::math::F32;

pub const MEASURED_WAVETABLE_PITCHES: usize = 36;
pub const MEASURED_WAVETABLE_WAVEFORMS: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeasuredWavetableBankError {
    WrongSampleCount { expected: usize, actual: usize },
    NonFinite,
    ChecksumMismatch { expected: u32, actual: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeasuredWavetableBankReport {
    pub samples: u32,
    pub bytes: u32,
    pub checksum: u32,
}

#[derive(Clone, Copy)]
pub struct MeasuredWavetableBank {
    samples: &'static [f32],
}

impl MeasuredWavetableBank {
    pub fn new(samples: &'static [f32]) -> Result<Self, MeasuredWavetableBankError> {
        if samples.len() != MEASURED_BANK_SAMPLE_COUNT {
            return Err(MeasuredWavetableBankError::WrongSampleCount {
                expected: MEASURED_BANK_SAMPLE_COUNT,
                actual: samples.len(),
            });
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(MeasuredWavetableBankError::NonFinite);
        }
        let actual = fnv1a_samples(samples);
        if actual != MEASURED_BANK_FNV1A32 {
            return Err(MeasuredWavetableBankError::ChecksumMismatch {
                expected: MEASURED_BANK_FNV1A32,
                actual,
            });
        }
        Ok(Self { samples })
    }

    pub fn report(self) -> MeasuredWavetableBankReport {
        MeasuredWavetableBankReport {
            samples: self.samples.len() as u32,
            bytes: core::mem::size_of_val(self.samples) as u32,
            checksum: fnv1a_samples(self.samples),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_unchecked_for_test(samples: &'static [f32]) -> Self {
        assert_eq!(samples.len(), MEASURED_BANK_SAMPLE_COUNT);
        Self { samples }
    }

    #[inline]
    fn sample(self, waveform_index: usize, pitch_index: usize, phase: f32) -> f32 {
        let position = phase * MEASURED_BANK_TABLE_LENGTH as f32;
        let unwrapped = position as usize;
        let index = unwrapped & (MEASURED_BANK_TABLE_LENGTH - 1);
        let next = (index + 1) & (MEASURED_BANK_TABLE_LENGTH - 1);
        let fraction = position - unwrapped as f32;
        let offset = (waveform_index * MEASURED_WAVETABLE_PITCHES + pitch_index)
            * MEASURED_BANK_TABLE_LENGTH;
        let first = self.samples[offset + index];
        first + (self.samples[offset + next] - first) * fraction
    }
}

pub(crate) struct MeasuredWavetableOscillator {
    bank: MeasuredWavetableBank,
    sample_rate_hz: f32,
    frequency_hz: f32,
    phase_increment: f32,
    phase: f32,
    waveform: Waveform,
    lower_pitch: usize,
    upper_pitch: usize,
    pitch_amount: f32,
}

impl MeasuredWavetableOscillator {
    pub(crate) fn new(bank: MeasuredWavetableBank, sample_rate_hz: f32) -> Self {
        let mut result = Self {
            bank,
            sample_rate_hz,
            frequency_hz: 220.0,
            phase_increment: 220.0 / sample_rate_hz,
            phase: 0.0,
            waveform: Waveform::Saw,
            lower_pitch: 0,
            upper_pitch: 0,
            pitch_amount: 0.0,
        };
        result.refresh_pitch_position();
        result
    }

    pub(crate) fn reset(&mut self, reset_phase: bool) {
        if reset_phase {
            self.phase = 0.0;
        }
    }

    pub(crate) fn hard_sync_reset(&mut self, subsample_offset: f32) {
        let offset = subsample_offset.clamp(0.0, 1.0);
        self.phase = self.phase_increment * (1.0 - offset);
    }

    pub(crate) fn phase(&self) -> f32 {
        self.phase
    }

    pub(crate) fn phase_increment(&self) -> f32 {
        self.phase_increment
    }

    pub(crate) fn set_waveform_live(&mut self, waveform: Waveform) -> bool {
        if waveform == Waveform::SawTri {
            return false;
        }
        self.waveform = waveform;
        self.refresh_pitch_position();
        self.frequency_hz <= maximum_frequency(waveform)
    }

    pub(crate) fn set_frequency_live(&mut self, frequency_hz: f32) -> bool {
        if !frequency_hz.is_finite() || frequency_hz <= 0.0 {
            return false;
        }
        self.frequency_hz = frequency_hz;
        self.phase_increment = frequency_hz / self.sample_rate_hz;
        self.refresh_pitch_position();
        frequency_hz <= maximum_frequency(self.waveform)
    }

    pub(crate) fn next_sample(&mut self) -> f32 {
        let output = self.sample_at_phase_offset(0.0);
        self.advance_phase();
        output
    }

    pub(crate) fn next_shaped_sample(&mut self, shape: f32) -> f32 {
        let shape = shape.clamp(0.0, 1.0);
        let raw = self.sample_at_phase_offset(0.0);
        let shifted = self.sample_at_phase_offset(shape * 0.5);
        self.advance_phase();
        raw + (shifted - raw) * shape
    }

    /// Builds a variable-width pulse from two reads of the measured saw table.
    /// Returns both the requested width and its phase-aligned 50% reference.
    pub(crate) fn next_pwm_from_saw(&mut self, width: f32) -> (f32, f32) {
        let width = width.clamp(0.5, 0.99);
        let first = self.sample_at_phase_offset(0.0);
        let shifted = self.sample_at_phase_offset(width);
        let half_shifted = self.sample_at_phase_offset(0.5);
        self.advance_phase();
        (shifted - first - (width * 2.0 - 1.0), half_shifted - first)
    }

    fn sample_at_phase_offset(&self, offset: f32) -> f32 {
        self.sample_at_phase(self.phase + offset)
    }

    pub(crate) fn sample_at_phase(&self, phase: f32) -> f32 {
        let waveform_index = match self.waveform {
            Waveform::Saw => 0,
            Waveform::Triangle => 1,
            Waveform::Pulse => 2,
            Waveform::SawTri => unreachable!(),
        };
        let phase = phase - F32(phase).floor().as_f32();
        let lower = self.bank.sample(waveform_index, self.lower_pitch, phase);
        let output = if self.lower_pitch == self.upper_pitch {
            lower
        } else {
            let upper = self.bank.sample(waveform_index, self.upper_pitch, phase);
            lower + (upper - lower) * self.pitch_amount
        };
        output
    }

    fn advance_phase(&mut self) {
        self.phase += self.phase_increment;
        self.phase -= F32(self.phase).floor().as_f32();
    }

    pub(crate) fn advance_phase_only(&mut self) {
        self.advance_phase();
    }

    fn refresh_pitch_position(&mut self) {
        let frequencies = match self.waveform {
            Waveform::Saw => &MEASURED_SAW_FREQUENCIES_HZ,
            Waveform::Triangle => &MEASURED_TRIANGLE_FREQUENCIES_HZ,
            Waveform::Pulse => &MEASURED_PULSE_FREQUENCIES_HZ,
            Waveform::SawTri => return,
        };
        let coordinate = libm::log2f(self.frequency_hz);
        if self.frequency_hz <= frequencies[0] {
            (self.lower_pitch, self.upper_pitch, self.pitch_amount) = (0, 0, 0.0);
            return;
        }
        let last = frequencies.len() - 1;
        if self.frequency_hz >= frequencies[last] {
            (self.lower_pitch, self.upper_pitch, self.pitch_amount) = (last, last, 0.0);
            return;
        }
        let upper = frequencies
            .iter()
            .position(|frequency| *frequency >= self.frequency_hz)
            .unwrap_or(last);
        let lower = upper - 1;
        let lower_log = libm::log2f(frequencies[lower]);
        let upper_log = libm::log2f(frequencies[upper]);
        self.lower_pitch = lower;
        self.upper_pitch = upper;
        self.pitch_amount = (coordinate - lower_log) / (upper_log - lower_log);
    }
}

fn maximum_frequency(waveform: Waveform) -> f32 {
    match waveform {
        Waveform::Saw => MEASURED_SAW_MAXIMUM_HZ,
        Waveform::Triangle => MEASURED_TRIANGLE_MAXIMUM_HZ,
        Waveform::Pulse => MEASURED_PULSE_MAXIMUM_HZ,
        Waveform::SawTri => 0.0,
    }
}

fn fnv1a_samples(samples: &[f32]) -> u32 {
    samples.iter().fold(0x811c_9dc5, |hash, sample| {
        sample.to_le_bytes().iter().fold(hash, |hash, byte| {
            (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_bank_sizes_are_rejected_without_reading_samples() {
        assert!(matches!(
            MeasuredWavetableBank::new(&[]),
            Err(MeasuredWavetableBankError::WrongSampleCount {
                expected: MEASURED_BANK_SAMPLE_COUNT,
                actual: 0,
            })
        ));
    }

    #[test]
    fn supported_frequency_limits_cover_the_measured_grid() {
        assert!(MEASURED_SAW_MAXIMUM_HZ > MEASURED_SAW_FREQUENCIES_HZ[35]);
        assert!(MEASURED_TRIANGLE_MAXIMUM_HZ > MEASURED_TRIANGLE_FREQUENCIES_HZ[35]);
        assert!(MEASURED_PULSE_MAXIMUM_HZ > MEASURED_PULSE_FREQUENCIES_HZ[35]);
    }
}
