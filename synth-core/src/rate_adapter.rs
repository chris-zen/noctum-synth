//! Engine-local internal-rate adaptation.
//!
//! Chooses full-rate passthrough or half-rate DSP plus [`crate::dsp::upsampler`]
//! reconstruction so embedded target flags stay out of semantic DSP algorithms.

#[cfg(any(
    not(all(feature = "embedded-math", target_os = "none")),
    feature = "daisy-full-rate"
))]
pub(crate) use full_rate::RateAdapter;
#[cfg(all(
    feature = "embedded-math",
    target_os = "none",
    not(feature = "daisy-full-rate")
))]
pub(crate) use half_rate::RateAdapter;

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
    use crate::dsp::upsampler::Upsampler;

    #[derive(Default)]
    pub(crate) struct RateAdapter {
        upsampler: Upsampler,
    }

    impl RateAdapter {
        pub(crate) fn internal_sample_rate(external_sample_rate: f32) -> f32 {
            (external_sample_rate * 0.5).max(1.0)
        }

        pub(crate) const fn needs_render(&self) -> bool {
            self.upsampler.needs_input()
        }

        pub(crate) fn submit(&mut self, frame: (f32, f32)) {
            self.upsampler.submit(frame);
        }

        pub(crate) fn output(&self) -> (f32, f32) {
            self.upsampler.output()
        }

        pub(crate) fn advance(&mut self) {
            self.upsampler.advance();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::half_rate::RateAdapter;

    #[test]
    fn half_rate_internal_sample_rate_is_half_the_external_rate() {
        assert_eq!(RateAdapter::internal_sample_rate(48_000.0), 24_000.0);
    }

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
}
