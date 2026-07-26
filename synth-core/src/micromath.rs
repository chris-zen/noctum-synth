//! Scalar four-lane math used on targets without a floating-point SIMD unit.

use core::ops::{
    Add, AddAssign, BitAnd, BitXor, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign,
};

/// Four independent `f32` lanes with the subset of `wide::f32x4` used by the synth.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C, align(16))]
pub struct f32x4([f32; 4]);

impl f32x4 {
    pub const ZERO: Self = Self([0.0; 4]);

    #[inline]
    pub const fn new(lanes: [f32; 4]) -> Self {
        Self(lanes)
    }

    #[inline]
    pub const fn splat(value: f32) -> Self {
        Self([value; 4])
    }

    #[inline]
    pub const fn to_array(self) -> [f32; 4] {
        self.0
    }

    #[inline]
    pub fn abs(self) -> Self {
        Self([
            self.0[0].abs(),
            self.0[1].abs(),
            self.0[2].abs(),
            self.0[3].abs(),
        ])
    }

    #[inline]
    pub fn floor(self) -> Self {
        Self([
            embedded_floor(self.0[0]),
            embedded_floor(self.0[1]),
            embedded_floor(self.0[2]),
            embedded_floor(self.0[3]),
        ])
    }

    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self([
            self.0[0].min(other.0[0]),
            self.0[1].min(other.0[1]),
            self.0[2].min(other.0[2]),
            self.0[3].min(other.0[3]),
        ])
    }

    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self([
            self.0[0].max(other.0[0]),
            self.0[1].max(other.0[1]),
            self.0[2].max(other.0[2]),
            self.0[3].max(other.0[3]),
        ])
    }

    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        self.max(min).min(max)
    }

    #[inline]
    pub fn is_finite(self) -> Self {
        Self::mask([
            self.0[0].is_finite(),
            self.0[1].is_finite(),
            self.0[2].is_finite(),
            self.0[3].is_finite(),
        ])
    }

    #[inline]
    pub fn simd_lt(self, other: Self) -> Self {
        Self::mask([
            self.0[0] < other.0[0],
            self.0[1] < other.0[1],
            self.0[2] < other.0[2],
            self.0[3] < other.0[3],
        ])
    }

    #[inline]
    pub fn simd_gt(self, other: Self) -> Self {
        Self::mask([
            self.0[0] > other.0[0],
            self.0[1] > other.0[1],
            self.0[2] > other.0[2],
            self.0[3] > other.0[3],
        ])
    }

    #[inline]
    pub fn simd_ge(self, other: Self) -> Self {
        Self::mask([
            self.0[0] >= other.0[0],
            self.0[1] >= other.0[1],
            self.0[2] >= other.0[2],
            self.0[3] >= other.0[3],
        ])
    }

    #[inline]
    pub fn blend(self, if_true: Self, if_false: Self) -> Self {
        Self([
            if Self::mask_lane(self.0[0]) {
                if_true.0[0]
            } else {
                if_false.0[0]
            },
            if Self::mask_lane(self.0[1]) {
                if_true.0[1]
            } else {
                if_false.0[1]
            },
            if Self::mask_lane(self.0[2]) {
                if_true.0[2]
            } else {
                if_false.0[2]
            },
            if Self::mask_lane(self.0[3]) {
                if_true.0[3]
            } else {
                if_false.0[3]
            },
        ])
    }

    #[inline]
    pub fn all(self) -> bool {
        Self::mask_lane(self.0[0])
            && Self::mask_lane(self.0[1])
            && Self::mask_lane(self.0[2])
            && Self::mask_lane(self.0[3])
    }

    #[inline]
    pub fn any(self) -> bool {
        Self::mask_lane(self.0[0])
            || Self::mask_lane(self.0[1])
            || Self::mask_lane(self.0[2])
            || Self::mask_lane(self.0[3])
    }

    #[inline]
    pub fn reduce_add(self) -> f32 {
        (self.0[0] + self.0[1]) + (self.0[2] + self.0[3])
    }

    #[inline]
    pub fn reduce_mean(self) -> f32 {
        self.reduce_add() / 4.0
    }

    #[inline]
    pub fn exp2(self) -> Self {
        Self([
            accurate_exp2(self.0[0]),
            accurate_exp2(self.0[1]),
            accurate_exp2(self.0[2]),
            accurate_exp2(self.0[3]),
        ])
    }

    #[inline]
    pub fn sin_cos(self) -> (Self, Self) {
        let lane0 = ::micromath::F32Ext::sin_cos(self.0[0]);
        let lane1 = ::micromath::F32Ext::sin_cos(self.0[1]);
        let lane2 = ::micromath::F32Ext::sin_cos(self.0[2]);
        let lane3 = ::micromath::F32Ext::sin_cos(self.0[3]);
        (
            Self([lane0.0, lane1.0, lane2.0, lane3.0]),
            Self([lane0.1, lane1.1, lane2.1, lane3.1]),
        )
    }

    #[inline]
    pub fn tan(self) -> Self {
        Self([
            accurate_tan(self.0[0]),
            accurate_tan(self.0[1]),
            accurate_tan(self.0[2]),
            accurate_tan(self.0[3]),
        ])
    }

    #[inline]
    pub fn tanh(self) -> Self {
        Self([
            rational_tanh(self.0[0]),
            rational_tanh(self.0[1]),
            rational_tanh(self.0[2]),
            rational_tanh(self.0[3]),
        ])
    }

    #[inline]
    fn mask(lanes: [bool; 4]) -> Self {
        Self([
            f32::from_bits(if lanes[0] { u32::MAX } else { 0 }),
            f32::from_bits(if lanes[1] { u32::MAX } else { 0 }),
            f32::from_bits(if lanes[2] { u32::MAX } else { 0 }),
            f32::from_bits(if lanes[3] { u32::MAX } else { 0 }),
        ])
    }

    #[inline]
    fn mask_lane(value: f32) -> bool {
        value.to_bits() & 0x8000_0000 != 0
    }
}

impl From<[f32; 4]> for f32x4 {
    #[inline]
    fn from(value: [f32; 4]) -> Self {
        Self::new(value)
    }
}

impl From<f32x4> for [f32; 4] {
    #[inline]
    fn from(value: f32x4) -> Self {
        value.to_array()
    }
}

impl crate::F32x4Ext for f32x4 {
    #[inline(always)]
    fn replace_lane(mut self, lane: usize, value: f32) -> Self {
        debug_assert!(lane < 4);
        self.0[lane] = value;
        self
    }
}

macro_rules! impl_vector_operator {
    ($trait:ident, $method:ident, $operator:tt) => {
        impl $trait for f32x4 {
            type Output = Self;

            #[inline]
            fn $method(self, other: Self) -> Self {
                Self([
                    self.0[0] $operator other.0[0],
                    self.0[1] $operator other.0[1],
                    self.0[2] $operator other.0[2],
                    self.0[3] $operator other.0[3],
                ])
            }
        }

        impl $trait<f32> for f32x4 {
            type Output = Self;

            #[inline]
            fn $method(self, other: f32) -> Self {
                Self([
                    self.0[0] $operator other,
                    self.0[1] $operator other,
                    self.0[2] $operator other,
                    self.0[3] $operator other,
                ])
            }
        }

        impl $trait<f32x4> for f32 {
            type Output = f32x4;

            #[inline]
            fn $method(self, other: f32x4) -> f32x4 {
                f32x4([
                    self $operator other.0[0],
                    self $operator other.0[1],
                    self $operator other.0[2],
                    self $operator other.0[3],
                ])
            }
        }
    };
}

impl_vector_operator!(Add, add, +);
impl_vector_operator!(Sub, sub, -);
impl_vector_operator!(Mul, mul, *);
impl_vector_operator!(Div, div, /);

macro_rules! impl_assign_operator {
    ($trait:ident, $method:ident, $operator:tt) => {
        impl $trait for f32x4 {
            #[inline]
            fn $method(&mut self, other: Self) {
                self.0[0] $operator other.0[0];
                self.0[1] $operator other.0[1];
                self.0[2] $operator other.0[2];
                self.0[3] $operator other.0[3];
            }
        }

        impl $trait<f32> for f32x4 {
            #[inline]
            fn $method(&mut self, other: f32) {
                self.0[0] $operator other;
                self.0[1] $operator other;
                self.0[2] $operator other;
                self.0[3] $operator other;
            }
        }
    };
}

impl_assign_operator!(AddAssign, add_assign, +=);
impl_assign_operator!(SubAssign, sub_assign, -=);
impl_assign_operator!(MulAssign, mul_assign, *=);
impl_assign_operator!(DivAssign, div_assign, /=);

impl Neg for f32x4 {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self {
        Self([-self.0[0], -self.0[1], -self.0[2], -self.0[3]])
    }
}

impl BitAnd for f32x4 {
    type Output = Self;

    #[inline]
    fn bitand(self, other: Self) -> Self {
        Self([
            f32::from_bits(self.0[0].to_bits() & other.0[0].to_bits()),
            f32::from_bits(self.0[1].to_bits() & other.0[1].to_bits()),
            f32::from_bits(self.0[2].to_bits() & other.0[2].to_bits()),
            f32::from_bits(self.0[3].to_bits() & other.0[3].to_bits()),
        ])
    }
}

/// Integer lanes needed by the white-noise generator.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C, align(16))]
pub(crate) struct i32x4([i32; 4]);

impl i32x4 {
    #[inline]
    pub(crate) const fn new(lanes: [i32; 4]) -> Self {
        Self(lanes)
    }

    #[inline]
    pub(crate) fn round_float(self) -> f32x4 {
        f32x4::new([
            self.0[0] as f32,
            self.0[1] as f32,
            self.0[2] as f32,
            self.0[3] as f32,
        ])
    }
}

impl Add for i32x4 {
    type Output = Self;

    #[inline]
    fn add(self, other: Self) -> Self {
        Self([
            self.0[0].wrapping_add(other.0[0]),
            self.0[1].wrapping_add(other.0[1]),
            self.0[2].wrapping_add(other.0[2]),
            self.0[3].wrapping_add(other.0[3]),
        ])
    }
}

impl BitXor for i32x4 {
    type Output = Self;

    #[inline]
    fn bitxor(self, other: Self) -> Self {
        Self([
            self.0[0] ^ other.0[0],
            self.0[1] ^ other.0[1],
            self.0[2] ^ other.0[2],
            self.0[3] ^ other.0[3],
        ])
    }
}

#[inline]
fn rational_tanh(value: f32) -> f32 {
    let x = value.clamp(-4.0, 4.0);
    let x2 = x * x;
    let numerator = x * (135_135.0 + x2 * (17_325.0 + x2 * (378.0 + x2)));
    let denominator = 135_135.0 + x2 * (62_370.0 + x2 * (3_150.0 + 28.0 * x2));
    numerator / denominator
}

/// Embedded scalar entry points used by subsystem math backends.
///
/// Keeping these beside the four-lane implementation ensures the scalar and
/// lane paths use the same bounded approximations without exposing them as DSP
/// API surface.
#[inline]
pub(crate) fn scalar_tanh(value: f32) -> f32 {
    rational_tanh(value)
}

#[inline]
pub(crate) fn scalar_tan(value: f32) -> f32 {
    accurate_tan(value)
}

#[inline]
pub(crate) fn scalar_exp2(value: f32) -> f32 {
    accurate_exp2(value)
}

#[inline]
fn embedded_floor(value: f32) -> f32 {
    if value == 0.0 || !value.is_finite() {
        value
    } else {
        ::micromath::F32Ext::floor(value)
    }
}

/// Scalar form of `wide`'s Agner Fog sine/cosine polynomial.
///
/// Micromath's tangent is intentionally not used here: its documented error
/// is too large for filter-frequency prewarping.
#[inline]
fn accurate_tan(value: f32) -> f32 {
    const DP1: f32 = 0.785_156_25 * 2.0;
    const DP2: f32 = 2.418_756_5e-4 * 2.0;
    const DP3: f32 = 3.774_895e-8 * 2.0;
    const SIN_0: f32 = -1.666_665_5e-1;
    const SIN_1: f32 = 8.332_161e-3;
    const SIN_2: f32 = -1.951_529_6e-4;
    const COS_0: f32 = 4.166_664_6e-2;
    const COS_1: f32 = -1.388_731_6e-3;
    const COS_2: f32 = 2.443_315_7e-5;
    const TWO_OVER_PI: f32 = 2.0 / core::f32::consts::PI;

    if !value.is_finite() {
        return f32::NAN;
    }

    let absolute = value.abs();
    let quadrant = ::micromath::F32Ext::round(absolute * TWO_OVER_PI) as i32;
    if quadrant > 0x0200_0000 {
        return 0.0;
    }

    let y = quadrant as f32;
    let x = absolute - y * DP1 - y * DP2 - y * DP3;
    let x2 = x * x;
    let x4 = x2 * x2;
    let sin_poly = x4 * SIN_2 + x2 * SIN_1 + SIN_0;
    let cos_poly = x4 * COS_2 + x2 * COS_1 + COS_0;
    let sin = sin_poly * (x * x2) + x;
    let cos = cos_poly * x4 + (1.0 - 0.5 * x2);

    let (mut sin, cos) = match quadrant & 3 {
        0 => (sin, cos),
        1 => (cos, -sin),
        2 => (-sin, -cos),
        _ => (-cos, sin),
    };
    if value.is_sign_negative() {
        sin = -sin;
    }
    sin / cos
}

/// Range-reduced scalar `2^x` polynomial matching the accuracy of `wide`.
#[inline]
fn accurate_exp2(value: f32) -> f32 {
    if value.is_nan() {
        return f32::NAN;
    }
    if value >= 128.0 {
        return f32::INFINITY;
    }
    if value < -149.0 {
        return 0.0;
    }

    let rounded = ::micromath::F32Ext::round(value).min(127.0);
    let exponent = rounded as i32;
    let fraction = (value - rounded) * core::f32::consts::LN_2;
    let mut polynomial = 1.0 / 5040.0;
    polynomial = 1.0 / 720.0 + fraction * polynomial;
    polynomial = 1.0 / 120.0 + fraction * polynomial;
    polynomial = 1.0 / 24.0 + fraction * polynomial;
    polynomial = 1.0 / 6.0 + fraction * polynomial;
    polynomial = 1.0 / 2.0 + fraction * polynomial;
    polynomial = 1.0 + fraction * polynomial;
    let fraction_exp = 1.0 + fraction * polynomial;
    fraction_exp * power_of_two(exponent)
}

#[inline]
fn power_of_two(exponent: i32) -> f32 {
    if exponent > 127 {
        f32::INFINITY
    } else if exponent >= -126 {
        f32::from_bits(((exponent + 127) as u32) << 23)
    } else if exponent >= -149 {
        f32::from_bits(1_u32 << (exponent + 149))
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_lanes_close(actual: f32x4, expected: wide::f32x4, tolerance: f32) {
        for (actual, expected) in actual.to_array().into_iter().zip(expected.to_array()) {
            assert!(
                (actual - expected).abs() <= tolerance,
                "lane mismatch: actual {actual}, expected {expected}, tolerance {tolerance}"
            );
        }
    }

    #[test]
    fn type_has_wide_compatible_size_and_alignment() {
        assert_eq!(core::mem::size_of::<f32x4>(), 16);
        assert_eq!(core::mem::align_of::<f32x4>(), 16);
    }

    #[test]
    fn masks_compare_blend_and_combine_like_wide() {
        let left = f32x4::new([-2.0, 1.0, 4.0, 8.0]);
        let right = f32x4::new([-1.0, 1.0, 3.0, 9.0]);
        let lt = left.simd_lt(right);
        let gt = left.simd_gt(right);
        assert_eq!(lt.blend(left, right).to_array(), [-2.0, 1.0, 3.0, 8.0]);
        assert!(!(lt & gt).all());
        assert!(left.is_finite().all());
    }

    #[test]
    fn integer_lanes_wrap_and_convert() {
        let left = i32x4::new([i32::MAX, -1, 2, 3]);
        let right = i32x4::new([1, 3, 4, 5]);
        assert_eq!((left + right).0, [i32::MIN, 2, 6, 8]);
        assert_eq!((left ^ right).0, [i32::MAX ^ 1, -4, 6, 6]);
        assert_eq!(right.round_float().to_array(), [1.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn arithmetic_and_reductions_match_wide() {
        let left_lanes = [-3.5, -0.25, 2.0, 11.0];
        let right_lanes = [0.5, 4.0, -8.0, 2.0];
        let left = f32x4::new(left_lanes);
        let right = f32x4::new(right_lanes);
        let wide_left = wide::f32x4::new(left_lanes);
        let wide_right = wide::f32x4::new(right_lanes);

        assert_lanes_close(left + right, wide_left + wide_right, 0.0);
        assert_lanes_close(left - right, wide_left - wide_right, 0.0);
        assert_lanes_close(left * right, wide_left * wide_right, 0.0);
        assert_lanes_close(left / right, wide_left / wide_right, 0.0);
        assert_lanes_close(left * 0.25, wide_left * 0.25, 0.0);
        assert_eq!(left.reduce_add(), wide_left.reduce_add());
    }

    #[test]
    fn lane_helpers_match_wide_for_finite_values() {
        let lanes = [-3.75, -0.0, 1.5, 8.25];
        let value = f32x4::new(lanes);
        let wide_value = wide::f32x4::new(lanes);
        assert_lanes_close(value.abs(), wide_value.abs(), 0.0);
        assert_lanes_close(value.floor(), wide_value.floor(), 0.0);
        assert_lanes_close(
            value.clamp(f32x4::splat(-2.0), f32x4::splat(4.0)),
            wide_value.clamp(wide::f32x4::splat(-2.0), wide::f32x4::splat(4.0)),
            0.0,
        );
    }

    #[test]
    fn special_values_match_wide_semantics() {
        let values = f32x4::new([f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.0]);
        let floored = values.floor().to_array();
        assert!(floored[0].is_nan());
        assert_eq!(floored[1], f32::INFINITY);
        assert_eq!(floored[2], f32::NEG_INFINITY);
        assert_eq!(floored[3].to_bits(), (-0.0f32).to_bits());

        let powers = f32x4::new([f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 127.75])
            .exp2()
            .to_array();
        assert!(powers[0].is_nan());
        assert_eq!(powers[1], f32::INFINITY);
        assert_eq!(powers[2], 0.0);
        assert!(powers[3].is_finite());
    }

    #[test]
    fn micromath_sine_and_cosine_stay_within_documented_error() {
        const SAMPLES: usize = 65_536;
        let mut maximum_error = 0.0f32;
        for start in (0..=SAMPLES).step_by(4) {
            let lanes = core::array::from_fn(|lane| {
                let fraction = (start + lane).min(SAMPLES) as f32 / SAMPLES as f32;
                -crate::TAU + 2.0 * crate::TAU * fraction
            });
            let (sin, cos) = f32x4::new(lanes).sin_cos();
            for lane in 0..4 {
                maximum_error = maximum_error
                    .max((sin.0[lane] - libm::sinf(lanes[lane])).abs())
                    .max((cos.0[lane] - libm::cosf(lanes[lane])).abs());
            }
        }
        assert!(
            maximum_error <= 0.002,
            "sine/cosine error {maximum_error} exceeds Micromath's documented bound"
        );
    }

    #[test]
    fn micromath_exp2_stays_monotonic_and_within_relative_error_bound() {
        const SAMPLES: usize = 65_536;
        let mut maximum_relative_error = 0.0f32;
        let mut previous = 0.0f32;
        for start in (0..=SAMPLES).step_by(4) {
            let lanes = core::array::from_fn(|lane| {
                let fraction = (start + lane).min(SAMPLES) as f32 / SAMPLES as f32;
                -10.0 + 20.0 * fraction
            });
            let actual = f32x4::new(lanes).exp2().to_array();
            for lane in 0..4 {
                let expected = libm::exp2f(lanes[lane]);
                maximum_relative_error =
                    maximum_relative_error.max(((actual[lane] - expected) / expected).abs());
                assert!(
                    actual[lane] >= previous,
                    "exp2 approximation is not monotonic"
                );
                previous = actual[lane];
            }
        }
        assert!(
            maximum_relative_error <= 2.0e-6,
            "exp2 relative error {maximum_relative_error} exceeds pitch-quality tolerance"
        );
    }

    #[test]
    fn rational_tanh_is_bounded_monotonic_and_accurate() {
        const SAMPLES: usize = 65_536;
        let mut maximum_error = 0.0f32;
        let mut previous = -1.0f32;
        for start in (0..=SAMPLES).step_by(4) {
            let lanes = core::array::from_fn(|lane| {
                let fraction = (start + lane).min(SAMPLES) as f32 / SAMPLES as f32;
                -4.0 + 8.0 * fraction
            });
            let actual = f32x4::new(lanes).tanh().to_array();
            for lane in 0..4 {
                maximum_error = maximum_error.max((actual[lane] - libm::tanhf(lanes[lane])).abs());
                assert!(
                    actual[lane] >= previous,
                    "tanh approximation is not monotonic"
                );
                assert!(
                    actual[lane].abs() <= 1.0,
                    "tanh approximation is not bounded"
                );
                previous = actual[lane];
            }
        }
        assert!(
            maximum_error <= 5.0e-5,
            "tanh value error {maximum_error} exceeds tolerance"
        );
    }
}
