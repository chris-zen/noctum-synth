//! Pitch-conditioned measured wavetable oscillator used by the live engine and research tools.

use crate::{
    dsp::{
        Waveform,
        analog_oscillator::{EngineOscillator, OscillatorStep, pulse_width_from_shape},
        blep::{table_blep_post_step_correction_lane, table_points_per_side_lane},
        wavetable_bank::{WavetableBank, WavetableTableOscillator},
    },
    math::WideF32,
    profiling::RenderContext,
};

const SYNC_BLEP_SAMPLES: usize = 4;
/// Pitch-grid interpolation is control-rate work. Phase increments still track
/// oscillator slop every sample; only the logarithmic table lookup is decimated.
const PITCH_GRID_REFRESH_SAMPLES: u8 = 16;

pub(crate) struct LiveWavetable {
    baseline: EngineOscillator,
    measured: [WavetableTableOscillator; WideF32::LANES],
    measured_secondary: [WavetableTableOscillator; WideF32::LANES],
    measured_waveform_lanes: [bool; WideF32::LANES],
    measured_lanes: [bool; WideF32::LANES],
    sync_correction: [[f32; SYNC_BLEP_SAMPLES]; WideF32::LANES],
    sync_correction_index: usize,
    enabled: bool,
    slop_enabled: bool,
    pitch_grid_refresh_countdown: u8,
    waveform: Waveform,
    shape: f32,
}

impl LiveWavetable {
    pub(crate) fn new(bank: WavetableBank, sample_rate: f32) -> Self {
        Self {
            baseline: EngineOscillator::new_engine(sample_rate),
            measured: core::array::from_fn(|_| WavetableTableOscillator::new(bank, sample_rate)),
            measured_secondary: core::array::from_fn(|_| {
                WavetableTableOscillator::new(bank, sample_rate)
            }),
            measured_waveform_lanes: [true; WideF32::LANES],
            measured_lanes: [true; WideF32::LANES],
            sync_correction: [[0.0; SYNC_BLEP_SAMPLES]; WideF32::LANES],
            sync_correction_index: 0,
            enabled: true,
            slop_enabled: false,
            pitch_grid_refresh_countdown: 0,
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
        self.update_measured_frequencies(self.baseline.frequency_hz(), true);
    }

    pub(crate) fn set_frequency(&mut self, frequency: WideF32) {
        self.baseline.set_frequency(frequency);
        self.update_measured_frequencies(self.baseline.frequency_hz(), true);
    }

    fn update_measured_frequencies(&mut self, frequency: WideF32, refresh_pitch_grid: bool) {
        for (lane, value) in frequency.to_array().into_iter().enumerate() {
            let primary_frequency = if refresh_pitch_grid {
                self.measured[lane].set_frequency_live(value)
            } else {
                self.measured[lane].set_frequency_live_control_rate(value, false)
            };
            let secondary_frequency = if refresh_pitch_grid {
                self.measured_secondary[lane].set_frequency_live(value)
            } else {
                self.measured_secondary[lane].set_frequency_live_control_rate(value, false)
            };
            self.measured_lanes[lane] =
                self.measured_waveform_lanes[lane] && primary_frequency && secondary_frequency;
        }
    }

    pub(crate) fn set_shape(&mut self, shape: f32) {
        self.shape = shape.clamp(0.0, 1.0);
        self.baseline.set_shape(shape);
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.baseline.set_enabled(enabled);
    }

    pub(crate) fn set_slop_amount(&mut self, amount: f32) {
        self.baseline.set_slop_amount(amount);
        self.slop_enabled = amount > 0.0;
        self.pitch_grid_refresh_countdown = PITCH_GRID_REFRESH_SAMPLES - 1;
        self.update_measured_frequencies(self.baseline.frequency_hz(), true);
    }

    pub(crate) fn trigger_lane(&mut self, lane: usize, reset_phase: bool) {
        self.baseline.trigger_lane(lane, reset_phase);
        self.measured[lane].reset(reset_phase);
        self.measured_secondary[lane].reset(reset_phase);
        if reset_phase {
            self.sync_correction[lane] = [0.0; SYNC_BLEP_SAMPLES];
        }
    }

    pub(crate) fn frequency_hz(&self) -> WideF32 {
        self.baseline.frequency_hz()
    }

    pub(crate) fn set_bank(&mut self, bank: WavetableBank) {
        for lane in 0..WideF32::LANES {
            self.measured[lane].set_bank(bank);
            self.measured_secondary[lane].set_bank(bank);
        }
        self.clear_sync_corrections();
        self.set_waveform(self.waveform);
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
            let refresh_pitch_grid = self.pitch_grid_refresh_countdown == 0;
            self.update_measured_frequencies(self.baseline.frequency_hz(), refresh_pitch_grid);
            self.pitch_grid_refresh_countdown = if refresh_pitch_grid {
                PITCH_GRID_REFRESH_SAMPLES - 1
            } else {
                self.pitch_grid_refresh_countdown - 1
            };
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

#[cfg(test)]
mod tests {
    extern crate std;

    use std::boxed::Box;
    use std::vec;

    use crate::dsp::wavetable_bank_profile::MONOLOGUE_WAVETABLE_BANK_PROFILE;

    use super::*;

    fn constant_measured_bank() -> WavetableBank {
        let profile = &MONOLOGUE_WAVETABLE_BANK_PROFILE;
        WavetableBank::new_unchecked_for_test(
            Box::leak(vec![0.25; profile.sample_count].into_boxed_slice()),
            profile,
        )
    }

    fn ramp_measured_bank() -> WavetableBank {
        let profile = &MONOLOGUE_WAVETABLE_BANK_PROFILE;
        let mut samples = vec![0.0; profile.sample_count];
        for (index, sample) in samples.iter_mut().enumerate() {
            let phase = (index % profile.table_length) as f32 / profile.table_length as f32;
            *sample = phase * 2.0 - 1.0;
        }
        WavetableBank::new_unchecked_for_test(Box::leak(samples.into_boxed_slice()), profile)
    }

    #[test]
    fn measured_live_source_renders_supported_lanes_and_falls_back_safely() {
        let mut source = LiveWavetable::new(constant_measured_bank(), 48_000.0);
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
        let mut stable = LiveWavetable::new(bank, 48_000.0);
        let mut sloppy = LiveWavetable::new(bank, 48_000.0);
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

    #[test]
    fn slop_updates_phase_increment_per_sample_but_pitch_grid_at_control_rate() {
        let mut source = LiveWavetable::new(ramp_measured_bank(), 48_000.0);
        source.set_waveform(Waveform::Saw);
        source.set_frequency(WideF32::splat(220.0));
        source.set_slop_amount(1.0);
        let initial_refreshes = source.measured[0].pitch_grid_refreshes_for_test();

        for _ in 0..256 {
            let mut context = crate::create_render_context!();
            let _ = source.next(&mut context);
            assert_eq!(
                source.measured[0].frequency_hz_for_test().to_bits(),
                source.baseline.frequency_hz().to_array()[0].to_bits(),
                "measured phase increment did not track the current slop frequency"
            );
        }

        let refreshes = source.measured[0].pitch_grid_refreshes_for_test() - initial_refreshes;
        assert!(refreshes > 0);
        assert!(
            refreshes <= 17,
            "pitch-grid search unexpectedly ran {refreshes} times for 256 samples"
        );
    }
}
