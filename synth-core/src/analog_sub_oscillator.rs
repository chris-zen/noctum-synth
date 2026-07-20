use crate::f32x4;

use crate::analog_oscillator::{MAX_PHASE_INC, MIN_PHASE_INC};
use crate::blep::{SawMethod, blep_pulse};
use crate::{F32x4Ext, wrap01};

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
        self.phase = self.phase.replace_lane(lane, 0.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_frequency_square_starts_positive() {
        let sample_rate = 44100.0;
        let mut sub = AnalogSubOscillator::default();
        sub.set_frequency(f32x4::splat(440.0), sample_rate);

        let mut positive_count = 0;
        for _ in 0..50 {
            if sub.next().to_array()[0] > 0.0 {
                positive_count += 1;
            }
        }
        assert_eq!(
            positive_count, 50,
            "first 50 samples should all be positive"
        );
    }
}
