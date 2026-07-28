use crate::math::{F32, WideF32};

const CUTOFF_HZ: f32 = 3.5;

pub struct DcBlocker {
    previous_input: WideF32,
    previous_output: WideF32,
    coefficient: WideF32,
}

impl DcBlocker {
    pub fn new(sample_rate: f32) -> Self {
        let coefficient = F32(-core::f32::consts::TAU * CUTOFF_HZ / sample_rate.max(1.0))
            .exp()
            .as_f32();
        Self {
            previous_input: WideF32::ZERO,
            previous_output: WideF32::ZERO,
            coefficient: WideF32::splat(coefficient),
        }
    }

    pub fn process(&mut self, input: WideF32) -> WideF32 {
        let output = input - self.previous_input + self.previous_output * self.coefficient;
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
        let mut blocker = DcBlocker::new(SAMPLE_RATE);
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
        let mut blocker = DcBlocker::new(SAMPLE_RATE);
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
        let mut blocker = DcBlocker::new(SAMPLE_RATE);
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

    #[test]
    fn dc_decay_is_stable_across_sample_rates() {
        fn output_after(sample_rate: f32, seconds: f32) -> f32 {
            let mut blocker = DcBlocker::new(sample_rate);
            let samples = (sample_rate * seconds) as usize;
            let mut output = 0.0;
            for _ in 0..samples {
                output = blocker.process(WideF32::splat(1.0)).to_array()[0];
            }
            output
        }

        let at_24k = output_after(24_000.0, 0.1);
        let at_44k = output_after(44_100.0, 0.1);
        let at_48k = output_after(48_000.0, 0.1);
        assert!(
            (at_24k - at_44k).abs() < 0.001,
            "24k={at_24k} 44.1k={at_44k}"
        );
        assert!(
            (at_48k - at_44k).abs() < 0.001,
            "48k={at_48k} 44.1k={at_44k}"
        );
    }
}
