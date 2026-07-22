//! 2× stereo half-band upsampler.
//!
//! Half-band zeros reduce the 15-tap FIR to seven multiply-accumulates on one
//! output phase and a delayed copy on the other. Coefficients include the 2×
//! interpolation gain.

const FILTERED_PHASE: [f32; 8] = [
    -0.003_332_343_2,
    0.034_400_29,
    -0.138_039_95,
    0.606_972,
    0.606_972,
    -0.138_039_95,
    0.034_400_29,
    -0.003_332_343_2,
];

/// Fixed-ratio stereo upsampler for reconstructing every other output sample.
#[derive(Default)]
pub(crate) struct Upsampler {
    history: [(f32, f32); 8],
    write_index: usize,
    copy_phase: bool,
}

impl Upsampler {
    pub(crate) const fn needs_input(&self) -> bool {
        !self.copy_phase
    }

    pub(crate) fn submit(&mut self, frame: (f32, f32)) {
        self.history[self.write_index] = frame;
        self.write_index = (self.write_index + 1) & 7;
    }

    #[inline]
    pub(crate) fn output(&self) -> (f32, f32) {
        if self.copy_phase {
            return self.history[(self.write_index + 4) & 7];
        }

        let mut left = 0.0;
        let mut right = 0.0;
        for (age, coefficient) in FILTERED_PHASE.into_iter().enumerate() {
            let sample = self.history[(self.write_index + 7 - age) & 7];
            left += sample.0 * coefficient;
            right += sample.1 * coefficient;
        }
        (left, right)
    }

    pub(crate) fn advance(&mut self) {
        self.copy_phase = !self.copy_phase;
    }
}

#[cfg(test)]
mod tests {
    use super::Upsampler;

    #[test]
    fn requests_one_input_for_each_output_pair() {
        let mut upsampler = Upsampler::default();

        assert!(upsampler.needs_input());
        upsampler.submit((1.0, -1.0));
        let _ = upsampler.output();
        upsampler.advance();

        assert!(!upsampler.needs_input());
        let _ = upsampler.output();
        upsampler.advance();

        assert!(upsampler.needs_input());
    }

    #[test]
    fn reconstruction_has_unity_dc_gain_and_stereo_independence() {
        let mut upsampler = Upsampler::default();
        let mut output = [(0.0, 0.0); 32];
        for frame in &mut output {
            if upsampler.needs_input() {
                upsampler.submit((1.0, -0.5));
            }
            *frame = upsampler.output();
            upsampler.advance();
        }

        for &(left, right) in &output[20..] {
            assert!((left - 1.0).abs() < 2.0e-4, "left={left}");
            assert!((right + 0.5).abs() < 1.0e-4, "right={right}");
        }
    }
}
