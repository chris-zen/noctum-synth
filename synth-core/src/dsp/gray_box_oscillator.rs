//! Bounded scalar saw-core relaxation oscillator for Plan 12 research.

use super::Waveform;
use super::analog_oscillator::{polyblamp2_triangle, pulse_width_from_shape};
use super::blep::{table_blep_post_step_correction_lane, table_points_per_side_lane};
use super::oscillator_research::{
    ResearchError, ResearchParameterDescriptor, ResearchParameterScale, ResearchRenderCase,
};
use crate::math::WideF32;

const BLEP_SAMPLES: usize = 4;
const MAX_STATE_EVENTS_PER_SAMPLE: u8 = 2;
const CURVATURE_EPSILON: f32 = 1.0e-4;

#[derive(Debug, Clone, Copy)]
pub(crate) struct GrayBoxOutput {
    pub(crate) lowpass_hz: f32,
    pub(crate) gain: f32,
    pub(crate) dc: f32,
}

pub(crate) struct GrayBoxProfile {
    pub(crate) id: &'static str,
    pub(crate) target_id: &'static str,
    pub(crate) revision: u32,
    pub(crate) curvature: f32,
    pub(crate) saw: GrayBoxOutput,
    pub(crate) triangle: GrayBoxOutput,
    pub(crate) pulse: GrayBoxOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoreMode {
    Charging,
    Resetting,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GrayBoxDiagnostics {
    pub(crate) capacitor_v: f32,
    pub(crate) comparator_high: bool,
    pub(crate) threshold_v: f32,
    pub(crate) raw_output: f32,
    pub(crate) corrected_output: f32,
    pub(crate) state_events: u8,
    pub(crate) last_event_offset: Option<f32>,
}

pub(crate) const PARAMETERS: [ResearchParameterDescriptor; 4] = [
    ResearchParameterDescriptor {
        id: "current-curvature",
        name: "Fitted Current Curvature",
        unit: "mix",
        minimum: 0.0,
        maximum: 1.0,
        default: 1.0,
        scale: ResearchParameterScale::Linear,
    },
    ResearchParameterDescriptor {
        id: "reset-duration",
        name: "Reset Duration",
        unit: "cycles",
        minimum: 0.0,
        maximum: 0.08,
        default: 0.0,
        scale: ResearchParameterScale::Linear,
    },
    ResearchParameterDescriptor {
        id: "output-filter",
        name: "Fitted Output Filter",
        unit: "mix",
        minimum: 0.0,
        maximum: 1.0,
        default: 1.0,
        scale: ResearchParameterScale::Linear,
    },
    ResearchParameterDescriptor {
        id: "antialias",
        name: "Event BLEP/BLAMP",
        unit: "mix",
        minimum: 0.0,
        maximum: 1.0,
        default: 1.0,
        scale: ResearchParameterScale::Linear,
    },
];

pub(crate) struct GrayBoxOscillator {
    profile: &'static GrayBoxProfile,
    sample_rate_hz: f32,
    frequency_hz: f32,
    phase_increment: f32,
    waveform: Waveform,
    shape: f32,
    capacitor_v: f32,
    mode: CoreMode,
    correction: [f32; BLEP_SAMPLES],
    correction_index: usize,
    filter_state: f32,
    curvature_amount: f32,
    reset_duration_amount: f32,
    output_filter_amount: f32,
    antialias_amount: f32,
    sample_already_advanced: bool,
    advanced_state_events: u8,
    diagnostics: GrayBoxDiagnostics,
}

impl GrayBoxOscillator {
    pub(crate) fn new(profile: &'static GrayBoxProfile, sample_rate_hz: f32) -> Self {
        Self {
            profile,
            sample_rate_hz,
            frequency_hz: 220.0,
            phase_increment: 220.0 / sample_rate_hz,
            waveform: Waveform::Saw,
            shape: 0.0,
            capacitor_v: 0.0,
            mode: CoreMode::Charging,
            correction: [0.0; BLEP_SAMPLES],
            correction_index: 0,
            filter_state: 0.0,
            curvature_amount: 1.0,
            reset_duration_amount: 1.0,
            output_filter_amount: 1.0,
            antialias_amount: 1.0,
            sample_already_advanced: false,
            advanced_state_events: 0,
            diagnostics: GrayBoxDiagnostics {
                capacitor_v: 0.0,
                comparator_high: true,
                threshold_v: 0.5,
                raw_output: -1.0,
                corrected_output: -1.0,
                state_events: 0,
                last_event_offset: None,
            },
        }
    }

    pub(crate) fn configure(&mut self, case: ResearchRenderCase) -> Result<(), ResearchError> {
        self.sample_rate_hz = case.sample_rate_hz;
        self.frequency_hz = case.frequency_hz;
        self.phase_increment = case.frequency_hz / case.sample_rate_hz;
        self.waveform = case.waveform;
        self.shape = case.shape;
        self.clear_output_state();
        if case.reset_phase {
            self.capacitor_v = 0.0;
            self.mode = CoreMode::Charging;
        }
        self.sample_already_advanced = false;
        self.advanced_state_events = 0;
        Ok(())
    }

    pub(crate) fn reset(&mut self, reset_phase: bool) {
        self.clear_output_state();
        if reset_phase {
            self.capacitor_v = 0.0;
            self.mode = CoreMode::Charging;
        }
        self.sample_already_advanced = false;
        self.advanced_state_events = 0;
    }

    pub(crate) fn set_frequency(&mut self, frequency_hz: f32) {
        self.frequency_hz = frequency_hz;
        self.phase_increment = frequency_hz / self.sample_rate_hz;
    }

    pub(crate) fn set_shape(&mut self, shape: f32) {
        let before = (self.waveform == Waveform::Pulse).then(|| self.raw_observation());
        self.shape = shape.clamp(0.0, 1.0);
        if let Some(before) = before {
            let after = self.raw_observation();
            if after != before {
                self.add_step_correction(after - before, 1.0);
            }
        }
    }

    pub(crate) fn hard_sync(&mut self, subsample_offset: f32) {
        let offset = subsample_offset.clamp(0.0, 1.0);
        let before_events = self.advance_core(self.phase_increment * offset);
        let before = self.raw_observation();
        self.capacitor_v = 0.0;
        self.mode = CoreMode::Charging;
        let after = self.raw_observation();
        self.add_step_correction(after - before, 1.0 - offset);
        let remaining = self.phase_increment * (1.0 - offset);
        let after_events = self.advance_core(remaining);
        self.sample_already_advanced = true;
        self.advanced_state_events = before_events + after_events;
        self.diagnostics.last_event_offset = Some(offset);
    }

    pub(crate) fn next_sample(&mut self) -> f32 {
        let state_events = if self.sample_already_advanced {
            self.sample_already_advanced = false;
            let events = self.advanced_state_events;
            self.advanced_state_events = 0;
            events
        } else {
            self.diagnostics.last_event_offset = None;
            self.advance_core(self.phase_increment)
        };
        let raw = self.raw_observation();
        let corrected = raw + self.take_correction();
        let output_profile = self.output_profile();
        let pole =
            libm::expf(-core::f32::consts::TAU * output_profile.lowpass_hz / self.sample_rate_hz);
        self.filter_state = (1.0 - pole) * corrected + pole * self.filter_state;
        let filtered = output_profile.gain * self.filter_state + output_profile.dc;
        let unfiltered = output_profile.gain * corrected + output_profile.dc;
        let output = unfiltered + (filtered - unfiltered) * self.output_filter_amount;
        self.correction_index = (self.correction_index + 1) % BLEP_SAMPLES;
        self.diagnostics = GrayBoxDiagnostics {
            capacitor_v: self.capacitor_v,
            comparator_high: self.capacitor_v < self.pulse_threshold(),
            threshold_v: self.pulse_threshold(),
            raw_output: raw,
            corrected_output: output,
            state_events,
            last_event_offset: self.diagnostics.last_event_offset,
        };
        output
    }

    pub(crate) fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), ResearchError> {
        let descriptor = PARAMETERS
            .iter()
            .find(|descriptor| descriptor.id == id)
            .ok_or(ResearchError::UnknownParameter)?;
        if !value.is_finite() || value < descriptor.minimum || value > descriptor.maximum {
            return Err(ResearchError::InvalidParameterValue);
        }
        match id {
            "current-curvature" => self.curvature_amount = value,
            "reset-duration" => {
                if value <= 1.0e-7 && self.mode == CoreMode::Resetting {
                    let before = self.raw_observation();
                    self.reset_duration_amount = value;
                    self.capacitor_v = 0.0;
                    self.mode = CoreMode::Charging;
                    let after = self.raw_observation();
                    self.add_step_correction(after - before, 1.0);
                } else {
                    self.reset_duration_amount = value;
                }
            }
            "output-filter" => self.output_filter_amount = value,
            "antialias" => self.antialias_amount = value,
            _ => return Err(ResearchError::UnknownParameter),
        }
        Ok(())
    }

    pub(crate) fn parameter_value(&self, id: &str) -> Option<f32> {
        match id {
            "current-curvature" => Some(self.curvature_amount),
            "reset-duration" => Some(self.reset_duration_amount),
            "output-filter" => Some(self.output_filter_amount),
            "antialias" => Some(self.antialias_amount),
            _ => None,
        }
    }

    pub(crate) fn diagnostics(&self) -> GrayBoxDiagnostics {
        self.diagnostics
    }

    fn advance_core(&mut self, delta_cycles: f32) -> u8 {
        let mut remaining = delta_cycles;
        let mut elapsed = 0.0;
        let mut state_events = 0;
        while remaining > 0.0 {
            match self.mode {
                CoreMode::Charging => {
                    let to_high = self.charge_time(self.capacitor_v, 1.0);
                    let segment = remaining.min(to_high);
                    let before = self.capacitor_v;
                    let after = self.advance_charge(before, segment);
                    self.maybe_add_pulse_crossing(before, after, elapsed, delta_cycles, false);
                    self.capacitor_v = after.min(1.0);
                    remaining -= segment;
                    elapsed += segment;
                    if segment + 1.0e-8 >= to_high {
                        state_events += 1;
                        debug_assert!(state_events <= MAX_STATE_EVENTS_PER_SAMPLE);
                        self.record_event(elapsed, delta_cycles);
                        if self.reset_duration() <= 1.0e-7 {
                            self.add_reset_steps(elapsed, delta_cycles);
                            self.capacitor_v = 0.0;
                        } else {
                            self.mode = CoreMode::Resetting;
                        }
                    }
                }
                CoreMode::Resetting => {
                    let reset_duration = self.reset_duration();
                    let to_low = self.capacitor_v * reset_duration;
                    let segment = remaining.min(to_low);
                    let before = self.capacitor_v;
                    let after = (before - segment / reset_duration).max(0.0);
                    self.maybe_add_pulse_crossing(before, after, elapsed, delta_cycles, true);
                    self.capacitor_v = after;
                    remaining -= segment;
                    elapsed += segment;
                    if segment + 1.0e-8 >= to_low {
                        state_events += 1;
                        debug_assert!(state_events <= MAX_STATE_EVENTS_PER_SAMPLE);
                        self.record_event(elapsed, delta_cycles);
                        self.capacitor_v = 0.0;
                        self.mode = CoreMode::Charging;
                    }
                }
            }
        }
        state_events
    }

    fn charge_time(&self, from: f32, to: f32) -> f32 {
        let curvature = self.curvature();
        let scale = self.charge_scale();
        if curvature.abs() < CURVATURE_EPSILON {
            return (to - from) / scale;
        }
        let a = 1.0 - curvature;
        let b = 2.0 * curvature;
        libm::logf((a + b * to) / (a + b * from)) / (scale * b)
    }

    fn advance_charge(&self, from: f32, duration: f32) -> f32 {
        let curvature = self.curvature();
        let scale = self.charge_scale();
        if curvature.abs() < CURVATURE_EPSILON {
            return from + scale * duration;
        }
        let a = 1.0 - curvature;
        let b = 2.0 * curvature;
        ((a + b * from) * libm::expf(scale * b * duration) - a) / b
    }

    fn charge_scale(&self) -> f32 {
        let curvature = self.curvature();
        let travel = if curvature.abs() < CURVATURE_EPSILON {
            1.0
        } else {
            libm::logf((1.0 + curvature) / (1.0 - curvature)) / (2.0 * curvature)
        };
        travel / (1.0 - self.reset_duration()).max(0.1)
    }

    fn maybe_add_pulse_crossing(
        &mut self,
        before: f32,
        after: f32,
        elapsed: f32,
        total: f32,
        descending: bool,
    ) {
        if self.waveform != Waveform::Pulse {
            return;
        }
        let threshold = self.pulse_threshold();
        let crossed = if descending {
            before > threshold && after <= threshold
        } else {
            before < threshold && after >= threshold
        };
        if !crossed {
            return;
        }
        let crossing = if descending {
            (before - threshold) * self.reset_duration()
        } else {
            self.charge_time(before, threshold)
        };
        let step = if descending { 2.0 } else { -2.0 };
        self.add_step_at_cycle(step, elapsed + crossing, total);
    }

    fn add_reset_steps(&mut self, elapsed: f32, total: f32) {
        match self.waveform {
            Waveform::Saw => self.add_step_at_cycle(-2.0, elapsed, total),
            Waveform::SawTri => self.add_step_at_cycle(-2.0 * (1.0 - self.shape), elapsed, total),
            Waveform::Pulse => self.add_step_at_cycle(2.0, elapsed, total),
            Waveform::Triangle => {}
        }
    }

    fn add_step_at_cycle(&mut self, step: f32, elapsed: f32, total: f32) {
        let fraction = if total > 0.0 { elapsed / total } else { 1.0 };
        self.add_step_correction(step, 1.0 - fraction.clamp(0.0, 1.0));
    }

    fn add_step_correction(&mut self, step: f32, samples_since_edge: f32) {
        let step = step * self.antialias_amount;
        let points = table_points_per_side_lane(self.phase_increment);
        for tap in 0..BLEP_SAMPLES {
            let correction =
                table_blep_post_step_correction_lane(samples_since_edge + tap as f32, step, points);
            let index = (self.correction_index + tap) % BLEP_SAMPLES;
            self.correction[index] += correction;
        }
    }

    fn take_correction(&mut self) -> f32 {
        let correction = self.correction[self.correction_index];
        self.correction[self.correction_index] = 0.0;
        correction
    }

    fn raw_observation(&self) -> f32 {
        let saw = self.capacitor_v * 2.0 - 1.0;
        let triangle_increment = match self.mode {
            CoreMode::Charging => {
                self.charge_scale()
                    * (1.0 + self.curvature() * (2.0 * self.capacitor_v - 1.0))
                    * self.phase_increment
            }
            CoreMode::Resetting => self.phase_increment / self.reset_duration().max(1.0e-7),
        }
        .abs()
        .min(0.249);
        let triangle = polyblamp2_triangle(
            WideF32::splat(self.capacitor_v),
            WideF32::splat(triangle_increment),
        )
        .to_array()[0];
        let naive_triangle = 1.0 - 4.0 * (self.capacitor_v - 0.5).abs();
        let triangle_antialias = if self.curvature().abs() <= 0.5 {
            self.antialias_amount
        } else {
            0.0
        };
        let triangle = naive_triangle + (triangle - naive_triangle) * triangle_antialias;
        match self.waveform {
            Waveform::Saw => saw,
            Waveform::SawTri => saw + (triangle - saw) * self.shape,
            Waveform::Triangle => triangle,
            Waveform::Pulse => {
                if self.capacitor_v < self.pulse_threshold() {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }

    fn pulse_threshold(&self) -> f32 {
        pulse_width_from_shape(self.shape).clamp(0.01, 0.99)
    }

    fn curvature(&self) -> f32 {
        (self.profile.curvature * self.curvature_amount).clamp(-0.85, 0.85)
    }

    fn reset_duration(&self) -> f32 {
        self.reset_duration_amount.clamp(0.0, 0.08)
    }

    fn output_profile(&self) -> GrayBoxOutput {
        match self.waveform {
            Waveform::Saw | Waveform::SawTri => self.profile.saw,
            Waveform::Triangle => self.profile.triangle,
            Waveform::Pulse => self.profile.pulse,
        }
    }

    fn record_event(&mut self, elapsed: f32, total: f32) {
        self.diagnostics.last_event_offset = Some(if total > 0.0 {
            (elapsed / total).clamp(0.0, 1.0)
        } else {
            0.0
        });
    }

    fn clear_output_state(&mut self) {
        self.correction = [0.0; BLEP_SAMPLES];
        self.correction_index = 0;
        self.filter_state = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::gray_box_profile::KORG_MONOLOGUE_GRAY_BOX_V1;

    fn oscillator(frequency_hz: f32) -> GrayBoxOscillator {
        let mut oscillator = GrayBoxOscillator::new(&KORG_MONOLOGUE_GRAY_BOX_V1, 48_000.0);
        oscillator
            .configure(ResearchRenderCase {
                waveform: Waveform::Saw,
                sample_rate_hz: 48_000.0,
                frequency_hz,
                shape: 0.0,
                warmup_samples: 0,
                render_samples: 1,
                seed: 0,
                reset_phase: true,
            })
            .unwrap();
        oscillator
    }

    #[test]
    fn affine_update_reaches_threshold_at_requested_period() {
        for curvature_amount in [0.0, 0.5, 1.0] {
            let mut oscillator = oscillator(100.0);
            oscillator.curvature_amount = curvature_amount;
            oscillator.reset_duration_amount = 0.0;
            let period = oscillator.charge_time(0.0, 1.0);
            assert!((period - 1.0).abs() < 2.0e-6, "period={period}");
        }
    }

    #[test]
    fn event_count_is_bounded_at_maximum_frequency() {
        let mut oscillator = oscillator(48_000.0 * 0.489);
        for _ in 0..2_000 {
            let sample = oscillator.next_sample();
            assert!(sample.is_finite());
            assert!(oscillator.diagnostics().state_events <= MAX_STATE_EVENTS_PER_SAMPLE);
            assert!((0.0..=1.0).contains(&oscillator.diagnostics().capacitor_v));
        }
    }

    #[test]
    fn fractional_event_timing_matches_high_rate_reference() {
        let mut oscillator = oscillator(997.0);
        oscillator.reset_duration_amount = 0.0;
        let mut event_time = None;
        for sample in 0..100 {
            let _ = oscillator.next_sample();
            if let Some(offset) = oscillator.diagnostics().last_event_offset {
                event_time = Some(sample as f32 + offset);
                break;
            }
        }
        let measured = event_time.unwrap() / 48_000.0;
        let reference = 1.0 / 997.0;
        assert!((measured - reference).abs() < 2.0e-8);
    }

    #[test]
    fn outputs_share_identical_capacitor_state() {
        let mut saw = oscillator(440.0);
        let mut triangle = oscillator(440.0);
        triangle.waveform = Waveform::Triangle;
        let mut pulse = oscillator(440.0);
        pulse.waveform = Waveform::Pulse;
        for _ in 0..2_000 {
            let _ = saw.next_sample();
            let _ = triangle.next_sample();
            let _ = pulse.next_sample();
            assert_eq!(saw.capacitor_v.to_bits(), triangle.capacitor_v.to_bits());
            assert_eq!(saw.capacitor_v.to_bits(), pulse.capacitor_v.to_bits());
        }
    }

    #[test]
    fn disabling_reset_duration_mid_reset_stays_finite() {
        let mut oscillator = oscillator(2_400.0);
        oscillator.set_parameter("reset-duration", 0.08).unwrap();
        for _ in 0..30 {
            let _ = oscillator.next_sample();
            if oscillator.mode == CoreMode::Resetting {
                break;
            }
        }
        assert_eq!(oscillator.mode, CoreMode::Resetting);
        oscillator.set_parameter("reset-duration", 0.0).unwrap();
        assert_eq!(oscillator.mode, CoreMode::Charging);
        for _ in 0..16 {
            assert!(oscillator.next_sample().is_finite());
        }
    }
}
