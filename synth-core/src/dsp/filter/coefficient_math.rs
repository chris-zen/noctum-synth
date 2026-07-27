//! Target-specific coefficient math hidden behind filter-level intent.

use crate::math::WideF32;

use super::MIN_CUTOFF_HZ;

/// Cutoff pitch modulation prepared by the signal-routing layer.
///
/// `uniform` is semantic information about the signal, not a backend choice.
/// Backends remain free to ignore it.
#[derive(Clone, Copy)]
pub(super) struct PreparedCutoffModulation {
    lanes: WideF32,
    #[allow(dead_code)]
    uniform: Option<f32>,
}

impl PreparedCutoffModulation {
    #[inline(always)]
    pub(super) fn new(lanes: WideF32, uniform: Option<f32>) -> Self {
        Self { lanes, uniform }
    }

    #[inline(always)]
    pub(super) fn lanes(self) -> WideF32 {
        self.lanes
    }

    #[inline(always)]
    #[allow(dead_code)]
    fn uniform(self) -> Option<f32> {
        self.uniform
    }
}

/// Converts a base cutoff and per-lane pitch offsets into TPT coefficients.
///
/// The filter algorithm deliberately does not choose between scalar and SIMD
/// execution. Target-specific selection stays inside this module.
#[inline(always)]
pub(super) fn modulated_tpt_coefficient(
    base_cutoff_hz: f32,
    modulation: PreparedCutoffModulation,
    maximum_cutoff_hz: f32,
    processing_rate_hz: f32,
) -> WideF32 {
    backend::modulated_tpt_coefficient(
        base_cutoff_hz,
        modulation,
        maximum_cutoff_hz,
        processing_rate_hz,
    )
}

#[cfg(feature = "fast-math")]
mod backend {
    use super::*;

    #[inline(always)]
    pub(super) fn modulated_tpt_coefficient(
        base_cutoff_hz: f32,
        modulation: PreparedCutoffModulation,
        maximum_cutoff_hz: f32,
        processing_rate_hz: f32,
    ) -> WideF32 {
        embedded_coefficient(
            base_cutoff_hz,
            modulation,
            maximum_cutoff_hz,
            processing_rate_hz,
        )
    }
}

#[cfg(not(feature = "fast-math"))]
mod backend {
    use super::*;

    #[inline(always)]
    pub(super) fn modulated_tpt_coefficient(
        base_cutoff_hz: f32,
        modulation: PreparedCutoffModulation,
        maximum_cutoff_hz: f32,
        processing_rate_hz: f32,
    ) -> WideF32 {
        vector_coefficient(
            base_cutoff_hz,
            modulation.lanes(),
            maximum_cutoff_hz,
            processing_rate_hz,
        )
    }
}

#[inline(always)]
#[cfg(any(test, feature = "fast-math"))]
pub(super) fn embedded_coefficient(
    base_cutoff_hz: f32,
    modulation: PreparedCutoffModulation,
    maximum_cutoff_hz: f32,
    processing_rate_hz: f32,
) -> WideF32 {
    if let Some(uniform) = modulation.uniform() {
        return WideF32::splat(lookup_coefficient(
            base_cutoff_hz,
            uniform,
            maximum_cutoff_hz,
            processing_rate_hz,
        ));
    }

    vector_coefficient(
        base_cutoff_hz,
        modulation.lanes(),
        maximum_cutoff_hz,
        processing_rate_hz,
    )
}

#[inline(always)]
#[cfg(any(test, feature = "fast-math"))]
fn lookup_coefficient(
    base_cutoff_hz: f32,
    modulation_semitones: f32,
    maximum_cutoff_hz: f32,
    processing_rate_hz: f32,
) -> f32 {
    let scale = embedded_exp2(modulation_semitones * (1.0 / 12.0));
    let cutoff_hz = (base_cutoff_hz * scale).clamp(MIN_CUTOFF_HZ, maximum_cutoff_hz);
    let normalized = cutoff_hz / processing_rate_hz;
    if normalized < 0.15 {
        return low_cutoff_coefficient(normalized);
    }
    let position = normalized
        * ((super::prewarp_table::COEFFICIENTS.len() - 1) as f32
            / super::prewarp_table::MAX_NORMALIZED_CUTOFF);
    let index = position as usize;
    if index >= super::prewarp_table::COEFFICIENTS.len() - 1 {
        return super::prewarp_table::COEFFICIENTS[super::prewarp_table::COEFFICIENTS.len() - 1];
    }
    let lower = super::prewarp_table::COEFFICIENTS[index];
    let upper = super::prewarp_table::COEFFICIENTS[index + 1];
    lower + (upper - lower) * (position - index as f32)
}

#[inline(always)]
#[cfg(any(test, feature = "fast-math"))]
fn low_cutoff_coefficient(normalized_cutoff: f32) -> f32 {
    let angle = core::f32::consts::PI * normalized_cutoff;
    angle
        * (0.99999896
            + angle
                * (-0.99987155
                    + angle
                        * (1.3294426
                            + angle
                                * (-1.6169741
                                    + angle
                                        * (1.8051608 + angle * (-1.4739962 + angle * 0.6103134))))))
}

#[inline(always)]
#[cfg(any(test, feature = "fast-math"))]
fn embedded_exp2(value: f32) -> f32 {
    // Centered range reduction keeps the fifth-order exponential polynomial
    // on [-ln(2)/2, ln(2)/2]. The modulation domain only needs small integer
    // powers, so constructing 2^n is exact.
    let exponent = if value >= 0.0 {
        (value + 0.5) as i32
    } else {
        (value - 0.5) as i32
    };
    let fraction = value - exponent as f32;
    let x = fraction * core::f32::consts::LN_2;
    let exp_fraction = 1.0
        + x * (1.0
            + x * (0.5
                + x * (1.0 / 6.0
                    + x * (1.0 / 24.0
                        + x * (1.0 / 120.0 + x * (1.0 / 720.0 + x * (1.0 / 5040.0)))))));
    let exponent_bits = ((exponent.clamp(-126, 127) + 127) as u32) << 23;
    f32::from_bits(exponent_bits) * exp_fraction
}

#[inline(always)]
pub(super) fn vector_coefficient(
    base_cutoff_hz: f32,
    modulation_semitones: WideF32,
    maximum_cutoff_hz: f32,
    processing_rate_hz: f32,
) -> WideF32 {
    let scale = (modulation_semitones * WideF32::splat(1.0 / 12.0)).exp2();
    coefficients_from_cutoff(
        (WideF32::splat(base_cutoff_hz) * scale).clamp(
            WideF32::splat(MIN_CUTOFF_HZ),
            WideF32::splat(maximum_cutoff_hz),
        ),
        maximum_cutoff_hz,
        processing_rate_hz,
    )
}

#[inline(always)]
fn coefficients_from_cutoff(
    cutoff_hz: WideF32,
    maximum_cutoff_hz: f32,
    processing_rate_hz: f32,
) -> WideF32 {
    let mut angles = cutoff_hz.to_array();
    for angle in &mut angles {
        let hz = angle.clamp(MIN_CUTOFF_HZ, maximum_cutoff_hz);
        *angle = core::f32::consts::PI * hz / processing_rate_hz;
    }
    let raw = WideF32::new(angles).tan();
    raw / (WideF32::splat(1.0) + raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::F32;

    fn scalar_reference_coefficient(
        base_cutoff_hz: f32,
        modulation_semitones: f32,
        maximum_cutoff_hz: f32,
        processing_rate_hz: f32,
    ) -> f32 {
        let scale = F32(modulation_semitones * (1.0 / 12.0)).exp2().as_f32();
        let cutoff_hz = (base_cutoff_hz * scale).clamp(MIN_CUTOFF_HZ, maximum_cutoff_hz);
        let raw = F32(core::f32::consts::PI * cutoff_hz / processing_rate_hz)
            .tan()
            .as_f32();
        raw / (1.0 + raw)
    }

    #[test]
    fn vector_backend_preserves_previous_operation_order_bit_exactly() {
        for sample_rate in [32_000.0f32, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            let maximum = (sample_rate * 0.45).min(super::super::MAX_CUTOFF_HZ);
            for index in 0..4096 {
                let base = 20.0 + (maximum - 20.0) * index as f32 / 4095.0;
                let modulation = WideF32::new(core::array::from_fn(|i| {
                    [-48.0 + index as f32 * (96.0 / 4095.0), -24.0, 12.0, 47.0][i % 4]
                }));
                let expected = previous_vector_coefficient(base, modulation, maximum, sample_rate);
                let actual = vector_coefficient(base, modulation, maximum, sample_rate);
                assert_eq!(
                    actual.to_array().map(f32::to_bits),
                    expected.to_array().map(f32::to_bits)
                );
            }
        }
    }

    #[test]
    fn embedded_uniform_path_stays_within_coefficient_and_pitch_bounds() {
        let mut maximum_coefficient_error = 0.0f32;
        let mut maximum_pitch_error_cents = 0.0f32;
        let mut worst_pitch_case = (0.0f32, 0.0f32, 0.0f32);
        for sample_rate in [32_000.0f32, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            let maximum = (sample_rate * 0.45).min(super::super::MAX_CUTOFF_HZ);
            for index in 0..=16_384 {
                let base = 20.0 + (maximum - 20.0) * index as f32 / 16_384.0;
                let semitones = -48.0 + 96.0 * index as f32 / 16_384.0;
                let modulation =
                    PreparedCutoffModulation::new(WideF32::splat(semitones), Some(semitones));
                let actual =
                    embedded_coefficient(base, modulation, maximum, sample_rate).to_array()[0];
                let expected = scalar_reference_coefficient(base, semitones, maximum, sample_rate);
                maximum_coefficient_error =
                    maximum_coefficient_error.max((actual - expected).abs());

                let actual_hz =
                    libm::atanf(actual / (1.0 - actual)) * sample_rate / core::f32::consts::PI;
                let expected_hz =
                    (base * libm::exp2f(semitones / 12.0)).clamp(MIN_CUTOFF_HZ, maximum);
                let cents = 1200.0 * libm::log2f(actual_hz / expected_hz).abs();
                if cents > maximum_pitch_error_cents {
                    maximum_pitch_error_cents = cents;
                    worst_pitch_case = (sample_rate, base, semitones);
                }
            }
        }
        assert!(
            maximum_coefficient_error <= 2.0e-6,
            "maximum coefficient error: {maximum_coefficient_error}"
        );
        assert!(
            maximum_pitch_error_cents <= 0.05,
            "maximum pitch error: {maximum_pitch_error_cents} cents at {worst_pitch_case:?}"
        );
    }

    #[test]
    fn embedded_nonuniform_path_is_vector_bit_exact() {
        let modulation = WideF32::new(core::array::from_fn(|i| [-12.0, -6.0, 3.0, 18.0][i % 4]));
        let expected = vector_coefficient(1_200.0, modulation, 18_000.0, 48_000.0);
        let actual = embedded_coefficient(
            1_200.0,
            PreparedCutoffModulation::new(modulation, None),
            18_000.0,
            48_000.0,
        );
        assert_eq!(
            actual.to_array().map(f32::to_bits),
            expected.to_array().map(f32::to_bits)
        );
    }

    fn previous_vector_coefficient(
        base_cutoff_hz: f32,
        modulation_semitones: WideF32,
        maximum_cutoff_hz: f32,
        processing_rate_hz: f32,
    ) -> WideF32 {
        let scale = (modulation_semitones * WideF32::splat(1.0 / 12.0)).exp2();
        let cutoff = (WideF32::splat(base_cutoff_hz) * scale).clamp(
            WideF32::splat(MIN_CUTOFF_HZ),
            WideF32::splat(maximum_cutoff_hz),
        );
        let mut values = cutoff.to_array();
        for value in &mut values {
            let hz = value.clamp(MIN_CUTOFF_HZ, maximum_cutoff_hz);
            *value = core::f32::consts::PI * hz / processing_rate_hz;
        }
        let raw = WideF32::new(values).tan();
        raw / (WideF32::splat(1.0) + raw)
    }
}
