use core::ops::{
    Add, AddAssign, BitAnd, BitXor, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign,
};

use super::scalar::F32;

#[cfg(feature = "wide-8")]
const WIDE_LANES: usize = 8;
#[cfg(feature = "wide-4")]
const WIDE_LANES: usize = 4;
#[cfg(feature = "wide-1")]
const WIDE_LANES: usize = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "wide-8", repr(C, align(32)))]
#[cfg_attr(feature = "wide-4", repr(C, align(16)))]
#[cfg_attr(feature = "wide-1", repr(C, align(4)))]
pub struct WideF32([f32; WIDE_LANES]);

impl WideF32 {
    pub const LANES: usize = WIDE_LANES;
    pub const ZERO: Self = Self([0.0; WIDE_LANES]);

    #[inline]
    pub fn new(lanes: [f32; WIDE_LANES]) -> Self {
        Self(lanes)
    }

    #[inline]
    pub const fn splat(value: f32) -> Self {
        Self([value; WIDE_LANES])
    }

    #[inline]
    pub fn to_array(self) -> [f32; WIDE_LANES] {
        self.0
    }

    #[inline]
    pub fn abs(self) -> Self {
        Self(self.0.map(|v| v.abs()))
    }

    #[inline]
    pub fn floor(self) -> Self {
        Self(self.0.map(|v| F32(v).floor().0))
    }

    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self(core::array::from_fn(|i| self.0[i].min(other.0[i])))
    }

    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self(core::array::from_fn(|i| self.0[i].max(other.0[i])))
    }

    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        self.max(min).min(max)
    }

    #[inline]
    pub fn is_finite(self) -> Self {
        Self::mask(core::array::from_fn(|i| self.0[i].is_finite()))
    }

    #[inline]
    pub fn simd_lt(self, other: Self) -> Self {
        Self::mask(core::array::from_fn(|i| self.0[i] < other.0[i]))
    }

    #[inline]
    pub fn simd_gt(self, other: Self) -> Self {
        Self::mask(core::array::from_fn(|i| self.0[i] > other.0[i]))
    }

    #[inline]
    pub fn simd_ge(self, other: Self) -> Self {
        Self::mask(core::array::from_fn(|i| self.0[i] >= other.0[i]))
    }

    #[inline]
    pub fn blend(self, if_true: Self, if_false: Self) -> Self {
        Self(core::array::from_fn(|i| {
            if mask_lane(self.0[i]) {
                if_true.0[i]
            } else {
                if_false.0[i]
            }
        }))
    }

    #[inline]
    pub fn all(self) -> bool {
        self.0.iter().all(|&v| mask_lane(v))
    }

    #[inline]
    pub fn any(self) -> bool {
        self.0.iter().any(|&v| mask_lane(v))
    }

    #[inline]
    pub fn reduce_add(self) -> f32 {
        self.0.iter().sum()
    }

    #[inline]
    pub fn reduce_mean(self) -> f32 {
        self.reduce_add() / Self::LANES as f32
    }

    #[inline]
    pub fn exp2(self) -> Self {
        Self(self.0.map(|v| F32(v).exp2().0))
    }

    #[inline]
    pub fn sin_cos(self) -> (Self, Self) {
        let mut sin = [0.0; WIDE_LANES];
        let mut cos = [0.0; WIDE_LANES];
        for i in 0..WIDE_LANES {
            let (s, c) = F32(self.0[i]).sin_cos();
            sin[i] = s.0;
            cos[i] = c.0;
        }
        (Self(sin), Self(cos))
    }

    #[inline]
    pub fn tan(self) -> Self {
        Self(self.0.map(|v| F32(v).tan().0))
    }

    #[inline]
    pub fn tanh(self) -> Self {
        Self(self.0.map(|v| F32(v).tanh().0))
    }

    #[inline]
    fn mask(lanes: [bool; WIDE_LANES]) -> Self {
        Self(lanes.map(|b| f32::from_bits(if b { u32::MAX } else { 0 })))
    }

    #[inline]
    pub fn replace_lane(mut self, lane: usize, value: f32) -> Self {
        debug_assert!(lane < Self::LANES);
        self.0[lane] = value;
        self
    }

    pub fn wrap01(self) -> Self {
        self - self.floor()
    }
}

#[inline]
fn mask_lane(value: f32) -> bool {
    value.to_bits() & 0x8000_0000 != 0
}

impl From<[f32; WIDE_LANES]> for WideF32 {
    #[inline]
    fn from(value: [f32; WIDE_LANES]) -> Self {
        Self::new(value)
    }
}

impl From<WideF32> for [f32; WIDE_LANES] {
    #[inline]
    fn from(value: WideF32) -> Self {
        value.to_array()
    }
}

macro_rules! impl_binop {
    ($trait:ident, $method:ident, $op:tt) => {
        impl $trait<WideF32> for WideF32 {
            type Output = WideF32;
            #[inline]
            fn $method(self, rhs: WideF32) -> WideF32 {
                WideF32(core::array::from_fn(|i| self.0[i] $op rhs.0[i]))
            }
        }
        impl $trait<f32> for WideF32 {
            type Output = WideF32;
            #[inline]
            fn $method(self, rhs: f32) -> WideF32 {
                WideF32(core::array::from_fn(|i| self.0[i] $op rhs))
            }
        }
        impl $trait<WideF32> for f32 {
            type Output = WideF32;
            #[inline]
            fn $method(self, rhs: WideF32) -> WideF32 {
                WideF32(core::array::from_fn(|i| self $op rhs.0[i]))
            }
        }
    };
}

macro_rules! impl_assign {
    ($trait:ident, $method:ident, $op:tt) => {
        impl $trait<WideF32> for WideF32 {
            #[inline]
            fn $method(&mut self, rhs: WideF32) {
                for i in 0..WIDE_LANES {
                    self.0[i] $op rhs.0[i];
                }
            }
        }
        impl $trait<f32> for WideF32 {
            #[inline]
            fn $method(&mut self, rhs: f32) {
                for i in 0..WIDE_LANES {
                    self.0[i] $op rhs;
                }
            }
        }
    };
}

impl_binop!(Add, add, +);
impl_binop!(Sub, sub, -);
impl_binop!(Mul, mul, *);
impl_binop!(Div, div, /);

impl_assign!(AddAssign, add_assign, +=);
impl_assign!(SubAssign, sub_assign, -=);
impl_assign!(MulAssign, mul_assign, *=);
impl_assign!(DivAssign, div_assign, /=);

impl Neg for WideF32 {
    type Output = WideF32;
    #[inline]
    fn neg(self) -> WideF32 {
        WideF32(self.0.map(|v| -v))
    }
}

impl BitAnd for WideF32 {
    type Output = WideF32;
    #[inline]
    fn bitand(self, rhs: WideF32) -> WideF32 {
        WideF32(core::array::from_fn(|i| {
            f32::from_bits(self.0[i].to_bits() & rhs.0[i].to_bits())
        }))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "wide-8", repr(C, align(32)))]
#[cfg_attr(feature = "wide-4", repr(C, align(16)))]
#[cfg_attr(feature = "wide-1", repr(C, align(4)))]
pub(crate) struct WideI32([i32; WIDE_LANES]);

impl WideI32 {
    #[inline]
    pub(crate) fn new(lanes: [i32; WIDE_LANES]) -> Self {
        Self(lanes)
    }

    #[cfg(test)]
    #[inline]
    pub const fn splat(value: i32) -> Self {
        Self([value; WIDE_LANES])
    }

    #[inline]
    pub(crate) fn round_float(self) -> WideF32 {
        WideF32::new(self.0.map(|v| v as f32))
    }
}

impl Add for WideI32 {
    type Output = WideI32;
    #[inline]
    fn add(self, rhs: WideI32) -> WideI32 {
        WideI32(core::array::from_fn(|i| self.0[i].wrapping_add(rhs.0[i])))
    }
}

impl BitXor for WideI32 {
    type Output = WideI32;
    #[inline]
    fn bitxor(self, rhs: WideI32) -> WideI32 {
        WideI32(core::array::from_fn(|i| self.0[i] ^ rhs.0[i]))
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
    use super::{WideF32, WideI32};
    use crate::math::testing::{from_fn, lane0, splat};

    #[test]
    fn type_has_size_and_alignment() {
        assert_eq!(
            core::mem::align_of::<WideF32>(),
            WideF32::LANES * core::mem::size_of::<f32>()
        );
        assert_eq!(
            core::mem::align_of::<WideI32>(),
            WideF32::LANES * core::mem::size_of::<i32>()
        );
    }

    #[test]
    fn splat_and_default_fill_every_lane() {
        let v = WideF32::splat(3.5);
        assert_eq!(v.to_array(), [3.5; WideF32::LANES]);
        let zero: WideF32 = Default::default();
        assert_eq!(zero.to_array(), [0.0; WideF32::LANES]);
        assert_eq!(WideF32::ZERO.to_array(), [0.0; WideF32::LANES]);
    }

    #[test]
    fn new_and_to_array_roundtrip_preserves_every_lane() {
        let arr = core::array::from_fn(|i| (i as f32 + 1.0) * 0.5);
        assert_eq!(WideF32::new(arr).to_array(), arr);
    }

    #[test]
    fn reduce_add_sums_distinct_lanes() {
        let v = from_fn(|i| (i + 1) as f32);
        let count = WideF32::LANES as f32;
        let expected = count * (count + 1.0) / 2.0;
        assert!((v.reduce_add() - expected).abs() < 1e-4);
        assert!((v.reduce_mean() - expected / count).abs() < 1e-4);
    }

    #[test]
    fn mixed_mask_any_is_not_all() {
        let left = from_fn(|i| if i == 0 { 1.0 } else { 5.0 });
        let right = splat(3.0);
        let lt = left.simd_lt(right);
        assert!(lt.any());
        assert_eq!(lt.all(), WideF32::LANES == 1);
        if WideF32::LANES > 1 {
            let gt = left.simd_gt(right);
            assert!(gt.any());
            assert!(!gt.all());
            assert!(!(lt & gt).any());
        }
    }

    #[test]
    fn blend_selects_per_lane_under_mixed_mask() {
        let if_true = from_fn(|i| (i + 1) as f32);
        let if_false = splat(-1.0);
        let mask = from_fn(|i| f32::from_bits(if i % 2 == 0 { u32::MAX } else { 0 }));
        let blended = mask.blend(if_true, if_false).to_array();
        for (i, sample) in blended.iter().enumerate() {
            let expected = if i % 2 == 0 { (i + 1) as f32 } else { -1.0 };
            assert_eq!(*sample, expected);
        }
    }

    #[test]
    fn is_finite_any_all_with_partial_nan() {
        assert!(splat(1.0).is_finite().all());
        assert!(splat(1.0).is_finite().any());

        let with_nan = splat(1.0).replace_lane(0, f32::NAN);
        assert!(!with_nan.is_finite().all());
        assert_eq!(with_nan.is_finite().any(), WideF32::LANES > 1);

        if WideF32::LANES > 1 {
            let only_last = splat(1.0).replace_lane(WideF32::LANES - 1, f32::NAN);
            assert!(!only_last.is_finite().all());
            assert!(only_last.is_finite().any());
        }
    }

    #[test]
    fn replace_lane_leaves_other_lanes_unchanged() {
        let base = from_fn(|i| (i + 1) as f32);
        for lane in 0..WideF32::LANES {
            let replaced = base.replace_lane(lane, 99.0).to_array();
            for (i, sample) in replaced.iter().enumerate() {
                let expected = if i == lane { 99.0 } else { (i + 1) as f32 };
                assert_eq!(*sample, expected, "lane {lane} rewrite leaked into {i}");
            }
        }
    }

    #[test]
    fn masks_compare_blend_lane0_scenarios() {
        for (left, right, expected) in [
            (-2.0, -1.0, -2.0),
            (1.0, 1.0, 1.0),
            (4.0, 3.0, 3.0),
            (8.0, 9.0, 8.0),
        ] {
            let left = splat(left);
            let right = splat(right);
            let lt = left.simd_lt(right);
            assert_eq!(lane0(lt.blend(left, right)), expected);
        }
    }

    #[test]
    fn arithmetic_lane0_scenarios() {
        for (left, right, expected_sum, expected_scaled) in [
            (-3.5, 0.5, -3.0, -1.75),
            (-0.25, 4.0, 3.75, -0.125),
            (2.0, -8.0, -6.0, 1.0),
            (11.0, 2.0, 13.0, 5.5),
        ] {
            let left = splat(left);
            let right = splat(right);
            assert_eq!(lane0(left + right), expected_sum);
            assert_eq!(lane0(left * 0.5), expected_scaled);
        }
    }

    #[test]
    fn scalar_arithmetic_is_commutative() {
        let v = splat(3.0);
        assert_eq!((v + 2.0).to_array(), (2.0 + v).to_array());
    }

    #[test]
    fn assign_ops_modify_every_lane() {
        let mut v = from_fn(|i| i as f32 + 1.0);
        let before = v.to_array();
        v += 1.0;
        for (i, x) in v.to_array().iter().enumerate() {
            assert!((*x - (before[i] + 1.0)).abs() < 0.01);
        }
    }

    #[test]
    fn wrap01_wraps_into_0_1() {
        for value in [-1.0, 0.5, 1.7, 3.0] {
            let wrapped = lane0(splat(value).wrap01());
            assert!(
                (0.0..1.0).contains(&wrapped),
                "{value} wrapped to {wrapped}, not in [0, 1)"
            );
        }
    }

    #[test]
    fn min_max_clamp() {
        for (value, expected) in [(5.0, 5.0), (-1.0, 0.0), (-3.0, 0.0), (8.0, 6.0)] {
            let clamped = lane0(splat(value).clamp(WideF32::ZERO, splat(6.0)));
            assert_eq!(clamped, expected);
        }
    }

    #[test]
    fn floor_and_abs() {
        for (value, expected_floor, expected_abs) in [
            (-3.7, -4.0, 3.7),
            (-0.2, -1.0, 0.2),
            (2.9, 2.0, 2.9),
            (4.0, 4.0, 4.0),
        ] {
            let v = splat(value);
            assert_eq!(lane0(v.floor()), expected_floor);
            assert_eq!(lane0(v.abs()), expected_abs);
        }
    }

    #[test]
    fn exp2_is_lane_wise_across_all_lanes() {
        let v = from_fn(|i| i as f32);
        let exp = v.exp2().to_array();
        for (i, x) in exp.iter().enumerate() {
            assert!((*x - 2.0f32.powi(i as i32)).abs() < 1e-3, "exp2({i}) = {x}");
        }
    }

    #[test]
    fn tanh_is_odd_bounded_and_monotonic() {
        for &(value, expect_negative) in &[(-2.0, true), (-1.0, true), (0.5, false), (2.0, false)] {
            let th = lane0(splat(value).tanh());
            if expect_negative {
                assert!(th.abs() < 1.0 && th < 0.0);
            } else {
                assert!(th > 0.0 && th < 1.0);
            }
        }
        let neg = lane0(splat(-2.0).tanh());
        let pos = lane0(splat(2.0).tanh());
        assert!((neg + pos).abs() < 0.01, "tanh not odd: {}", neg + pos);
    }

    #[test]
    fn sin_cos_is_lane_wise_and_orthonormal() {
        let v = from_fn(|i| core::f32::consts::PI * i as f32 / 3.0);
        let (s, c) = v.sin_cos();
        for i in 0..WideF32::LANES {
            let mag = s.to_array()[i].powi(2) + c.to_array()[i].powi(2);
            assert!((mag - 1.0).abs() < 0.002, "sin^2+cos^2={mag}");
        }
    }

    #[test]
    fn special_values_are_finite() {
        let zero = WideF32::ZERO;
        assert_eq!(zero.floor().to_array(), [0.0; WideF32::LANES]);
        assert!((zero.exp2().reduce_add() - WideF32::LANES as f32).abs() < 0.01);
    }

    #[test]
    fn widei32_round_float_preserves_every_lane() {
        let values = core::array::from_fn(|i| (i as i32 + 1) * 2);
        let converted = WideI32::new(values).round_float().to_array();
        for (i, sample) in converted.iter().enumerate() {
            assert_eq!(*sample, values[i] as f32);
        }
    }

    #[test]
    fn widei32_wrapping_add_and_xor() {
        let sum = (WideI32::splat(i32::MAX) + WideI32::splat(1)).round_float();
        assert_eq!(sum.to_array(), [i32::MIN as f32; WideF32::LANES]);

        let xor = (WideI32::splat(0xF0i32) ^ WideI32::splat(0xFFi32)).round_float();
        assert_eq!(xor.to_array(), [0x0F_i32 as f32; WideF32::LANES]);
    }
}
