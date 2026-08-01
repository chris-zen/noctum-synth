//! Compact measured-target oscillator used only by the research registry.

use super::analog_oscillator::{polyblamp2_triangle, pulse_width_from_shape};
use super::blep::blep_saw;
use super::oscillator_research::{
    ResearchError, ResearchParameterDescriptor, ResearchParameterScale, ResearchRenderCase,
};
use super::{SawMethod, Waveform};
use crate::math::{F32, WideF32};

const TAU: f32 = core::f32::consts::TAU;

#[derive(Debug, Clone, Copy)]
pub(crate) struct PhaseFilterKnot {
    pub(crate) log2_frequency: f32,
    pub(crate) phase_a: f32,
    pub(crate) phase_b: f32,
    pub(crate) phase_offset_cycles: f32,
    pub(crate) lowpass_hz: f32,
    pub(crate) highpass_hz: f32,
    pub(crate) pole_hz: f32,
    pub(crate) zero_hz: f32,
    pub(crate) gain: f32,
    pub(crate) dc: f32,
}

pub(crate) struct PhaseFilterProfile {
    pub(crate) id: &'static str,
    pub(crate) target_id: &'static str,
    pub(crate) revision: u32,
    pub(crate) saw: &'static [PhaseFilterKnot],
    pub(crate) triangle: &'static [PhaseFilterKnot],
    pub(crate) pulse: &'static [PhaseFilterKnot],
}

#[derive(Debug, Clone, Copy)]
struct InterpolatedParameters {
    phase_a: f32,
    phase_b: f32,
    phase_offset_cycles: f32,
    lowpass_pole: f32,
    highpass_pole: f32,
    pole: f32,
    zero: f32,
    gain: f32,
    dc: f32,
}

pub(crate) const PARAMETERS: [ResearchParameterDescriptor; 2] = [
    ResearchParameterDescriptor {
        id: "phase-amount",
        name: "Fitted Phase Shape",
        unit: "mix",
        minimum: 0.0,
        maximum: 1.0,
        default: 1.0,
        scale: ResearchParameterScale::Linear,
    },
    ResearchParameterDescriptor {
        id: "filter-amount",
        name: "Fitted Linear Color",
        unit: "mix",
        minimum: 0.0,
        maximum: 1.0,
        default: 1.0,
        scale: ResearchParameterScale::Linear,
    },
];

/// Scalar reference implementation. It is deliberately not connected to the
/// production SIMD voice path until its sound and bounded cost are accepted.
pub(crate) struct TargetConditionedOscillator {
    profile: &'static PhaseFilterProfile,
    sample_rate_hz: f32,
    frequency_hz: f32,
    phase_increment: f32,
    phase: f32,
    waveform: Waveform,
    shape: f32,
    parameters: InterpolatedParameters,
    phase_amount: f32,
    filter_amount: f32,
    lowpass_state: f32,
    highpass_input: f32,
    highpass_output: f32,
    pole_zero_input: f32,
    pole_zero_output: f32,
    discontinuity_phase: f32,
}

impl TargetConditionedOscillator {
    pub(crate) fn new(profile: &'static PhaseFilterProfile, sample_rate_hz: f32) -> Self {
        let mut result = Self {
            profile,
            sample_rate_hz,
            frequency_hz: 220.0,
            phase_increment: 220.0 / sample_rate_hz,
            phase: 0.0,
            waveform: Waveform::Saw,
            shape: 0.0,
            parameters: interpolate(profile.saw, 220.0, sample_rate_hz),
            phase_amount: 1.0,
            filter_amount: 1.0,
            lowpass_state: 0.0,
            highpass_input: 0.0,
            highpass_output: 0.0,
            pole_zero_input: 0.0,
            pole_zero_output: 0.0,
            discontinuity_phase: 0.5,
        };
        result.refresh_parameters();
        result
    }

    pub(crate) fn configure(&mut self, case: ResearchRenderCase) -> Result<(), ResearchError> {
        if case.waveform == Waveform::SawTri {
            return Err(ResearchError::UnsupportedEvent);
        }
        self.sample_rate_hz = case.sample_rate_hz;
        self.frequency_hz = case.frequency_hz;
        self.phase_increment = case.frequency_hz / case.sample_rate_hz;
        self.waveform = case.waveform;
        self.shape = case.shape;
        self.clear_filter_state();
        if case.reset_phase {
            self.phase = 0.0;
        }
        self.refresh_parameters();
        Ok(())
    }

    pub(crate) fn set_frequency(&mut self, frequency_hz: f32) {
        self.frequency_hz = frequency_hz;
        self.phase_increment = frequency_hz / self.sample_rate_hz;
        self.refresh_parameters();
    }

    pub(crate) fn set_shape(&mut self, shape: f32) {
        self.shape = shape;
        self.refresh_discontinuity_phase();
    }

    pub(crate) fn hard_sync(&mut self, subsample_offset: f32) {
        self.phase = self.phase_increment * (1.0 - subsample_offset.clamp(0.0, 1.0));
    }

    pub(crate) fn next_sample(&mut self) -> f32 {
        let shifted = wrap01(self.phase + self.parameters.phase_offset_cycles * self.phase_amount);
        let angle = TAU * shifted;
        let phase_a = self.parameters.phase_a * self.phase_amount;
        let phase_b = self.parameters.phase_b * self.phase_amount;
        let warped = wrap01(
            shifted
                + phase_a * F32(angle).sin().as_f32() / TAU
                + phase_b * F32(angle * 2.0).sin().as_f32() / (TAU * 2.0),
        );
        let derivative =
            (1.0 + phase_a * F32(angle).cos().as_f32() + phase_b * F32(angle * 2.0).cos().as_f32())
                .max(0.08);
        let warped_increment = (self.phase_increment * derivative).min(0.499);
        let increment = WideF32::splat(warped_increment);
        let base_increment = WideF32::splat(self.phase_increment);
        let source = if self.profile.revision >= 2 {
            match self.waveform {
                Waveform::Saw => {
                    let bandlimited =
                        blep_saw(WideF32::splat(shifted), base_increment, SawMethod::Blep)
                            .to_array()[0];
                    bandlimited + 2.0 * (warped - shifted)
                }
                Waveform::Triangle => {
                    polyblamp2_triangle(WideF32::splat(warped), increment).to_array()[0]
                }
                Waveform::Pulse => {
                    let width = pulse_width_from_shape(self.shape);
                    let edge = 1.0 - width;
                    let rising = blep_saw(WideF32::splat(shifted), base_increment, SawMethod::Blep)
                        .to_array()[0];
                    let falling = blep_saw(
                        WideF32::splat(wrap01(shifted - self.discontinuity_phase)),
                        base_increment,
                        SawMethod::Blep,
                    )
                    .to_array()[0];
                    rising - falling + (1.0 - 2.0 * edge)
                }
                Waveform::SawTri => 0.0,
            }
        } else {
            match self.waveform {
                Waveform::Saw => {
                    let event_phase = wrap01(shifted - self.discontinuity_phase);
                    // A warped saw is a linear saw at the relocated edge plus a
                    // smooth periodic curvature residual. Band-limit only the
                    // discontinuity, then restore that low-harmonic residual.
                    let bandlimited_linear =
                        blep_saw(WideF32::splat(event_phase), base_increment, SawMethod::Blep)
                            .to_array()[0];
                    let naive_linear = 2.0 * event_phase - 1.0;
                    let naive_warped = 2.0 * wrap01(warped + 0.5) - 1.0;
                    bandlimited_linear + naive_warped - naive_linear
                }
                Waveform::Triangle => {
                    polyblamp2_triangle(WideF32::splat(wrap01(warped + 0.25)), increment).to_array()
                        [0]
                }
                Waveform::Pulse => {
                    let width = pulse_width_from_shape(self.shape);
                    let edge = 1.0 - width;
                    let falling = blep_saw(
                        WideF32::splat(wrap01(shifted - self.discontinuity_phase)),
                        base_increment,
                        SawMethod::Blep,
                    )
                    .to_array()[0];
                    let rising = blep_saw(WideF32::splat(shifted), base_increment, SawMethod::Blep)
                        .to_array()[0];
                    // Two band-limited ramps cancel into the comparator plateau
                    // while preserving the relocated rising and falling edges.
                    falling - rising + (2.0 * edge - 1.0)
                }
                Waveform::SawTri => 0.0,
            }
        };

        let p = self.parameters;
        self.lowpass_state = (1.0 - p.lowpass_pole) * source + p.lowpass_pole * self.lowpass_state;
        let highpass =
            p.highpass_pole * (self.highpass_output + self.lowpass_state - self.highpass_input);
        self.highpass_input = self.lowpass_state;
        self.highpass_output = highpass;
        let colored = highpass - p.zero * self.pole_zero_input + p.pole * self.pole_zero_output;
        self.pole_zero_input = highpass;
        self.pole_zero_output = colored;
        let fitted = p.gain * colored + p.dc;
        let output = source + (fitted - source) * self.filter_amount;

        self.phase = wrap01(self.phase + self.phase_increment);
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
            "phase-amount" => {
                self.phase_amount = value;
                self.refresh_discontinuity_phase();
            }
            "filter-amount" => self.filter_amount = value,
            _ => return Err(ResearchError::UnknownParameter),
        }
        Ok(())
    }

    pub(crate) fn parameter_value(&self, id: &str) -> Option<f32> {
        match id {
            "phase-amount" => Some(self.phase_amount),
            "filter-amount" => Some(self.filter_amount),
            _ => None,
        }
    }

    fn refresh_parameters(&mut self) {
        let knots = match self.waveform {
            Waveform::Saw | Waveform::SawTri => self.profile.saw,
            Waveform::Triangle => self.profile.triangle,
            Waveform::Pulse => self.profile.pulse,
        };
        self.parameters = interpolate(knots, self.frequency_hz, self.sample_rate_hz);
        self.refresh_discontinuity_phase();
    }

    fn refresh_discontinuity_phase(&mut self) {
        let threshold = match self.waveform {
            Waveform::Saw => 0.5,
            Waveform::Pulse => 1.0 - pulse_width_from_shape(self.shape),
            Waveform::Triangle | Waveform::SawTri => return,
        };
        let phase_a = self.parameters.phase_a * self.phase_amount;
        let phase_b = self.parameters.phase_b * self.phase_amount;
        let mut lower = 0.0_f32;
        let mut upper = 1.0_f32;
        // The fitted phase map is monotonic. A fixed iteration count keeps
        // coefficient updates deterministic and bounded outside the render loop.
        for _ in 0..24 {
            let midpoint = (lower + upper) * 0.5;
            if unwrapped_phase_map(midpoint, phase_a, phase_b) < threshold {
                lower = midpoint;
            } else {
                upper = midpoint;
            }
        }
        self.discontinuity_phase = (lower + upper) * 0.5;
    }

    fn clear_filter_state(&mut self) {
        self.lowpass_state = 0.0;
        self.highpass_input = 0.0;
        self.highpass_output = 0.0;
        self.pole_zero_input = 0.0;
        self.pole_zero_output = 0.0;
    }
}

fn interpolate(
    knots: &'static [PhaseFilterKnot],
    frequency_hz: f32,
    sample_rate_hz: f32,
) -> InterpolatedParameters {
    debug_assert!(!knots.is_empty());
    let log_frequency = libm::log2f(frequency_hz);
    let (lower, upper, amount) = if log_frequency <= knots[0].log2_frequency {
        (&knots[0], &knots[0], 0.0)
    } else if log_frequency >= knots[knots.len() - 1].log2_frequency {
        let last = &knots[knots.len() - 1];
        (last, last, 0.0)
    } else {
        let upper_index = knots
            .iter()
            .position(|knot| knot.log2_frequency >= log_frequency)
            .unwrap_or(knots.len() - 1);
        let lower = &knots[upper_index - 1];
        let upper = &knots[upper_index];
        let amount =
            (log_frequency - lower.log2_frequency) / (upper.log2_frequency - lower.log2_frequency);
        (lower, upper, amount)
    };
    let lerp = |a: f32, b: f32| a + (b - a) * amount;
    let geometric_lerp = |a: f32, b: f32| {
        F32(lerp(F32(a).ln().as_f32(), F32(b).ln().as_f32()))
            .exp()
            .as_f32()
    };
    let coefficient = |frequency: f32| F32(-TAU * frequency / sample_rate_hz).exp().as_f32();
    InterpolatedParameters {
        phase_a: lerp(lower.phase_a, upper.phase_a),
        phase_b: lerp(lower.phase_b, upper.phase_b),
        phase_offset_cycles: lerp(lower.phase_offset_cycles, upper.phase_offset_cycles),
        lowpass_pole: coefficient(geometric_lerp(lower.lowpass_hz, upper.lowpass_hz)),
        highpass_pole: coefficient(geometric_lerp(lower.highpass_hz, upper.highpass_hz)),
        pole: coefficient(geometric_lerp(lower.pole_hz, upper.pole_hz)),
        zero: coefficient(geometric_lerp(lower.zero_hz, upper.zero_hz)),
        gain: lerp(lower.gain, upper.gain),
        dc: lerp(lower.dc, upper.dc),
    }
}

#[inline]
fn wrap01(value: f32) -> f32 {
    value - F32(value).floor().as_f32()
}

#[inline]
fn unwrapped_phase_map(phase: f32, phase_a: f32, phase_b: f32) -> f32 {
    let angle = TAU * phase;
    phase
        + phase_a * F32(angle).sin().as_f32() / TAU
        + phase_b * F32(angle * 2.0).sin().as_f32() / (TAU * 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::target_conditioned_profile::KORG_MONOLOGUE_PHASE_FILTER_V1;
    use crate::dsp::target_conditioned_profile_v2::KORG_MONOLOGUE_PHASE_FILTER_V2;

    #[test]
    fn interpolation_preserves_a_knot() {
        let knot = KORG_MONOLOGUE_PHASE_FILTER_V1.saw[8];
        let frequency = F32(knot.log2_frequency).exp2().as_f32();
        let parameters = interpolate(KORG_MONOLOGUE_PHASE_FILTER_V1.saw, frequency, 48_000.0);
        assert!((parameters.phase_a - knot.phase_a).abs() < 2.0e-6);
        assert!((parameters.gain - knot.gain).abs() < 2.0e-6);
    }

    #[test]
    fn v2_identity_phase_source_uses_production_convention() {
        for waveform in [Waveform::Saw, Waveform::Triangle, Waveform::Pulse] {
            let mut baseline =
                TargetConditionedOscillator::new(&KORG_MONOLOGUE_PHASE_FILTER_V2, 48_000.0);
            baseline
                .configure(ResearchRenderCase {
                    waveform,
                    sample_rate_hz: 48_000.0,
                    frequency_hz: 440.0,
                    shape: 0.0,
                    warmup_samples: 0,
                    render_samples: 1,
                    seed: 0,
                    reset_phase: true,
                })
                .unwrap();
            baseline.set_parameter("phase-amount", 0.0).unwrap();
            baseline.set_parameter("filter-amount", 0.0).unwrap();

            let expected = match waveform {
                Waveform::Saw => blep_saw(
                    WideF32::splat(0.0),
                    WideF32::splat(440.0 / 48_000.0),
                    SawMethod::Blep,
                )
                .to_array()[0],
                Waveform::Triangle => {
                    polyblamp2_triangle(WideF32::splat(0.0), WideF32::splat(440.0 / 48_000.0))
                        .to_array()[0]
                }
                Waveform::Pulse => {
                    let phase = WideF32::splat(0.0);
                    let increment = WideF32::splat(440.0 / 48_000.0);
                    blep_saw(phase, increment, SawMethod::Blep).to_array()[0]
                        - blep_saw(WideF32::splat(0.5), increment, SawMethod::Blep).to_array()[0]
                }
                Waveform::SawTri => unreachable!(),
            };
            assert!((baseline.next_sample() - expected).abs() < 2.0e-6);
        }
    }

    #[test]
    fn every_supported_waveform_stays_finite() {
        for waveform in [Waveform::Saw, Waveform::Triangle, Waveform::Pulse] {
            let mut oscillator =
                TargetConditionedOscillator::new(&KORG_MONOLOGUE_PHASE_FILTER_V1, 48_000.0);
            oscillator
                .configure(ResearchRenderCase {
                    waveform,
                    sample_rate_hz: 48_000.0,
                    frequency_hz: 997.0,
                    shape: 0.0,
                    warmup_samples: 0,
                    render_samples: 1,
                    seed: 0,
                    reset_phase: true,
                })
                .unwrap();
            for _ in 0..96_000 {
                assert!(oscillator.next_sample().is_finite());
            }
        }
    }
}
