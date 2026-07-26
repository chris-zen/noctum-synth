//! Resonance-control shaping selected at the filter subsystem boundary.

#[inline(always)]
pub(super) fn shape(value: f32) -> f32 {
    backend::shape(value)
}

#[cfg(target_arch = "arm")]
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

#[cfg(not(target_arch = "arm"))]
mod backend {
    use crate::dsp::filter::RESONANCE_CONTROL_EXPONENT;
    use crate::math::F32;

    #[inline(always)]
    pub(super) fn shape(value: f32) -> f32 {
        F32(value).powf(F32(RESONANCE_CONTROL_EXPONENT)).as_f32()
    }
}

#[cfg(test)]
mod tests {
    use crate::dsp::filter::RESONANCE_CONTROL_EXPONENT;

    #[test]
    fn hardware_sqrt_form_tracks_reference_power() {
        let mut maximum_error = 0.0_f32;
        for index in 0..=65_536 {
            let value = index as f32 / 65_536.0;
            let square_root = value.sqrt();
            let actual = value * square_root * square_root.sqrt();
            let expected = libm::powf(value, RESONANCE_CONTROL_EXPONENT);
            maximum_error = maximum_error.max((actual - expected).abs());
        }
        assert!(maximum_error <= 2.0e-7, "maximum error={maximum_error}");
    }
}
