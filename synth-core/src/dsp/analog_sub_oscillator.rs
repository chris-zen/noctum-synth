use crate::math::WideF32;

use crate::dsp::analog_oscillator::{MAX_PHASE_INC, MIN_PHASE_INC};
use crate::dsp::blep::{SawMethod, blep_pulse};
use crate::wrap01;

/// One-octave-down square sub oscillator.
pub struct AnalogSubOscillator {
    phase: WideF32,
    phase_inc: WideF32,
}

impl Default for AnalogSubOscillator {
    fn default() -> Self {
        Self {
            phase: WideF32::ZERO,
            phase_inc: WideF32::ZERO,
        }
    }
}

impl AnalogSubOscillator {
    pub fn reset(&mut self) {
        self.phase = WideF32::ZERO;
    }

    pub fn reset_lane(&mut self, lane: usize) {
        self.phase = self.phase.replace_lane(lane, 0.0);
    }

    pub fn next(&mut self) -> WideF32 {
        let phi = self.phase;
        self.phase += self.phase_inc;
        self.phase = wrap01(self.phase);
        -blep_pulse(phi, self.phase_inc, WideF32::splat(0.5), SawMethod::Blep)
    }

    pub fn set_frequency(&mut self, freq: WideF32, sample_rate: f32) {
        self.phase_inc = (freq * WideF32::splat(0.5 / sample_rate))
            .clamp(WideF32::splat(MIN_PHASE_INC), WideF32::splat(MAX_PHASE_INC));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_frequency_square_starts_positive() {
        let sample_rate = 44100.0;
        let mut sub = AnalogSubOscillator::default();
        sub.set_frequency(WideF32::splat(440.0), sample_rate);

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
