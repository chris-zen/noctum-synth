use crate::math::WideF32;

const COEFFICIENT: f32 = 0.9995;

pub struct DcBlocker {
    previous_input: WideF32,
    previous_output: WideF32,
}

impl Default for DcBlocker {
    fn default() -> Self {
        Self::new()
    }
}

impl DcBlocker {
    pub fn new() -> Self {
        Self {
            previous_input: WideF32::ZERO,
            previous_output: WideF32::ZERO,
        }
    }

    pub fn process(&mut self, input: WideF32) -> WideF32 {
        let output = input - self.previous_input
            + self.previous_output * WideF32::splat(COEFFICIENT);
        self.previous_input = input;
        self.previous_output = output;
        output
    }

    pub fn reset_lane(&mut self, lane: usize) {
        self.previous_input = self.previous_input.replace_lane(lane, 0.0);
        self.previous_output = self.previous_output.replace_lane(lane, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    extern crate std;
    use std::vec::Vec;

    const SAMPLE_RATE: f32 = 44_100.0;

    fn mean(samples: &[f32]) -> f32 {
        samples.iter().sum::<f32>() / samples.len() as f32
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn removes_dc_offset_within_about_a_quarter_second() {
        let mut blocker = DcBlocker::new();
        let settle = (SAMPLE_RATE * 0.25) as usize;
        let measure = (SAMPLE_RATE * 0.02) as usize;
        let mut measured = Vec::with_capacity(measure);

        for i in 0..(settle + measure) {
            let output = blocker.process(WideF32::splat(0.5)).to_array()[0];
            if i >= settle {
                measured.push(output);
            }
        }

        assert!(
            mean(&measured).abs() < 0.01,
            "DC should settle near zero, mean={}",
            mean(&measured)
        );
    }

    #[test]
    fn preserves_low_bass_energy() {
        let mut blocker = DcBlocker::new();
        let frequency = 40.0;
        let step = core::f32::consts::TAU * frequency / SAMPLE_RATE;
        let settle = (SAMPLE_RATE * 0.1) as usize;
        let measure = (SAMPLE_RATE * 0.1) as usize;
        let mut phase = 0.0f32;
        let mut blocked = Vec::with_capacity(measure);
        let mut dry = Vec::with_capacity(measure);

        for i in 0..(settle + measure) {
            let sample = phase.sin();
            phase += step;
            let output = blocker.process(WideF32::splat(sample)).to_array()[0];
            if i >= settle {
                blocked.push(output);
                dry.push(sample);
            }
        }

        let blocked_rms = rms(&blocked);
        let dry_rms = rms(&dry);
        assert!(
            (blocked_rms / dry_rms - 1.0).abs() < 0.05,
            "40 Hz should stay within 5%, blocked_rms={blocked_rms} dry_rms={dry_rms}"
        );
    }

    #[test]
    fn reset_lane_clears_only_that_lane() {
        let mut blocker = DcBlocker::new();
        for _ in 0..100 {
            let _ = blocker.process(WideF32::splat(1.0));
        }
        let before = blocker.previous_input.to_array();
        blocker.reset_lane(0);
        let after = blocker.previous_input.to_array();
        assert_eq!(after[0], 0.0);
        for lane in 1..WideF32::LANES {
            assert_eq!(after[lane], before[lane]);
        }
    }
}
