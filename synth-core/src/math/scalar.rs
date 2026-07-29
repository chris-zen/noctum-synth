use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct F32(pub f32);

impl From<f32> for F32 {
    #[inline]
    fn from(v: f32) -> Self {
        Self(v)
    }
}

impl From<F32> for f32 {
    #[inline]
    fn from(v: F32) -> Self {
        v.0
    }
}

macro_rules! impl_binop {
    ($trait:ident, $method:ident, $op:tt) => {
        impl $trait<F32> for F32 {
            type Output = F32;
            #[inline]
            fn $method(self, rhs: F32) -> F32 {
                F32(self.0 $op rhs.0)
            }
        }
        impl $trait<f32> for F32 {
            type Output = F32;
            #[inline]
            fn $method(self, rhs: f32) -> F32 {
                F32(self.0 $op rhs)
            }
        }
        impl $trait<F32> for f32 {
            type Output = F32;
            #[inline]
            fn $method(self, rhs: F32) -> F32 {
                F32(self $op rhs.0)
            }
        }
    };
}

macro_rules! impl_assign_op {
    ($trait:ident, $method:ident, $op:tt) => {
        impl $trait<F32> for F32 {
            #[inline]
            fn $method(&mut self, rhs: F32) {
                self.0 $op rhs.0;
            }
        }
        impl $trait<f32> for F32 {
            #[inline]
            fn $method(&mut self, rhs: f32) {
                self.0 $op rhs;
            }
        }
    };
}

impl_binop!(Add, add, +);
impl_binop!(Sub, sub, -);
impl_binop!(Mul, mul, *);
impl_binop!(Div, div, /);

impl_assign_op!(AddAssign, add_assign, +=);
impl_assign_op!(SubAssign, sub_assign, -=);
impl_assign_op!(MulAssign, mul_assign, *=);
impl_assign_op!(DivAssign, div_assign, /=);

impl Neg for F32 {
    type Output = F32;
    #[inline]
    fn neg(self) -> F32 {
        F32(-self.0)
    }
}

impl F32 {
    #[inline]
    pub fn as_f32(self) -> f32 {
        self.0
    }

    #[inline]
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }

    #[inline]
    pub fn floor(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(if self.0 == 0.0 || !self.0.is_finite() {
                self.0
            } else {
                ::micromath::F32Ext::floor(self.0)
            })
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::floorf(self.0))
        }
    }

    #[inline]
    pub fn round(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(::micromath::F32Ext::round(self.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::roundf(self.0))
        }
    }

    #[inline]
    pub fn exp(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(::micromath::F32Ext::exp(self.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::expf(self.0))
        }
    }

    #[inline]
    pub fn ln(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(::micromath::F32Ext::ln(self.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::logf(self.0))
        }
    }

    #[inline]
    pub fn accurate_ln(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(super::micro::scalar_ln(self.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::logf(self.0))
        }
    }

    #[inline]
    pub fn accurate_log2(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(super::micro::scalar_log2(self.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::log2f(self.0))
        }
    }

    #[inline]
    pub fn powf(self, y: Self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(::micromath::F32Ext::powf(self.0, y.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::powf(self.0, y.0))
        }
    }

    #[inline]
    pub fn exp2(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(super::micro::scalar_exp2(self.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::powf(2.0, self.0))
        }
    }

    #[inline]
    pub fn sin(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(::micromath::F32Ext::sin(self.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::sinf(self.0))
        }
    }

    #[inline]
    pub fn cos(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(::micromath::F32Ext::cos(self.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::cosf(self.0))
        }
    }

    #[inline]
    pub fn tan(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(super::micro::scalar_tan(self.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::tanf(self.0))
        }
    }

    #[inline]
    pub fn tanh(self) -> Self {
        #[cfg(feature = "fast-math")]
        {
            Self(super::micro::scalar_tanh(self.0))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            Self(libm::tanhf(self.0))
        }
    }

    #[inline]
    pub fn sin_cos(self) -> (Self, Self) {
        #[cfg(feature = "fast-math")]
        {
            let (s, c) = ::micromath::F32Ext::sin_cos(self.0);
            (Self(s), Self(c))
        }
        #[cfg(not(feature = "fast-math"))]
        {
            (Self(libm::sinf(self.0)), Self(libm::cosf(self.0)))
        }
    }

    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self(self.0.clamp(min.0, max.0))
    }

    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

#[cfg(test)]
mod tests {
    use super::F32;

    #[test]
    fn arithmetic_with_f32_is_commutative() {
        let x = F32(3.0);
        assert_eq!((x + 1.0).0, 4.0);
        assert_eq!((1.0 + x).0, 4.0);
        assert_eq!((x * 2.0).0, 6.0);
        assert_eq!((2.0 * x).0, 6.0);
        assert_eq!((-x).0, -3.0);
    }

    #[test]
    fn assign_ops_mutate_in_place() {
        let mut x = F32(3.0);
        x += 1.0;
        assert_eq!(x.0, 4.0);
        x *= 0.5;
        assert_eq!(x.0, 2.0);
        x -= F32(1.0);
        assert_eq!(x.0, 1.0);
        x /= 2.0;
        assert_eq!(x.0, 0.5);
    }

    #[test]
    fn conversions_roundtrip() {
        let a: f32 = 2.718;
        let x: F32 = a.into();
        let b: f32 = x.into();
        assert_eq!(a, b);
        assert_eq!(F32(a).0, a);
    }

    #[test]
    fn abs_floor_round() {
        assert_eq!(F32(-3.5).abs().0, 3.5);
        assert_eq!(F32(3.14).floor().0, 3.0);
        assert_eq!(F32(3.7).round().0, 4.0);
        assert_eq!(F32(-3.7).round().0, -4.0);
    }

    #[test]
    fn clamp_min_max() {
        let x = F32(5.0);
        assert_eq!(x.clamp(F32(0.0), F32(3.0)).0, 3.0);
        assert_eq!(x.clamp(F32(10.0), F32(20.0)).0, 10.0);
        assert_eq!(x.max(F32(2.0)).0, 5.0);
        assert_eq!(x.min(F32(2.0)).0, 2.0);
    }

    #[test]
    fn is_finite_detects_inf_and_nan() {
        assert!(F32(1.0).is_finite());
        assert!(!F32(f32::NAN).is_finite());
        assert!(!F32(f32::INFINITY).is_finite());
    }

    #[test]
    fn exp_and_ln_are_rough_inverses() {
        let x = F32(2.0);
        let result = x.ln().exp().0;
        assert!((result - 2.0).abs() < 1e-2, "embedded exp(ln(x)) error");
    }

    #[test]
    fn powf_matches_libm() {
        let result = F32(2.0).powf(F32(3.0)).0;
        #[cfg(feature = "fast-math")]
        let tol = 1e-1;
        #[cfg(not(feature = "fast-math"))]
        let tol = 1e-6;
        let expected = libm::powf(2.0, 3.0);
        assert!((result - expected).abs() < tol);
    }

    #[test]
    fn exp2_is_power_of_two() {
        let result = F32(3.0).exp2().0;
        assert!((result - 8.0).abs() < 2e-3);
    }

    #[test]
    fn accurate_log2_matches_integer_powers() {
        assert!((F32(8.0).accurate_log2().0 - 3.0).abs() < 1e-5);
        assert!((F32(0.25).accurate_log2().0 - (-2.0)).abs() < 1e-5);
        let c4_ratio = F32(261.62555 / 440.0).accurate_log2().0;
        assert!((c4_ratio - (-0.75)).abs() < 1e-5);
    }

    #[test]
    fn accurate_ln_matches_libm_at_pitch_ratios() {
        let ratio = 261.62555 / 440.0;
        let expected = libm::logf(ratio);
        assert!((F32(ratio).accurate_ln().0 - expected).abs() < 1e-5);
        assert!((F32(2.0).accurate_ln().0 - libm::logf(2.0)).abs() < 1e-5);
    }

    #[test]
    fn sin_and_cos_orthonormal() {
        let angle = F32(core::f32::consts::PI / 4.0);
        let (s, c) = angle.sin_cos();
        assert!((s.0.powi(2) + c.0.powi(2) - 1.0).abs() < 2e-3);
    }

    #[test]
    fn tan_at_pi_over_four_is_one() {
        let result = F32(core::f32::consts::PI / 4.0).tan().0;
        assert!((result - 1.0).abs() < 1e-4);
    }

    #[test]
    fn tanh_is_odd_bounded_and_monotonic() {
        let a = F32(0.5).tanh().0;
        let b = F32(1.0).tanh().0;
        let neg = F32(-0.5).tanh().0;
        assert!(a > 0.0 && a < 1.0);
        assert!(b > a);
        assert!((neg + a).abs() < 1e-5);
    }

    #[test]
    fn special_values_are_finite() {
        assert!(F32(0.0).floor().0 == 0.0);
        assert!(F32(0.0).exp().0 == 1.0);
        assert!(F32(1.0).ln().0 == 0.0);
    }
}
