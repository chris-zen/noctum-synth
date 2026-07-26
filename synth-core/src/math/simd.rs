use core::ops::{
    Add, AddAssign, BitAnd, BitXor, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign,
};

#[cfg(feature = "wide-1")]
use super::scalar::F32;

#[cfg(feature = "wide-8")]
pub const WIDE_LANES: usize = 8;
#[cfg(feature = "wide-4")]
pub const WIDE_LANES: usize = 4;
#[cfg(feature = "wide-1")]
pub const WIDE_LANES: usize = 1;

#[cfg(any(feature = "wide-8", feature = "wide-4"))]
mod wide_backed {
    #[cfg(feature = "wide-8")]
    pub type Backend = wide::f32x8;
    #[cfg(feature = "wide-4")]
    pub type Backend = wide::f32x4;
}

#[cfg(feature = "wide-1")]
mod scalar_backed {
    pub type Backend = f32;
}

#[cfg(feature = "wide-1")]
use scalar_backed::Backend;
#[cfg(any(feature = "wide-8", feature = "wide-4"))]
use wide_backed::Backend;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WideF32(Backend);

#[cfg(any(feature = "wide-8", feature = "wide-4"))]
impl WideF32 {
    pub const LANES: usize = WIDE_LANES;
    pub const ZERO: Self = Self(Backend::ZERO);

    #[inline]
    pub fn new(lanes: [f32; WIDE_LANES]) -> Self {
        Self(Backend::new(lanes))
    }

    #[inline]
    pub const fn splat(value: f32) -> Self {
        Self(Backend::splat(value))
    }

    #[inline]
    pub fn to_array(self) -> [f32; WIDE_LANES] {
        self.0.to_array()
    }

    #[inline]
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }

    #[inline]
    pub fn floor(self) -> Self {
        Self(self.0.floor())
    }

    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }

    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        self.max(min).min(max)
    }

    #[inline]
    pub fn is_finite(self) -> Self {
        Self(self.0.is_finite())
    }

    #[inline]
    pub fn simd_lt(self, other: Self) -> Self {
        Self(self.0.simd_lt(other.0))
    }

    #[inline]
    pub fn simd_gt(self, other: Self) -> Self {
        Self(self.0.simd_gt(other.0))
    }

    #[inline]
    pub fn simd_ge(self, other: Self) -> Self {
        Self(self.0.simd_ge(other.0))
    }

    #[inline]
    pub fn blend(self, if_true: Self, if_false: Self) -> Self {
        Self(self.0.blend(if_true.0, if_false.0))
    }

    #[inline]
    pub fn all(self) -> bool {
        self.0.all()
    }

    #[inline]
    pub fn any(self) -> bool {
        self.0.any()
    }

    #[inline]
    pub fn reduce_add(self) -> f32 {
        self.0.reduce_add()
    }

    #[inline]
    pub fn reduce_mean(self) -> f32 {
        self.reduce_add() / Self::LANES as f32
    }

    #[inline]
    pub fn exp2(self) -> Self {
        Self(self.0.exp2())
    }

    #[inline]
    pub fn sin_cos(self) -> (Self, Self) {
        let (s, c) = self.0.sin_cos();
        (Self(s), Self(c))
    }

    #[inline]
    pub fn tan(self) -> Self {
        Self(self.0.tan())
    }

    #[inline]
    pub fn tanh(self) -> Self {
        Self(self.0.tanh())
    }

    #[inline]
    pub fn replace_lane(self, lane: usize, value: f32) -> Self {
        debug_assert!(lane < Self::LANES);
        let mut values = self.0.to_array();
        values[lane] = value;
        Self(Backend::new(values))
    }

    pub fn wrap01(self) -> Self {
        self - self.floor()
    }
}

#[cfg(feature = "wide-1")]
impl WideF32 {
    pub const LANES: usize = WIDE_LANES;
    pub const ZERO: Self = Self(0.0);

    #[inline]
    pub fn new(lanes: [f32; 1]) -> Self {
        Self(lanes[0])
    }

    #[inline]
    pub const fn splat(value: f32) -> Self {
        Self(value)
    }

    #[inline]
    pub fn to_array(self) -> [f32; 1] {
        [self.0]
    }

    #[inline]
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }

    #[inline]
    pub fn floor(self) -> Self {
        F32(self.0).floor().into()
    }

    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }

    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self(self.0.clamp(min.0, max.0))
    }

    #[inline]
    pub fn is_finite(self) -> Self {
        Self(f32::from_bits(if self.0.is_finite() {
            u32::MAX
        } else {
            0
        }))
    }

    #[inline]
    pub fn simd_lt(self, other: Self) -> Self {
        Self(f32::from_bits(if self.0 < other.0 { u32::MAX } else { 0 }))
    }

    #[inline]
    pub fn simd_gt(self, other: Self) -> Self {
        Self(f32::from_bits(if self.0 > other.0 { u32::MAX } else { 0 }))
    }

    #[inline]
    pub fn simd_ge(self, other: Self) -> Self {
        Self(f32::from_bits(if self.0 >= other.0 { u32::MAX } else { 0 }))
    }

    #[inline]
    pub fn blend(self, if_true: Self, if_false: Self) -> Self {
        if self.mask_lane() { if_true } else { if_false }
    }

    #[inline]
    pub fn all(self) -> bool {
        self.mask_lane()
    }

    #[inline]
    pub fn any(self) -> bool {
        self.mask_lane()
    }

    #[inline]
    pub fn reduce_add(self) -> f32 {
        self.0
    }

    #[inline]
    pub fn reduce_mean(self) -> f32 {
        self.0
    }

    #[inline]
    pub fn exp2(self) -> Self {
        F32(self.0).exp2().into()
    }

    #[inline]
    pub fn sin_cos(self) -> (Self, Self) {
        let (s, c) = F32(self.0).sin_cos();
        (Self(s.0), Self(c.0))
    }

    #[inline]
    pub fn tan(self) -> Self {
        F32(self.0).tan().into()
    }

    #[inline]
    pub fn tanh(self) -> Self {
        F32(self.0).tanh().into()
    }

    #[inline]
    pub fn replace_lane(self, lane: usize, value: f32) -> Self {
        debug_assert!(lane < Self::LANES);
        Self(value)
    }

    #[inline]
    fn mask_lane(self) -> bool {
        self.0.to_bits() & 0x8000_0000 != 0
    }

    pub fn wrap01(self) -> Self {
        self - self.floor()
    }
}

impl Default for WideF32 {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

macro_rules! impl_widef32_binop {
    ($trait:ident, $method:ident, $op:tt) => {
        impl $trait<WideF32> for WideF32 {
            type Output = WideF32;
            #[inline]
            fn $method(self, rhs: WideF32) -> WideF32 {
                WideF32(self.0 $op rhs.0)
            }
        }
        impl $trait<f32> for WideF32 {
            type Output = WideF32;
            #[inline]
            fn $method(self, rhs: f32) -> WideF32 {
                #[cfg(any(feature = "wide-8", feature = "wide-4"))]
                { WideF32(self.0 $op Backend::splat(rhs)) }
                #[cfg(feature = "wide-1")]
                { WideF32(self.0 $op rhs) }
            }
        }
        impl $trait<WideF32> for f32 {
            type Output = WideF32;
            #[inline]
            fn $method(self, rhs: WideF32) -> WideF32 {
                #[cfg(any(feature = "wide-8", feature = "wide-4"))]
                { WideF32(Backend::splat(self) $op rhs.0) }
                #[cfg(feature = "wide-1")]
                { WideF32(self $op rhs.0) }
            }
        }
    };
}

macro_rules! impl_widef32_assign {
    ($trait:ident, $method:ident, $op:tt) => {
        impl $trait<WideF32> for WideF32 {
            #[inline]
            fn $method(&mut self, rhs: WideF32) {
                self.0 = self.0 $op rhs.0;
            }
        }
        impl $trait<f32> for WideF32 {
            #[inline]
            fn $method(&mut self, rhs: f32) {
                #[cfg(any(feature = "wide-8", feature = "wide-4"))]
                { self.0 = self.0 $op Backend::splat(rhs); }
                #[cfg(feature = "wide-1")]
                { self.0 = self.0 $op rhs; }
            }
        }
    };
}

impl_widef32_binop!(Add, add, +);
impl_widef32_binop!(Sub, sub, -);
impl_widef32_binop!(Mul, mul, *);
impl_widef32_binop!(Div, div, /);

impl_widef32_assign!(AddAssign, add_assign, +);
impl_widef32_assign!(SubAssign, sub_assign, -);
impl_widef32_assign!(MulAssign, mul_assign, *);
impl_widef32_assign!(DivAssign, div_assign, /);

impl Neg for WideF32 {
    type Output = WideF32;
    #[inline]
    fn neg(self) -> WideF32 {
        WideF32(-self.0)
    }
}

#[cfg(any(feature = "wide-8", feature = "wide-4"))]
impl BitAnd for WideF32 {
    type Output = WideF32;
    #[inline]
    fn bitand(self, rhs: WideF32) -> WideF32 {
        WideF32(self.0 & rhs.0)
    }
}

#[cfg(feature = "wide-1")]
impl BitAnd for WideF32 {
    type Output = WideF32;
    #[inline]
    fn bitand(self, rhs: WideF32) -> WideF32 {
        WideF32(f32::from_bits(self.0.to_bits() & rhs.0.to_bits()))
    }
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

#[cfg(feature = "wide-1")]
impl From<F32> for WideF32 {
    #[inline]
    fn from(value: F32) -> Self {
        Self(value.0)
    }
}

#[cfg(feature = "wide-1")]
impl From<WideF32> for F32 {
    #[inline]
    fn from(value: WideF32) -> Self {
        F32(value.0)
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(crate) struct WideI32 {
    #[cfg(feature = "wide-8")]
    inner: wide::i32x8,
    #[cfg(feature = "wide-4")]
    inner: wide::i32x4,
    #[cfg(feature = "wide-1")]
    inner: i32,
}

impl WideI32 {
    #[cfg(feature = "wide-8")]
    #[inline]
    pub(crate) fn new(lanes: [i32; 8]) -> Self {
        Self {
            inner: wide::i32x8::new(lanes),
        }
    }

    #[cfg(feature = "wide-4")]
    #[inline]
    pub(crate) fn new(lanes: [i32; 4]) -> Self {
        Self {
            inner: wide::i32x4::new(lanes),
        }
    }

    #[cfg(feature = "wide-1")]
    #[inline]
    pub(crate) fn new(lanes: [i32; 1]) -> Self {
        Self { inner: lanes[0] }
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn splat(value: i32) -> Self {
        Self::new([value; WIDE_LANES])
    }

    #[cfg(feature = "wide-8")]
    #[inline]
    pub(crate) fn round_float(self) -> WideF32 {
        WideF32::new(self.inner.to_array().map(|v| v as f32))
    }

    #[cfg(feature = "wide-4")]
    #[inline]
    pub(crate) fn round_float(self) -> WideF32 {
        WideF32::new(self.inner.to_array().map(|v| v as f32))
    }

    #[cfg(feature = "wide-1")]
    #[inline]
    pub(crate) fn round_float(self) -> WideF32 {
        WideF32::new([self.inner as f32])
    }
}

#[cfg(feature = "wide-8")]
impl Add for WideI32 {
    type Output = WideI32;
    #[inline]
    fn add(self, rhs: WideI32) -> WideI32 {
        WideI32 {
            inner: self.inner + rhs.inner,
        }
    }
}

#[cfg(feature = "wide-4")]
impl Add for WideI32 {
    type Output = WideI32;
    #[inline]
    fn add(self, rhs: WideI32) -> WideI32 {
        WideI32 {
            inner: self.inner + rhs.inner,
        }
    }
}

#[cfg(feature = "wide-1")]
impl Add for WideI32 {
    type Output = WideI32;
    #[inline]
    fn add(self, rhs: WideI32) -> WideI32 {
        WideI32 {
            inner: self.inner.wrapping_add(rhs.inner),
        }
    }
}

#[cfg(feature = "wide-8")]
impl BitXor for WideI32 {
    type Output = WideI32;
    #[inline]
    fn bitxor(self, rhs: WideI32) -> WideI32 {
        WideI32 {
            inner: self.inner ^ rhs.inner,
        }
    }
}

#[cfg(feature = "wide-4")]
impl BitXor for WideI32 {
    type Output = WideI32;
    #[inline]
    fn bitxor(self, rhs: WideI32) -> WideI32 {
        WideI32 {
            inner: self.inner ^ rhs.inner,
        }
    }
}

#[cfg(feature = "wide-1")]
impl BitXor for WideI32 {
    type Output = WideI32;
    #[inline]
    fn bitxor(self, rhs: WideI32) -> WideI32 {
        WideI32 {
            inner: self.inner ^ rhs.inner,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::testing::{from_fn, lane0, splat};

    #[test]
    fn splat_and_default_fill_every_lane() {
        let v = WideF32::splat(3.5);
        assert_eq!(v.to_array(), [3.5; WIDE_LANES]);
        let zero: WideF32 = Default::default();
        assert_eq!(zero.to_array(), [0.0; WIDE_LANES]);
        assert_eq!(WideF32::ZERO.to_array(), [0.0; WIDE_LANES]);
    }

    #[test]
    fn new_and_to_array_roundtrip_preserves_every_lane() {
        let arr = core::array::from_fn(|i| (i as f32 + 1.0) * 0.5);
        assert_eq!(WideF32::new(arr).to_array(), arr);
    }

    #[test]
    fn reduce_add_sums_distinct_lanes() {
        let v = from_fn(|i| (i + 1) as f32);
        let count = WIDE_LANES as f32;
        let expected = count * (count + 1.0) / 2.0;
        assert!((v.reduce_add() - expected).abs() < 0.001);
        assert!((v.reduce_mean() - expected / count).abs() < 0.001);
    }

    #[test]
    fn mixed_mask_any_is_not_all() {
        let left = from_fn(|i| if i == 0 { 1.0 } else { 5.0 });
        let right = splat(3.0);
        let lt = left.simd_lt(right);
        assert!(lt.any());
        assert_eq!(lt.all(), WIDE_LANES == 1);
        if WIDE_LANES > 1 {
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
        assert_eq!(with_nan.is_finite().any(), WIDE_LANES > 1);

        if WIDE_LANES > 1 {
            let only_last = splat(1.0).replace_lane(WIDE_LANES - 1, f32::NAN);
            assert!(!only_last.is_finite().all());
            assert!(only_last.is_finite().any());
        }
    }

    #[test]
    fn replace_lane_leaves_other_lanes_unchanged() {
        let base = from_fn(|i| (i + 1) as f32);
        for lane in 0..WIDE_LANES {
            let replaced = base.replace_lane(lane, 99.0).to_array();
            for (i, sample) in replaced.iter().enumerate() {
                let expected = if i == lane { 99.0 } else { (i + 1) as f32 };
                assert_eq!(*sample, expected, "lane {lane} rewrite leaked into {i}");
            }
        }
    }

    #[test]
    fn arithmetic_lane0_scenarios() {
        for (a, b) in [(1.0, 4.0), (2.0, 3.0), (3.0, 2.0), (4.0, 1.0)] {
            assert_eq!(lane0(splat(a) + splat(b)), 5.0);
        }
    }

    #[test]
    fn scalar_arithmetic_applies_to_every_lane() {
        let v = from_fn(|i| (i + 1) as f32);
        let expected = from_fn(|i| (i + 2) as f32);
        assert_eq!((v + 1.0).to_array(), expected.to_array());
        assert_eq!((1.0 + v).to_array(), expected.to_array());
    }

    #[test]
    fn simd_comparisons_and_blend_lane0_scenarios() {
        for (a, b, expected) in [(1.0, 2.0, 1.0), (5.0, 3.0, 3.0), (2.0, 3.0, 2.0), (8.0, 3.0, 3.0)]
        {
            let left = splat(a);
            let right = splat(b);
            let lt = left.simd_lt(right);
            assert_eq!(lt.all(), a < b);
            assert_eq!(lane0(lt.blend(left, right)), expected);
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
    fn math_methods_are_lane_wise_across_all_lanes() {
        let v = from_fn(|i| i as f32);
        let exp = v.exp2().to_array();
        for (i, x) in exp.iter().enumerate() {
            assert!((*x - 2.0f32.powi(i as i32)).abs() < 1e-3, "exp2({i}) = {x}");
        }
    }

    #[test]
    fn widei32_round_float_preserves_every_lane() {
        let values = core::array::from_fn(|idx| (idx as i32 + 1) * 2);
        let converted = WideI32::new(values).round_float().to_array();
        for (idx, sample) in converted.iter().enumerate() {
            assert_eq!(*sample, values[idx] as f32);
        }
    }

    #[test]
    fn widei32_wrapping_add() {
        let sum = (WideI32::splat(i32::MAX) + WideI32::splat(1)).round_float();
        assert_eq!(sum.to_array(), [i32::MIN as f32; WIDE_LANES]);
    }

    #[test]
    fn widei32_xor() {
        let xor = (WideI32::splat(0xF0i32) ^ WideI32::splat(0xFFi32)).round_float();
        assert_eq!(xor.to_array(), [0x0F_i32 as f32; WIDE_LANES]);
    }
}
