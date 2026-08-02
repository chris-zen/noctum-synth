//! External pitch-conditioned wavetable bank and scalar research oscillator.

use super::Waveform;
use crate::math::F32;

pub const WAVETABLE_WAVEFORMS: usize = 3;

#[derive(Clone, Copy, Debug)]
pub struct WavetableProfile {
    pub id: &'static str,
    pub target_id: &'static str,
    pub manifest_sha256: &'static str,
    pub fnv1a32: u32,
    pub sample_count: usize,
    pub table_length: usize,
    /// Playback sample rate used when the bank's harmonic limits were built.
    pub reference_sample_rate_hz: f32,
    pub saw_hz: &'static [f32],
    pub triangle_hz: &'static [f32],
    pub pulse_hz: &'static [f32],
    pub saw_max_hz: f32,
    pub triangle_max_hz: f32,
    pub pulse_max_hz: f32,
}

impl WavetableProfile {
    pub fn supports_sample_rate(&self, sample_rate_hz: f32) -> bool {
        sample_rate_hz.is_finite() && sample_rate_hz >= self.reference_sample_rate_hz * 0.90
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WavetableBankError {
    WrongSampleCount { expected: usize, actual: usize },
    NonFinite,
    ChecksumMismatch { expected: u32, actual: u32 },
    InvalidProfile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WavetableBankReport {
    pub samples: u32,
    pub bytes: u32,
    pub checksum: u32,
}

#[derive(Clone, Copy)]
pub struct WavetableBank {
    samples: &'static [f32],
    profile: &'static WavetableProfile,
}

impl WavetableBank {
    pub(crate) const fn from_compiled(
        samples: &'static [f32],
        profile: &'static WavetableProfile,
    ) -> Self {
        assert!(profile.table_length.is_power_of_two());
        assert!(samples.len() == profile.sample_count);
        Self { samples, profile }
    }

    pub fn new(
        samples: &'static [f32],
        profile: &'static WavetableProfile,
    ) -> Result<Self, WavetableBankError> {
        if profile.table_length == 0
            || !profile.table_length.is_power_of_two()
            || !profile.reference_sample_rate_hz.is_finite()
            || profile.reference_sample_rate_hz <= 0.0
            || profile.saw_hz.is_empty()
            || profile.saw_hz.len() != profile.triangle_hz.len()
            || profile.saw_hz.len() != profile.pulse_hz.len()
            || profile.sample_count
                != WAVETABLE_WAVEFORMS * profile.saw_hz.len() * profile.table_length
        {
            return Err(WavetableBankError::InvalidProfile);
        }
        if samples.len() != profile.sample_count {
            return Err(WavetableBankError::WrongSampleCount {
                expected: profile.sample_count,
                actual: samples.len(),
            });
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(WavetableBankError::NonFinite);
        }
        let actual = fnv1a_samples(samples);
        if actual != profile.fnv1a32 {
            return Err(WavetableBankError::ChecksumMismatch {
                expected: profile.fnv1a32,
                actual,
            });
        }
        Ok(Self { samples, profile })
    }

    pub fn profile(self) -> &'static WavetableProfile {
        self.profile
    }

    pub fn report(self) -> WavetableBankReport {
        WavetableBankReport {
            samples: self.samples.len() as u32,
            bytes: core::mem::size_of_val(self.samples) as u32,
            checksum: fnv1a_samples(self.samples),
        }
    }

    /// The bank uses a 0.45 × reference-rate spectral guard. Playback below
    /// 0.90 × the build rate would move retained harmonics above Nyquist.
    pub fn supports_sample_rate(self, sample_rate_hz: f32) -> bool {
        self.profile.supports_sample_rate(sample_rate_hz)
    }

    #[cfg(test)]
    pub(crate) fn new_unchecked_for_test(
        samples: &'static [f32],
        profile: &'static WavetableProfile,
    ) -> Self {
        assert_eq!(samples.len(), profile.sample_count);
        Self { samples, profile }
    }

    #[inline]
    fn pitch_count(self) -> usize {
        self.profile.saw_hz.len()
    }

    #[inline]
    fn sample(self, waveform_index: usize, pitch_index: usize, phase: f32) -> f32 {
        let table_length = self.profile.table_length;
        let position = phase * table_length as f32;
        let unwrapped = position as usize;
        let index = unwrapped & (table_length - 1);
        let next = (index + 1) & (table_length - 1);
        let fraction = position - unwrapped as f32;
        let offset = (waveform_index * self.pitch_count() + pitch_index) * table_length;
        let first = self.samples[offset + index];
        first + (self.samples[offset + next] - first) * fraction
    }
}

pub(crate) struct WavetableTableOscillator {
    bank: WavetableBank,
    sample_rate_hz: f32,
    frequency_hz: f32,
    phase_increment: f32,
    phase: f32,
    waveform: Waveform,
    lower_pitch: usize,
    upper_pitch: usize,
    pitch_amount: f32,
    sample_rate_supported: bool,
    #[cfg(test)]
    pitch_grid_refreshes: usize,
}

impl WavetableTableOscillator {
    pub(crate) fn new(bank: WavetableBank, sample_rate_hz: f32) -> Self {
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
            sample_rate_supported: bank.supports_sample_rate(sample_rate_hz),
            #[cfg(test)]
            pitch_grid_refreshes: 0,
        };
        result.refresh_pitch_position();
        result
    }

    pub(crate) fn reset(&mut self, reset_phase: bool) {
        if reset_phase {
            self.phase = 0.0;
        }
    }

    pub(crate) fn set_bank(&mut self, bank: WavetableBank) {
        self.bank = bank;
        self.sample_rate_supported = bank.supports_sample_rate(self.sample_rate_hz);
        self.refresh_pitch_position();
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
        if waveform == Waveform::SawTri || !self.sample_rate_supported {
            return false;
        }
        self.waveform = waveform;
        self.refresh_pitch_position();
        self.frequency_hz <= self.maximum_frequency(waveform)
    }

    pub(crate) fn set_frequency_live(&mut self, frequency_hz: f32) -> bool {
        self.set_frequency_live_control_rate(frequency_hz, true)
    }

    pub(crate) fn set_frequency_live_control_rate(
        &mut self,
        frequency_hz: f32,
        refresh_pitch_grid: bool,
    ) -> bool {
        if !self.sample_rate_supported || !frequency_hz.is_finite() || frequency_hz <= 0.0 {
            return false;
        }
        self.frequency_hz = frequency_hz;
        self.phase_increment = frequency_hz / self.sample_rate_hz;
        if refresh_pitch_grid {
            self.refresh_pitch_position();
        }
        frequency_hz <= self.maximum_frequency(self.waveform)
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

    #[cfg(test)]
    pub(crate) fn pitch_grid_refreshes_for_test(&self) -> usize {
        self.pitch_grid_refreshes
    }

    #[cfg(test)]
    pub(crate) fn frequency_hz_for_test(&self) -> f32 {
        self.frequency_hz
    }

    fn refresh_pitch_position(&mut self) {
        #[cfg(test)]
        {
            self.pitch_grid_refreshes += 1;
        }
        let frequencies = match self.waveform {
            Waveform::Saw => self.bank.profile.saw_hz,
            Waveform::Triangle => self.bank.profile.triangle_hz,
            Waveform::Pulse => self.bank.profile.pulse_hz,
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

    fn maximum_frequency(&self, waveform: Waveform) -> f32 {
        match waveform {
            Waveform::Saw => self.bank.profile.saw_max_hz,
            Waveform::Triangle => self.bank.profile.triangle_max_hz,
            Waveform::Pulse => self.bank.profile.pulse_max_hz,
            Waveform::SawTri => 0.0,
        }
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
    use crate::dsp::wavetable_bank_profile::MONOLOGUE_WAVETABLE_BANK_PROFILE;
    use crate::dsp::wavetable_bank_profile_prophet5::PROPHET5_WAVETABLE_BANK_PROFILE;

    static ONE_PITCH: [f32; 1] = [220.0];
    static NON_POWER_OF_TWO_PROFILE: WavetableProfile = WavetableProfile {
        id: "invalid-table-length",
        target_id: "test",
        manifest_sha256: "",
        fnv1a32: 0,
        sample_count: WAVETABLE_WAVEFORMS * 3,
        table_length: 3,
        reference_sample_rate_hz: 48_000.0,
        saw_hz: &ONE_PITCH,
        triangle_hz: &ONE_PITCH,
        pulse_hz: &ONE_PITCH,
        saw_max_hz: 220.0,
        triangle_max_hz: 220.0,
        pulse_max_hz: 220.0,
    };

    #[test]
    fn non_power_of_two_table_lengths_are_rejected() {
        assert!(matches!(
            WavetableBank::new(&[], &NON_POWER_OF_TWO_PROFILE),
            Err(WavetableBankError::InvalidProfile)
        ));
    }

    #[test]
    fn invalid_bank_sizes_are_rejected_without_reading_samples() {
        assert!(matches!(
            WavetableBank::new(&[], &MONOLOGUE_WAVETABLE_BANK_PROFILE),
            Err(WavetableBankError::WrongSampleCount {
                expected: 221184,
                actual: 0,
            })
        ));
        assert!(matches!(
            WavetableBank::new(&[], &PROPHET5_WAVETABLE_BANK_PROFILE),
            Err(WavetableBankError::WrongSampleCount {
                expected: 227328,
                actual: 0,
            })
        ));
    }

    #[test]
    fn supported_frequency_limits_cover_the_measured_grid() {
        let mono = &MONOLOGUE_WAVETABLE_BANK_PROFILE;
        let arturia = &PROPHET5_WAVETABLE_BANK_PROFILE;
        assert!(mono.saw_max_hz > *mono.saw_hz.last().unwrap());
        assert!(mono.triangle_max_hz > *mono.triangle_hz.last().unwrap());
        assert!(mono.pulse_max_hz > *mono.pulse_hz.last().unwrap());
        assert!(arturia.saw_max_hz > *arturia.saw_hz.last().unwrap());
        assert!(arturia.triangle_max_hz > *arturia.triangle_hz.last().unwrap());
        assert!(arturia.pulse_max_hz > *arturia.pulse_hz.last().unwrap());
    }

    #[test]
    fn playback_rate_must_respect_the_bank_build_guard() {
        assert!(MONOLOGUE_WAVETABLE_BANK_PROFILE.supports_sample_rate(48_000.0));
        assert!(MONOLOGUE_WAVETABLE_BANK_PROFILE.supports_sample_rate(43_200.0));
        assert!(!MONOLOGUE_WAVETABLE_BANK_PROFILE.supports_sample_rate(43_199.0));

        assert!(!PROPHET5_WAVETABLE_BANK_PROFILE.supports_sample_rate(48_000.0));
        assert!(PROPHET5_WAVETABLE_BANK_PROFILE.supports_sample_rate(96_000.0));
    }
}
