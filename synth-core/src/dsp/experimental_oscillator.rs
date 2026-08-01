//! Desktop-only closed oscillator facade used for live research audition.

use super::analog_oscillator::pulse_width_from_shape;
use super::analog_oscillator::{AnalogOscillator, EngineOscillator, OscillatorStep};
use super::blep::{table_blep_post_step_correction_lane, table_points_per_side_lane};
use super::measured_wavetable::{MeasuredWavetableBank, MeasuredWavetableOscillator};
use super::{SawMethod, Waveform};
use crate::math::WideF32;
use crate::profiling::RenderContext;

const SYNC_BLEP_SAMPLES: usize = 4;

/// Oscillator implementations that can be selected in a research build.
///
/// This selection is intentionally separate from patches, MIDI, SysEx, and
/// firmware model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExperimentalOscillatorModel {
    /// The exact compile-time production oscillator.
    #[default]
    Baseline,
    /// Existing table-BLEP implementation through the runtime analysis kernel.
    TableBlep,
    /// Existing PolyBLEP/PolyBLAMP implementation through the runtime kernel.
    PolyBlep,
    /// Pitch-conditioned tables measured from the Korg Monologue dataset.
    MeasuredWavetable,
}

impl ExperimentalOscillatorModel {
    pub const ALL: [Self; 4] = [
        Self::Baseline,
        Self::TableBlep,
        Self::PolyBlep,
        Self::MeasuredWavetable,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Baseline => "baseline-v1",
            Self::TableBlep => "table-blep-v1",
            Self::PolyBlep => "polyblep-v1",
            Self::MeasuredWavetable => "korg-monologue-measured-wavetable-v1",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Baseline => "Production Baseline",
            Self::TableBlep => "Table BLEP",
            Self::PolyBlep => "PolyBLEP / PolyBLAMP",
            Self::MeasuredWavetable => "Measured Wavetable (Monologue)",
        }
    }

    pub const fn revision(self) -> u32 {
        1
    }

    pub const fn capabilities(self) -> ExperimentalOscillatorCapabilities {
        match self {
            Self::MeasuredWavetable => ExperimentalOscillatorCapabilities {
                saw: true,
                saw_triangle: true,
                triangle: true,
                pulse: true,
                shape: true,
                audio_rate_pwm: true,
                hard_sync: true,
                note_reset: true,
                slop: true,
                simd_lanes: false,
                real_time_safe: true,
            },
            _ => ExperimentalOscillatorCapabilities {
                saw: true,
                saw_triangle: true,
                triangle: true,
                pulse: true,
                shape: true,
                audio_rate_pwm: true,
                hard_sync: true,
                note_reset: true,
                slop: true,
                simd_lanes: true,
                real_time_safe: true,
            },
        }
    }
}

/// Common capabilities surfaced to the audition and analysis UIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExperimentalOscillatorCapabilities {
    pub saw: bool,
    pub saw_triangle: bool,
    pub triangle: bool,
    pub pulse: bool,
    pub shape: bool,
    pub audio_rate_pwm: bool,
    pub hard_sync: bool,
    pub note_reset: bool,
    pub slop: bool,
    pub simd_lanes: bool,
    pub real_time_safe: bool,
}

/// Closed dispatch used only when `experimental-oscillators` is enabled.
pub(crate) enum ExperimentalOscillatorSource {
    Baseline(EngineOscillator),
    TableBlep(AnalogOscillator),
    PolyBlep(AnalogOscillator),
    MeasuredWavetable(LiveMeasuredWavetable),
}

pub(crate) struct LiveMeasuredWavetable {
    baseline: EngineOscillator,
    measured: [MeasuredWavetableOscillator; WideF32::LANES],
    measured_secondary: [MeasuredWavetableOscillator; WideF32::LANES],
    measured_waveform_lanes: [bool; WideF32::LANES],
    measured_lanes: [bool; WideF32::LANES],
    sync_correction: [[f32; SYNC_BLEP_SAMPLES]; WideF32::LANES],
    sync_correction_index: usize,
    enabled: bool,
    slop_enabled: bool,
    waveform: Waveform,
    shape: f32,
}

impl LiveMeasuredWavetable {
    pub(crate) fn new(bank: MeasuredWavetableBank, sample_rate: f32) -> Self {
        Self {
            baseline: EngineOscillator::new_engine(sample_rate),
            measured: core::array::from_fn(|_| MeasuredWavetableOscillator::new(bank, sample_rate)),
            measured_secondary: core::array::from_fn(|_| {
                MeasuredWavetableOscillator::new(bank, sample_rate)
            }),
            measured_waveform_lanes: [true; WideF32::LANES],
            measured_lanes: [true; WideF32::LANES],
            sync_correction: [[0.0; SYNC_BLEP_SAMPLES]; WideF32::LANES],
            sync_correction_index: 0,
            enabled: true,
            slop_enabled: false,
            waveform: Waveform::Saw,
            shape: 0.0,
        }
    }

    pub(crate) fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
        self.baseline.set_waveform(waveform);
        self.clear_sync_corrections();
        for lane in 0..WideF32::LANES {
            let (primary, secondary) = match waveform {
                Waveform::SawTri => (Waveform::Saw, Waveform::Triangle),
                Waveform::Pulse => (Waveform::Pulse, Waveform::Saw),
                waveform => (waveform, waveform),
            };
            let primary_supported = self.measured[lane].set_waveform_live(primary);
            let secondary_supported = self.measured_secondary[lane].set_waveform_live(secondary);
            self.measured_waveform_lanes[lane] = primary_supported && secondary_supported;
        }
        self.update_measured_frequencies(self.baseline.frequency_hz());
    }

    pub(crate) fn set_frequency(&mut self, frequency: WideF32) {
        self.baseline.set_frequency(frequency);
        self.update_measured_frequencies(self.baseline.frequency_hz());
    }

    fn update_measured_frequencies(&mut self, frequency: WideF32) {
        for (lane, value) in frequency.to_array().into_iter().enumerate() {
            let primary_frequency = self.measured[lane].set_frequency_live(value);
            let secondary_frequency = self.measured_secondary[lane].set_frequency_live(value);
            self.measured_lanes[lane] =
                self.measured_waveform_lanes[lane] && primary_frequency && secondary_frequency;
        }
    }

    pub(crate) fn set_shape(&mut self, shape: f32) {
        self.shape = shape.clamp(0.0, 1.0);
        self.baseline.set_shape(shape);
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.baseline.set_enabled(enabled);
    }

    fn set_slop_amount(&mut self, amount: f32) {
        self.baseline.set_slop_amount(amount);
        self.slop_enabled = amount > 0.0;
        self.update_measured_frequencies(self.baseline.frequency_hz());
    }

    pub(crate) fn trigger_lane(&mut self, lane: usize, reset_phase: bool) {
        self.baseline.trigger_lane(lane, reset_phase);
        self.measured[lane].reset(reset_phase);
        self.measured_secondary[lane].reset(reset_phase);
        if reset_phase {
            self.sync_correction[lane] = [0.0; SYNC_BLEP_SAMPLES];
        }
    }

    pub(crate) fn hard_sync_reset(&mut self, reset: WideF32, subsample_offset: WideF32) {
        self.baseline.hard_sync_reset(reset, subsample_offset);
        let reset = reset.to_array();
        let offsets = subsample_offset.to_array();
        for lane in 0..WideF32::LANES {
            if reset[lane] != 0.0 {
                let offset = offsets[lane].clamp(0.0, 1.0);
                if self.measured_lanes[lane] {
                    let phase = self.measured[lane].phase();
                    let phase_increment = self.measured[lane].phase_increment();
                    let before_phase = phase + phase_increment * offset;
                    let before = self.measured_sample_at_phase(lane, before_phase);
                    let after = self.measured_sample_at_phase(lane, 0.0);
                    self.add_sync_correction(
                        lane,
                        after - before,
                        1.0 - offset,
                        table_points_per_side_lane(phase_increment),
                    );
                }
                self.measured[lane].hard_sync_reset(offset);
                self.measured_secondary[lane].hard_sync_reset(offset);
            }
        }
    }

    fn measured_sample_at_phase(&self, lane: usize, phase: f32) -> f32 {
        match self.waveform {
            Waveform::Saw | Waveform::Triangle => {
                let raw = self.measured[lane].sample_at_phase(phase);
                let shifted = self.measured[lane].sample_at_phase(phase + self.shape * 0.5);
                raw + (shifted - raw) * self.shape
            }
            Waveform::SawTri => {
                let saw = self.measured[lane].sample_at_phase(phase);
                let triangle = self.measured_secondary[lane].sample_at_phase(phase);
                saw + (triangle - saw) * self.shape
            }
            Waveform::Pulse => {
                let measured_pulse = self.measured[lane].sample_at_phase(phase);
                let width = pulse_width_from_shape(self.shape);
                let first = self.measured_secondary[lane].sample_at_phase(phase);
                let shifted = self.measured_secondary[lane].sample_at_phase(phase + width);
                let half_shifted = self.measured_secondary[lane].sample_at_phase(phase + 0.5);
                let pwm = shifted - first - (width * 2.0 - 1.0);
                let generated_half = half_shifted - first;
                pwm + (measured_pulse - generated_half) * (1.0 - self.shape)
            }
        }
    }

    fn add_sync_correction(
        &mut self,
        lane: usize,
        step: f32,
        samples_since_edge: f32,
        points: u32,
    ) {
        for tap in 0..SYNC_BLEP_SAMPLES {
            let correction =
                table_blep_post_step_correction_lane(samples_since_edge + tap as f32, step, points);
            let index = (self.sync_correction_index + tap) % SYNC_BLEP_SAMPLES;
            self.sync_correction[lane][index] += correction;
        }
    }

    fn take_sync_correction(&mut self, lane: usize) -> f32 {
        let correction = self.sync_correction[lane][self.sync_correction_index];
        self.sync_correction[lane][self.sync_correction_index] = 0.0;
        correction
    }

    fn clear_sync_corrections(&mut self) {
        self.sync_correction = [[0.0; SYNC_BLEP_SAMPLES]; WideF32::LANES];
        self.sync_correction_index = 0;
    }

    #[cfg(feature = "oscillator-research")]
    pub(crate) fn uses_measured_tables(&self) -> bool {
        self.measured_lanes.iter().all(|enabled| *enabled)
    }

    pub(crate) fn next(&mut self, context: &mut RenderContext<'_>) -> OscillatorStep {
        let mut step = self.baseline.next(context);
        if self.slop_enabled {
            self.update_measured_frequencies(self.baseline.frequency_hz());
        }
        let mut output = step.output.to_array();
        for lane in 0..WideF32::LANES {
            let sync_correction = self.take_sync_correction(lane);
            if self.measured_lanes[lane] {
                let measured = match self.waveform {
                    Waveform::Saw | Waveform::Triangle => {
                        let value = self.measured[lane].next_shaped_sample(self.shape);
                        self.measured_secondary[lane].advance_phase_only();
                        value
                    }
                    Waveform::SawTri => {
                        let saw = self.measured[lane].next_sample();
                        let triangle = self.measured_secondary[lane].next_sample();
                        saw + (triangle - saw) * self.shape
                    }
                    Waveform::Pulse => {
                        let measured_pulse = self.measured[lane].next_sample();
                        let width = pulse_width_from_shape(self.shape);
                        let (pwm, generated_half) =
                            self.measured_secondary[lane].next_pwm_from_saw(width);
                        let character_amount = 1.0 - self.shape;
                        pwm + (measured_pulse - generated_half) * character_amount
                    }
                };
                let measured = if sync_correction == 0.0 {
                    measured
                } else {
                    measured + sync_correction
                };
                if self.enabled {
                    output[lane] = measured;
                }
            } else {
                self.measured[lane].advance_phase_only();
                self.measured_secondary[lane].advance_phase_only();
            }
        }
        self.sync_correction_index = (self.sync_correction_index + 1) % SYNC_BLEP_SAMPLES;
        step.output = WideF32::new(output);
        step
    }
}

macro_rules! with_source_mut {
    ($source:expr, $oscillator:ident => $body:expr) => {
        match $source {
            ExperimentalOscillatorSource::Baseline($oscillator) => $body,
            ExperimentalOscillatorSource::TableBlep($oscillator)
            | ExperimentalOscillatorSource::PolyBlep($oscillator) => $body,
            ExperimentalOscillatorSource::MeasuredWavetable($oscillator) => $body,
        }
    };
}

impl ExperimentalOscillatorSource {
    pub(crate) fn new(
        model: ExperimentalOscillatorModel,
        sample_rate: f32,
        measured_bank: Option<MeasuredWavetableBank>,
    ) -> Self {
        match model {
            ExperimentalOscillatorModel::Baseline => {
                Self::Baseline(EngineOscillator::new_engine(sample_rate))
            }
            ExperimentalOscillatorModel::TableBlep => {
                let mut oscillator = AnalogOscillator::new(sample_rate);
                oscillator.set_saw_method(SawMethod::Blep);
                Self::TableBlep(oscillator)
            }
            ExperimentalOscillatorModel::PolyBlep => {
                let mut oscillator = AnalogOscillator::new(sample_rate);
                oscillator.set_saw_method(SawMethod::PolyBlep);
                Self::PolyBlep(oscillator)
            }
            ExperimentalOscillatorModel::MeasuredWavetable => {
                Self::MeasuredWavetable(LiveMeasuredWavetable::new(
                    measured_bank.expect("validated measured bank must be installed"),
                    sample_rate,
                ))
            }
        }
    }

    pub(crate) const fn model(&self) -> ExperimentalOscillatorModel {
        match self {
            Self::Baseline(_) => ExperimentalOscillatorModel::Baseline,
            Self::TableBlep(_) => ExperimentalOscillatorModel::TableBlep,
            Self::PolyBlep(_) => ExperimentalOscillatorModel::PolyBlep,
            Self::MeasuredWavetable(_) => ExperimentalOscillatorModel::MeasuredWavetable,
        }
    }

    pub(crate) fn set_waveform(&mut self, waveform: Waveform) {
        with_source_mut!(self, oscillator => oscillator.set_waveform(waveform));
    }

    pub(crate) fn set_shape(&mut self, shape: f32) {
        with_source_mut!(self, oscillator => oscillator.set_shape(shape));
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        with_source_mut!(self, oscillator => oscillator.set_enabled(enabled));
    }

    pub(crate) fn set_frequency(&mut self, frequency: WideF32) {
        with_source_mut!(self, oscillator => oscillator.set_frequency(frequency));
    }

    pub(crate) fn frequency_hz(&self) -> WideF32 {
        match self {
            Self::Baseline(oscillator) => oscillator.frequency_hz(),
            Self::TableBlep(oscillator) | Self::PolyBlep(oscillator) => oscillator.frequency_hz(),
            Self::MeasuredWavetable(oscillator) => oscillator.baseline.frequency_hz(),
        }
    }

    pub(crate) fn set_slop_amount(&mut self, amount: f32) {
        with_source_mut!(self, oscillator => oscillator.set_slop_amount(amount));
    }

    pub(crate) fn trigger_lane(&mut self, lane: usize, reset_phase: bool) {
        with_source_mut!(self, oscillator => oscillator.trigger_lane(lane, reset_phase));
    }

    pub(crate) fn hard_sync_reset(&mut self, reset: WideF32, subsample_offset: WideF32) {
        with_source_mut!(self, oscillator => oscillator.hard_sync_reset(reset, subsample_offset));
    }

    pub(crate) fn next(&mut self, context: &mut RenderContext<'_>) -> OscillatorStep {
        match self {
            Self::Baseline(oscillator) => oscillator.next(context),
            Self::TableBlep(oscillator) | Self::PolyBlep(oscillator) => oscillator.next(context),
            Self::MeasuredWavetable(oscillator) => oscillator.next(context),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::boxed::Box;
    use std::vec;

    fn constant_measured_bank() -> MeasuredWavetableBank {
        MeasuredWavetableBank::new_unchecked_for_test(Box::leak(
            vec![0.25; super::super::measured_wavetable_profile::MEASURED_BANK_SAMPLE_COUNT]
                .into_boxed_slice(),
        ))
    }

    fn ramp_measured_bank() -> MeasuredWavetableBank {
        let mut samples =
            vec![0.0; super::super::measured_wavetable_profile::MEASURED_BANK_SAMPLE_COUNT];
        for (index, sample) in samples.iter_mut().enumerate() {
            let phase = (index
                % super::super::measured_wavetable_profile::MEASURED_BANK_TABLE_LENGTH)
                as f32
                / super::super::measured_wavetable_profile::MEASURED_BANK_TABLE_LENGTH as f32;
            *sample = phase * 2.0 - 1.0;
        }
        MeasuredWavetableBank::new_unchecked_for_test(Box::leak(samples.into_boxed_slice()))
    }

    #[test]
    fn baseline_facade_is_bit_identical_to_engine_oscillator() {
        for waveform in [
            Waveform::Saw,
            Waveform::SawTri,
            Waveform::Triangle,
            Waveform::Pulse,
        ] {
            let mut direct = EngineOscillator::new_engine(48_000.0);
            let mut facade = ExperimentalOscillatorSource::new(
                ExperimentalOscillatorModel::Baseline,
                48_000.0,
                None,
            );
            direct.set_waveform(waveform);
            direct.set_shape(0.37);
            direct.set_frequency(WideF32::new(core::array::from_fn(|lane| {
                110.0 * (lane + 1) as f32
            })));
            facade.set_waveform(waveform);
            facade.set_shape(0.37);
            facade.set_frequency(WideF32::new(core::array::from_fn(|lane| {
                110.0 * (lane + 1) as f32
            })));

            for sample in 0..4_096 {
                if sample == 1_024 {
                    direct.trigger_lane(2, true);
                    facade.trigger_lane(2, true);
                }
                let mut direct_context = crate::create_render_context!();
                let mut facade_context = crate::create_render_context!();
                let expected = direct.next(&mut direct_context);
                let actual = facade.next(&mut facade_context);
                assert_eq!(
                    actual.output.to_array().map(f32::to_bits),
                    expected.output.to_array().map(f32::to_bits),
                    "{waveform:?} diverged at sample {sample}"
                );
                assert_eq!(
                    actual.wrapped.to_array().map(f32::to_bits),
                    expected.wrapped.to_array().map(f32::to_bits)
                );
                assert_eq!(
                    actual.subsample_offset.to_array().map(f32::to_bits),
                    expected.subsample_offset.to_array().map(f32::to_bits)
                );
            }
        }
    }

    #[test]
    fn measured_live_source_renders_supported_lanes_and_falls_back_safely() {
        let mut source = ExperimentalOscillatorSource::new(
            ExperimentalOscillatorModel::MeasuredWavetable,
            48_000.0,
            Some(constant_measured_bank()),
        );
        source.set_waveform(Waveform::Triangle);
        source.set_frequency(WideF32::splat(220.0));
        let mut context = crate::create_render_context!();
        let measured = source.next(&mut context).output.to_array();
        assert!(measured.iter().all(|sample| *sample == 0.25));

        source.set_waveform(Waveform::Pulse);
        source.set_shape(0.0);
        let mut context = crate::create_render_context!();
        let neutral_pulse = source.next(&mut context).output.to_array();
        assert!(neutral_pulse.iter().all(|sample| *sample == 0.25));
        source.set_shape(1.0);
        let mut context = crate::create_render_context!();
        let narrow_pulse = source.next(&mut context).output.to_array();
        assert!(
            narrow_pulse
                .iter()
                .all(|sample| (*sample + 0.98).abs() < 1.0e-6)
        );

        source.set_waveform(Waveform::SawTri);
        let mut context = crate::create_render_context!();
        let measured_saw_tri = source.next(&mut context).output.to_array();
        assert!(measured_saw_tri.iter().all(|sample| *sample == 0.25));

        source.set_frequency(WideF32::splat(4_000.0));
        let mut context = crate::create_render_context!();
        let fallback = source.next(&mut context).output.to_array();
        assert!(fallback.iter().all(|sample| sample.is_finite()));
        assert!(fallback.iter().any(|sample| *sample != 0.25));
    }

    #[test]
    fn measured_live_source_applies_slop_to_the_audible_phase() {
        let bank = ramp_measured_bank();
        let mut stable = ExperimentalOscillatorSource::new(
            ExperimentalOscillatorModel::MeasuredWavetable,
            48_000.0,
            Some(bank),
        );
        let mut sloppy = ExperimentalOscillatorSource::new(
            ExperimentalOscillatorModel::MeasuredWavetable,
            48_000.0,
            Some(bank),
        );
        for source in [&mut stable, &mut sloppy] {
            source.set_waveform(Waveform::Saw);
            source.set_frequency(WideF32::splat(220.0));
        }
        sloppy.set_slop_amount(1.0);
        for lane in 0..WideF32::LANES {
            stable.trigger_lane(lane, true);
            sloppy.trigger_lane(lane, true);
        }
        assert!(
            stable
                .frequency_hz()
                .to_array()
                .iter()
                .zip(sloppy.frequency_hz().to_array())
                .any(|(stable, sloppy)| (stable - sloppy).abs() > 0.001)
        );

        let mut maximum_output_difference = 0.0_f32;
        for _ in 0..2_048 {
            let mut stable_context = crate::create_render_context!();
            let mut sloppy_context = crate::create_render_context!();
            let stable_output = stable.next(&mut stable_context).output.to_array();
            let sloppy_output = sloppy.next(&mut sloppy_context).output.to_array();
            for lane in 0..WideF32::LANES {
                maximum_output_difference = maximum_output_difference
                    .max((stable_output[lane] - sloppy_output[lane]).abs());
            }
        }
        assert!(
            maximum_output_difference > 1.0e-4,
            "slop did not reach the measured output: {maximum_output_difference}"
        );
    }
}
