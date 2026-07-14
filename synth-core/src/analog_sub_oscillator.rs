use crate::f32x4;

use crate::analog_oscillator::{MAX_PHASE_INC, MIN_PHASE_INC};
use crate::blep::{SawMethod, blep_pulse};
use crate::wrap01;

/// One-octave-down square sub oscillator.
pub struct AnalogSubOscillator {
    phase: f32x4,
    phase_inc: f32x4,
}

impl Default for AnalogSubOscillator {
    fn default() -> Self {
        Self {
            phase: f32x4::splat(0.0),
            phase_inc: f32x4::splat(0.0),
        }
    }
}

impl AnalogSubOscillator {
    pub fn reset(&mut self) {
        self.phase = f32x4::splat(0.0);
    }

    pub fn reset_lane(&mut self, lane: usize) {
        let mut phase = self.phase.to_array();
        phase[lane] = 0.0;
        self.phase = f32x4::new(phase);
    }

    pub fn next(&mut self) -> f32x4 {
        let phi = self.phase;
        self.phase += self.phase_inc;
        self.phase = wrap01(self.phase);
        -blep_pulse(phi, self.phase_inc, f32x4::splat(0.5), SawMethod::Blep)
    }

    pub fn set_frequency(&mut self, freq: f32x4, sample_rate: f32) {
        self.phase_inc = (freq * f32x4::splat(0.5 / sample_rate))
            .clamp(f32x4::splat(MIN_PHASE_INC), f32x4::splat(MAX_PHASE_INC));
    }
}
