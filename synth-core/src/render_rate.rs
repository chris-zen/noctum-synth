//! Engine-local internal-rate adaptation.
//!
//! The synthesis engine expresses only whether a new internal frame is due and
//! submits that frame to this component. Target selection and reconstruction
//! stay here instead of leaking embedded flags into semantic DSP algorithms.

#[cfg(any(
    not(all(feature = "embedded-math", target_os = "none")),
    feature = "daisy-full-rate"
))]
pub(crate) use full_rate::RateAdapter as EngineRateAdapter;
#[cfg(all(
    feature = "embedded-math",
    target_os = "none",
    not(feature = "daisy-full-rate")
))]
pub(crate) use half_rate::RateAdapter as EngineRateAdapter;

#[cfg(any(
    not(all(feature = "embedded-math", target_os = "none")),
    feature = "daisy-full-rate"
))]
mod full_rate {
    #[derive(Default)]
    pub(crate) struct RateAdapter {
        frame: (f32, f32),
    }

    impl RateAdapter {
        pub(crate) const fn internal_sample_rate(external_sample_rate: f32) -> f32 {
            external_sample_rate
        }

        pub(crate) const fn needs_render(&self) -> bool {
            true
        }

        pub(crate) fn submit(&mut self, frame: (f32, f32)) {
            self.frame = frame;
        }

        pub(crate) const fn output(&self) -> (f32, f32) {
            self.frame
        }

        pub(crate) const fn advance(&mut self) {}
    }
}

#[cfg(any(
    test,
    all(
        feature = "embedded-math",
        target_os = "none",
        not(feature = "daisy-full-rate")
    )
))]
mod half_rate {
    /// Fixed 24 kHz internal processing with 15-tap half-band reconstruction.
    ///
    /// Keeping the adapter state outside every DSP subsystem makes the chosen
    /// quality tier explicit without adding target branches to oscillators,
    /// filters, modulation, or effects. Half-band zeros reduce the 15-tap FIR
    /// to seven multiply-accumulates on one output phase and a delayed copy on
    /// the other. Coefficients include the 2x interpolation gain.
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

    #[derive(Default)]
    pub(crate) struct RateAdapter {
        history: [(f32, f32); 8],
        write_index: usize,
        copy_phase: bool,
    }

    impl RateAdapter {
        pub(crate) fn internal_sample_rate(external_sample_rate: f32) -> f32 {
            (external_sample_rate * 0.5).max(1.0)
        }

        pub(crate) const fn needs_render(&self) -> bool {
            !self.copy_phase
        }

        pub(crate) fn submit(&mut self, frame: (f32, f32)) {
            self.history[self.write_index] = frame;
            self.write_index = (self.write_index + 1) & 7;
        }

        #[inline]
        pub(crate) fn output(&self) -> (f32, f32) {
            if self.copy_phase {
                let sample = self.history[(self.write_index + 4) & 7];
                return sample;
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
}

#[cfg(test)]
mod tests {
    use super::half_rate::RateAdapter;

    #[test]
    fn half_rate_requests_one_render_for_each_output_pair() {
        let mut adapter = RateAdapter::default();

        assert!(adapter.needs_render());
        adapter.submit((1.0, -1.0));
        let _ = adapter.output();
        adapter.advance();

        assert!(!adapter.needs_render());
        let _ = adapter.output();
        adapter.advance();

        assert!(adapter.needs_render());
    }

    #[test]
    fn half_rate_reconstruction_has_unity_dc_gain_and_stereo_independence() {
        let mut adapter = RateAdapter::default();
        let mut output = [(0.0, 0.0); 32];
        for frame in &mut output {
            if adapter.needs_render() {
                adapter.submit((1.0, -0.5));
            }
            *frame = adapter.output();
            adapter.advance();
        }

        for &(left, right) in &output[20..] {
            assert!((left - 1.0).abs() < 2.0e-4, "left={left}");
            assert!((right + 0.5).abs() < 1.0e-4, "right={right}");
        }
    }

    #[test]
    fn half_rate_internal_sample_rate_is_half_the_external_rate() {
        assert_eq!(RateAdapter::internal_sample_rate(48_000.0), 24_000.0);
    }
}
