//! Platform-neutral floating-point math wrappers.
//!
//! Desktop builds use `libm` for predictable accuracy. The `embedded-math`
//! feature selectively enables approximations only where target benchmarks and
//! DSP error bounds justify them.

#[inline]
pub(crate) fn exp(x: f32) -> f32 {
    #[cfg(feature = "embedded-math")]
    {
        ::micromath::F32Ext::exp(x)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        libm::expf(x)
    }
}

#[inline]
pub(crate) fn ln(x: f32) -> f32 {
    #[cfg(feature = "embedded-math")]
    {
        ::micromath::F32Ext::ln(x)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        libm::logf(x)
    }
}

#[inline]
pub(crate) fn powf(x: f32, y: f32) -> f32 {
    #[cfg(feature = "embedded-math")]
    {
        ::micromath::F32Ext::powf(x, y)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        libm::powf(x, y)
    }
}

#[inline]
pub(crate) fn exp2(x: f32) -> f32 {
    #[cfg(feature = "embedded-math")]
    {
        crate::micromath::scalar_exp2(x)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        // Preserve the desktop operation used before the embedded exp2
        // specialization so existing renders remain bit-identical.
        libm::powf(2.0, x)
    }
}

#[inline]
pub(crate) fn round(x: f32) -> f32 {
    #[cfg(feature = "embedded-math")]
    {
        ::micromath::F32Ext::round(x)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        libm::roundf(x)
    }
}

#[inline]
pub(crate) fn floor(x: f32) -> f32 {
    #[cfg(feature = "embedded-math")]
    {
        ::micromath::F32Ext::floor(x)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        libm::floorf(x)
    }
}

#[inline]
pub(crate) fn tan(x: f32) -> f32 {
    #[cfg(feature = "embedded-math")]
    {
        crate::micromath::scalar_tan(x)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        libm::tanf(x)
    }
}

#[inline]
pub(crate) fn effect_sin(x: f32) -> f32 {
    #[cfg(feature = "embedded-math")]
    {
        ::micromath::F32Ext::sin(x)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        libm::sinf(x)
    }
}

#[inline]
pub(crate) fn tanh(x: f32) -> f32 {
    #[cfg(feature = "embedded-math")]
    {
        crate::micromath::scalar_tanh(x)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        libm::tanhf(x)
    }
}

#[cfg(test)]
mod tests {
    use crate::f32x4;

    #[test]
    #[cfg(feature = "embedded-math")]
    fn embedded_sine_stays_within_micromath_error_bound() {
        const SAMPLES: usize = 16_384;
        let mut maximum_error = 0.0f32;
        for index in 0..=SAMPLES {
            let phase = index as f32 / SAMPLES as f32;
            let angle = crate::TAU * phase;
            maximum_error = maximum_error.max((super::effect_sin(angle) - libm::sinf(angle)).abs());
        }

        assert!(
            maximum_error <= 0.002,
            "micromath documents a maximum sine error of 0.002; measured {maximum_error}"
        );
    }

    #[test]
    #[cfg(feature = "embedded-math")]
    fn embedded_sine_is_continuous_across_phase_wrap() {
        let step = crate::TAU / 48_000.0;
        let before_wrap = super::effect_sin(crate::TAU - step);
        let after_wrap = super::effect_sin(0.0);

        assert!(
            (after_wrap - before_wrap).abs() <= step + 0.002,
            "phase wrap introduced a discontinuity: before {before_wrap}, after {after_wrap}"
        );
    }

    #[test]
    fn vector_tangent_preserves_filter_prewarp_pitch() {
        const SAMPLE_RATES: [f32; 5] = [32_000.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0];
        const SAMPLES: usize = 16_384;
        let mut maximum_coefficient_error = 0.0f32;
        let mut maximum_pitch_error_cents = 0.0f32;

        for sample_rate in SAMPLE_RATES {
            let max_cutoff = (sample_rate * 0.45).min(20_000.0);
            for start in (0..=SAMPLES).step_by(4) {
                let cutoffs = core::array::from_fn(|lane| {
                    let index = (start + lane).min(SAMPLES);
                    20.0 + (max_cutoff - 20.0) * index as f32 / SAMPLES as f32
                });
                let angles = f32x4::new(cutoffs.map(|hz| core::f32::consts::PI * hz / sample_rate));
                let actual_g = angles.tan().to_array();

                for lane in 0..4 {
                    let expected_g = libm::tanf(angles.to_array()[lane]);
                    let actual_coefficient = actual_g[lane] / (1.0 + actual_g[lane]);
                    let expected_coefficient = expected_g / (1.0 + expected_g);
                    maximum_coefficient_error = maximum_coefficient_error
                        .max((actual_coefficient - expected_coefficient).abs());

                    let actual_hz =
                        libm::atanf(actual_g[lane]) * sample_rate / core::f32::consts::PI;
                    let pitch_error_cents = 1200.0 * libm::logf(actual_hz / cutoffs[lane]).abs()
                        / core::f32::consts::LN_2;
                    maximum_pitch_error_cents = maximum_pitch_error_cents.max(pitch_error_cents);
                }
            }
        }

        assert!(
            maximum_coefficient_error <= 2.0e-6,
            "filter coefficient error {maximum_coefficient_error} exceeds tolerance"
        );
        assert!(
            maximum_pitch_error_cents <= 0.05,
            "filter prewarp pitch error {maximum_pitch_error_cents} cents exceeds tolerance"
        );
    }
}
