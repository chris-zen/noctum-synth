//! Resonance-control shaping selected at the filter subsystem boundary.

#[cfg(any(test, not(all(feature = "embedded-math", target_os = "none"))))]
use super::RESONANCE_CONTROL_EXPONENT;

#[inline(always)]
pub(super) fn shape(value: f32) -> f32 {
    backend::shape(value)
}

#[cfg(all(feature = "embedded-math", target_os = "none"))]
mod backend {
    #[inline(always)]
    fn hardware_sqrt(mut value: f32) -> f32 {
        // SAFETY: VSQRT.F32 has no memory effects and the selected Daisy target
        // always has the FPv5 single-precision register file enabled.
        unsafe {
            core::arch::asm!(
                "vsqrt.f32 {value}, {value}",
                value = inout(sreg) value,
                options(pure, nomem, nostack)
            );
        }
        value
    }

    /// `x^1.75 == x * sqrt(x) * sqrt(sqrt(x))` on `[0, 1]`.
    ///
    /// LLVM lowers the square roots to the Cortex-M7 FPv5 `VSQRT.F32`
    /// instruction, avoiding a general logarithm/exponential power function.
    #[inline(always)]
    pub(super) fn shape(value: f32) -> f32 {
        let square_root = hardware_sqrt(value);
        value * square_root * hardware_sqrt(square_root)
    }
}

#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
mod backend {
    use super::RESONANCE_CONTROL_EXPONENT;

    #[inline(always)]
    pub(super) fn shape(value: f32) -> f32 {
        crate::math::powf(value, RESONANCE_CONTROL_EXPONENT)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hardware_sqrt_form_tracks_reference_power() {
        let mut maximum_error = 0.0_f32;
        for index in 0..=65_536 {
            let value = index as f32 / 65_536.0;
            let square_root = value.sqrt();
            let actual = value * square_root * square_root.sqrt();
            let expected = libm::powf(value, super::RESONANCE_CONTROL_EXPONENT);
            maximum_error = maximum_error.max((actual - expected).abs());
        }
        assert!(maximum_error <= 2.0e-7, "maximum error={maximum_error}");
    }
}
