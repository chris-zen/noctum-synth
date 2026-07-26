use super::WideF32;

#[inline]
pub fn splat(value: f32) -> WideF32 {
    WideF32::splat(value)
}

#[inline]
pub fn from_fn(f: impl FnMut(usize) -> f32) -> WideF32 {
    WideF32::new(core::array::from_fn(f))
}

#[inline]
pub fn lane0(values: WideF32) -> f32 {
    values.to_array()[0]
}

#[inline]
pub fn mask_lane(lane: usize) -> WideF32 {
    from_fn(|i| f32::from_bits(if i == lane { u32::MAX } else { 0 }))
}

#[inline]
pub fn mask_lane_active(mask: WideF32, lane: usize) -> bool {
    mask.to_array()[lane].to_bits() & 0x8000_0000 != 0
}
