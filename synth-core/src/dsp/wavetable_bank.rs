//! External pitch-conditioned, multirate wavetable bank and scalar oscillator.

use crate::{
    dsp::{Waveform, WavetableSupportStatus},
    math::F32,
};

pub const WAVETABLE_WAVEFORMS: usize = 3;
pub const WAVETABLE_MIP_COUNT: usize = 33;
pub const WAVETABLE_MAX_HARMONIC: usize = 1023;
pub const WAVETABLE_MIP_HARMONIC_LIMITS: [u16; WAVETABLE_MIP_COUNT] = [
    1023, 860, 723, 607, 510, 428, 359, 301, 253, 212, 178, 149, 125, 105, 88, 73, 61, 51, 42, 35,
    29, 24, 20, 16, 13, 10, 8, 6, 5, 4, 3, 2, 1,
];

#[derive(Clone, Copy, Debug)]
pub struct WavetableProfile {
    pub id: &'static str,
    pub target_id: &'static str,
    pub manifest_sha256: &'static str,
    pub fnv1a32: u32,
    pub sample_count: usize,
    pub samples_per_waveform: usize,
    pub source_sample_rate_hz: f32,
    pub mip_harmonic_limits: &'static [u16],
    pub mip_table_lengths: &'static [u16],
    /// Per-waveform sample offsets for each mip. A mip contains all pitch
    /// tables before the next mip begins.
    pub mip_offsets: &'static [u32],
    /// Frozen v1 compatibility only. V2 generated profiles leave these zero.
    pub legacy_table_length: usize,
    pub legacy_reference_sample_rate_hz: f32,
    pub saw_hz: &'static [f32],
    pub triangle_hz: &'static [f32],
    pub pulse_hz: &'static [f32],
    pub saw_max_hz: f32,
    pub triangle_max_hz: f32,
    pub pulse_max_hz: f32,
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
        assert!(samples.len() == profile.sample_count);
        assert!(profile.sample_count == WAVETABLE_WAVEFORMS * profile.samples_per_waveform);
        Self { samples, profile }
    }

    pub fn new(
        samples: &'static [f32],
        profile: &'static WavetableProfile,
    ) -> Result<Self, WavetableBankError> {
        if !profile_is_valid(profile) {
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

    #[cfg(test)]
    pub(crate) fn new_unchecked_for_test(
        samples: &'static [f32],
        profile: &'static WavetableProfile,
    ) -> Self {
        assert_eq!(samples.len(), profile.sample_count);
        Self { samples, profile }
    }

    #[inline]
    fn sample(&self, waveform: usize, mip: usize, pitch: usize, phase: f32) -> f32 {
        let table_length = if self.profile.mip_table_lengths.is_empty() {
            self.profile.legacy_table_length
        } else {
            usize::from(self.profile.mip_table_lengths[mip])
        };
        let position = phase * table_length as f32;
        let unwrapped = position as usize;
        let index = unwrapped & (table_length - 1);
        let next = (index + 1) & (table_length - 1);
        let fraction = position - unwrapped as f32;
        let mip_offset = self.profile.mip_offsets.get(mip).copied().unwrap_or(0) as usize;
        let offset =
            waveform * self.profile.samples_per_waveform + mip_offset + pitch * table_length;
        let first = self.samples[offset + index];
        first + (self.samples[offset + next] - first) * fraction
    }
}

#[derive(Clone, Copy)]
struct MipSelection {
    richer: usize,
    leaner: usize,
    richer_amount: f32,
}

include!(concat!(env!("OUT_DIR"), "/wavetable_mip_lookup.rs"));

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
    mip: MipSelection,
    status: WavetableSupportStatus,
    measured_amount: f32,
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
            mip: MIP_SELECTION_LOOKUP[WAVETABLE_MAX_HARMONIC],
            status: WavetableSupportStatus::Measured,
            measured_amount: 1.0,
            #[cfg(test)]
            pitch_grid_refreshes: 0,
        };
        result.refresh_frequency_state(true);
        result
    }

    pub(crate) fn reset(&mut self, reset_phase: bool) {
        if reset_phase {
            self.phase = 0.0;
        }
    }

    pub(crate) fn set_bank(&mut self, bank: WavetableBank) {
        self.bank = bank;
        self.refresh_frequency_state(true);
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

    pub(crate) fn measured_amount(&self) -> f32 {
        self.measured_amount
    }

    pub(crate) fn set_waveform_live(&mut self, waveform: Waveform) -> bool {
        if waveform == Waveform::SawTri {
            return false;
        }
        self.waveform = waveform;
        self.refresh_frequency_state(true);
        true
    }

    pub(crate) fn set_frequency_live(&mut self, frequency_hz: f32) -> WavetableSupportStatus {
        self.set_frequency_live_control_rate(frequency_hz, true)
    }

    pub(crate) fn set_frequency_live_control_rate(
        &mut self,
        frequency_hz: f32,
        refresh_pitch_grid: bool,
    ) -> WavetableSupportStatus {
        self.frequency_hz = frequency_hz;
        self.phase_increment = frequency_hz / self.sample_rate_hz;
        self.refresh_frequency_state(refresh_pitch_grid);
        self.status
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

    pub(crate) fn sample_at_phase(&self, phase: f32) -> f32 {
        let waveform = match self.waveform {
            Waveform::Saw => 0,
            Waveform::Triangle => 1,
            Waveform::Pulse => 2,
            Waveform::SawTri => unreachable!(),
        };
        let phase = phase - F32(phase).floor().as_f32();
        let richer = self.sample_pitch_pair(waveform, self.mip.richer, phase);
        if self.mip.richer == self.mip.leaner {
            richer
        } else {
            let leaner = self.sample_pitch_pair(waveform, self.mip.leaner, phase);
            leaner + (richer - leaner) * self.mip.richer_amount
        }
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

    #[cfg(test)]
    fn selected_harmonic_for_test(&self) -> u16 {
        self.bank.profile.mip_harmonic_limits[self.mip.richer]
    }

    fn sample_at_phase_offset(&self, offset: f32) -> f32 {
        self.sample_at_phase(self.phase + offset)
    }

    fn sample_pitch_pair(&self, waveform: usize, mip: usize, phase: f32) -> f32 {
        let lower = self.bank.sample(waveform, mip, self.lower_pitch, phase);
        if self.lower_pitch == self.upper_pitch {
            lower
        } else {
            let upper = self.bank.sample(waveform, mip, self.upper_pitch, phase);
            lower + (upper - lower) * self.pitch_amount
        }
    }

    fn advance_phase(&mut self) {
        if !self.phase_increment.is_finite() {
            return;
        }
        self.phase += self.phase_increment;
        self.phase -= F32(self.phase).floor().as_f32();
    }

    fn refresh_frequency_state(&mut self, refresh_pitch_grid: bool) {
        self.status = self.calculate_support_status();
        self.measured_amount = match self.status {
            WavetableSupportStatus::Measured => 1.0,
            WavetableSupportStatus::TransitionToFallback => {
                let maximum = self.maximum_frequency();
                1.0 - 12.0 * libm::log2f(self.frequency_hz / maximum)
            }
            _ => 0.0,
        }
        .clamp(0.0, 1.0);
        if self.frequency_hz.is_finite() && self.frequency_hz > 0.0 {
            if !self.bank.profile.mip_harmonic_limits.is_empty() {
                let safe_harmonics =
                    F32(0.45 / self.phase_increment).floor().as_f32().max(0.0) as usize;
                self.mip = MIP_SELECTION_LOOKUP[safe_harmonics.min(WAVETABLE_MAX_HARMONIC)];
            } else {
                self.mip = MipSelection {
                    richer: 0,
                    leaner: 0,
                    richer_amount: 1.0,
                };
            }
            if refresh_pitch_grid {
                self.refresh_pitch_position();
            }
        }
    }

    fn calculate_support_status(&self) -> WavetableSupportStatus {
        if !self.sample_rate_hz.is_finite()
            || self.sample_rate_hz <= 0.0
            || !self.frequency_hz.is_finite()
            || self.frequency_hz <= 0.0
            || !self.phase_increment.is_finite()
        {
            return WavetableSupportStatus::InvalidFrequency;
        }
        if self.bank.profile.mip_harmonic_limits.is_empty()
            && self.sample_rate_hz < self.bank.profile.legacy_reference_sample_rate_hz * 0.90
        {
            return WavetableSupportStatus::UnsupportedPlaybackRate;
        }
        if self.phase_increment > 0.45 {
            return WavetableSupportStatus::FundamentalAboveNyquistGuard;
        }
        let maximum = self.maximum_frequency();
        if self.frequency_hz <= maximum {
            WavetableSupportStatus::Measured
        } else if self.frequency_hz < maximum * libm::exp2f(1.0 / 12.0) {
            WavetableSupportStatus::TransitionToFallback
        } else {
            WavetableSupportStatus::AboveCapturedRange
        }
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

    fn maximum_frequency(&self) -> f32 {
        match self.waveform {
            Waveform::Saw => self.bank.profile.saw_max_hz,
            Waveform::Triangle => self.bank.profile.triangle_max_hz,
            Waveform::Pulse => self.bank.profile.pulse_max_hz,
            Waveform::SawTri => 0.0,
        }
    }
}

fn profile_is_valid(profile: &WavetableProfile) -> bool {
    let pitch_count = profile.saw_hz.len();
    if pitch_count == 0
        || pitch_count != profile.triangle_hz.len()
        || pitch_count != profile.pulse_hz.len()
        || profile.sample_count != WAVETABLE_WAVEFORMS * profile.samples_per_waveform
        || !profile.source_sample_rate_hz.is_finite()
        || profile.source_sample_rate_hz <= 0.0
    {
        return false;
    }
    if profile.mip_harmonic_limits.is_empty()
        && profile.mip_table_lengths.is_empty()
        && profile.mip_offsets.is_empty()
    {
        return profile.legacy_table_length.is_power_of_two()
            && profile.legacy_table_length >= 64
            && profile.legacy_reference_sample_rate_hz.is_finite()
            && profile.legacy_reference_sample_rate_hz > 0.0
            && profile.samples_per_waveform == pitch_count * profile.legacy_table_length;
    }
    if profile.mip_harmonic_limits.len() != WAVETABLE_MIP_COUNT
        || profile.mip_table_lengths.len() != WAVETABLE_MIP_COUNT
        || profile.mip_offsets.len() != WAVETABLE_MIP_COUNT
        || profile.legacy_table_length != 0
        || profile.legacy_reference_sample_rate_hz != 0.0
    {
        return false;
    }
    let mut expected_offset = 0usize;
    let mut previous_limit = u16::MAX;
    for mip in 0..WAVETABLE_MIP_COUNT {
        let limit = profile.mip_harmonic_limits[mip];
        let length = usize::from(profile.mip_table_lengths[mip]);
        if limit != WAVETABLE_MIP_HARMONIC_LIMITS[mip]
            || limit >= previous_limit
            || !length.is_power_of_two()
            || length < 64
            || length != (2 * (usize::from(limit) + 1)).next_power_of_two().max(64)
            || profile.mip_offsets[mip] as usize != expected_offset
        {
            return false;
        }
        expected_offset += pitch_count * length;
        previous_limit = limit;
    }
    expected_offset == profile.samples_per_waveform
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
    extern crate std;

    use std::{boxed::Box, vec};

    use crate::dsp::Waveform;

    use super::*;

    const PITCHES: [f32; 2] = [110.0, 220.0];
    const LIMITS: [u16; WAVETABLE_MIP_COUNT] = [
        1023, 860, 723, 607, 510, 428, 359, 301, 253, 212, 178, 149, 125, 105, 88, 73, 61, 51, 42,
        35, 29, 24, 20, 16, 13, 10, 8, 6, 5, 4, 3, 2, 1,
    ];
    const LENGTHS: [u16; WAVETABLE_MIP_COUNT] = [
        2048, 2048, 2048, 2048, 1024, 1024, 1024, 1024, 512, 512, 512, 512, 256, 256, 256, 256,
        128, 128, 128, 128, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64,
    ];
    const OFFSETS: [u32; WAVETABLE_MIP_COUNT] = mip_offsets();
    const SAMPLES_PER_WAVEFORM: usize = samples_per_waveform();
    static PROFILE: WavetableProfile = WavetableProfile {
        id: "test-v2",
        target_id: "test",
        manifest_sha256: "",
        fnv1a32: 0,
        sample_count: WAVETABLE_WAVEFORMS * SAMPLES_PER_WAVEFORM,
        samples_per_waveform: SAMPLES_PER_WAVEFORM,
        source_sample_rate_hz: 96_000.0,
        mip_harmonic_limits: &LIMITS,
        mip_table_lengths: &LENGTHS,
        mip_offsets: &OFFSETS,
        legacy_table_length: 0,
        legacy_reference_sample_rate_hz: 0.0,
        saw_hz: &PITCHES,
        triangle_hz: &PITCHES,
        pulse_hz: &PITCHES,
        saw_max_hz: 220.0,
        triangle_max_hz: 220.0,
        pulse_max_hz: 220.0,
    };

    #[test]
    fn profile_layout_is_validated_before_samples() {
        assert!(matches!(
            WavetableBank::new(&[], &PROFILE),
            Err(WavetableBankError::WrongSampleCount { .. })
        ));
    }

    #[test]
    fn mip_selection_never_exceeds_the_guard() {
        let samples = Box::leak(vec![0.0; PROFILE.sample_count].into_boxed_slice());
        let bank = WavetableBank::new_unchecked_for_test(samples, &PROFILE);
        let mut oscillator = WavetableTableOscillator::new(bank, 48_000.0);
        oscillator.set_waveform_live(Waveform::Saw);
        for midi in 0..=127 {
            let frequency = 440.0 * libm::exp2f((midi as f32 - 69.0) / 12.0);
            oscillator.set_frequency_live(frequency);
            if oscillator.status.uses_measured() {
                assert!(
                    f32::from(oscillator.selected_harmonic_for_test()) * frequency
                        <= 0.45 * 48_000.0
                );
            }
        }
    }

    #[test]
    fn upper_boundary_crossfades_for_exactly_one_semitone() {
        let samples = Box::leak(vec![0.0; PROFILE.sample_count].into_boxed_slice());
        let bank = WavetableBank::new_unchecked_for_test(samples, &PROFILE);
        let mut oscillator = WavetableTableOscillator::new(bank, 48_000.0);
        assert_eq!(
            oscillator.set_frequency_live(220.0),
            WavetableSupportStatus::Measured
        );
        assert_eq!(
            oscillator.set_frequency_live(220.0 * libm::exp2f(0.5 / 12.0)),
            WavetableSupportStatus::TransitionToFallback
        );
        assert!((oscillator.measured_amount() - 0.5).abs() < 1.0e-5);
        assert_eq!(
            oscillator.set_frequency_live(220.0 * libm::exp2f(1.0 / 12.0)),
            WavetableSupportStatus::AboveCapturedRange
        );
    }

    #[test]
    fn invalid_frequency_and_unguarded_fundamental_have_explicit_statuses() {
        let samples = Box::leak(vec![0.0; PROFILE.sample_count].into_boxed_slice());
        let bank = WavetableBank::new_unchecked_for_test(samples, &PROFILE);
        let mut oscillator = WavetableTableOscillator::new(bank, 48_000.0);
        assert_eq!(
            oscillator.set_frequency_live(22_000.0),
            WavetableSupportStatus::FundamentalAboveNyquistGuard
        );
        assert_eq!(
            oscillator.set_frequency_live(0.0),
            WavetableSupportStatus::InvalidFrequency
        );
    }

    #[test]
    fn every_mip_boundary_keeps_shape_pwm_and_sync_sampling_finite() {
        let samples = Box::leak(vec![0.0; PROFILE.sample_count].into_boxed_slice());
        let bank = WavetableBank::new_unchecked_for_test(samples, &PROFILE);
        let mut oscillator = WavetableTableOscillator::new(bank, 48_000.0);
        for limit in WAVETABLE_MIP_HARMONIC_LIMITS {
            let frequency = 0.45 * 48_000.0 / f32::from(limit);
            oscillator.set_frequency_live(frequency);
            oscillator.hard_sync_reset(0.37);
            assert!(oscillator.next_shaped_sample(0.63).is_finite());
            let (pwm, half) = oscillator.next_pwm_from_saw(0.77);
            assert!(pwm.is_finite() && half.is_finite());
            assert!(oscillator.sample_at_phase(0.91).is_finite());
        }
    }

    const fn mip_offsets() -> [u32; WAVETABLE_MIP_COUNT] {
        let mut result = [0; WAVETABLE_MIP_COUNT];
        let mut offset = 0;
        let mut index = 0;
        while index < WAVETABLE_MIP_COUNT {
            result[index] = offset;
            offset += PITCHES.len() as u32 * LENGTHS[index] as u32;
            index += 1;
        }
        result
    }

    const fn samples_per_waveform() -> usize {
        let mut result = 0;
        let mut index = 0;
        while index < WAVETABLE_MIP_COUNT {
            result += PITCHES.len() * LENGTHS[index] as usize;
            index += 1;
        }
        result
    }
}
