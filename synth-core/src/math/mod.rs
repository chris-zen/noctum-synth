mod scalar;

#[cfg(feature = "fast-math")]
mod micro;
#[cfg(not(feature = "fast-math"))]
mod simd;

pub use scalar::F32;

#[cfg(feature = "fast-math")]
pub use micro::WideF32;
#[cfg(not(feature = "fast-math"))]
pub use simd::WideF32;

#[cfg(test)]
pub(crate) mod testing;

#[cfg(feature = "fast-math")]
#[allow(unused_imports)]
pub(crate) use micro::WideI32;
#[cfg(not(feature = "fast-math"))]
#[allow(unused_imports)]
pub(crate) use simd::WideI32;

/// Circle constant π.
pub const PI: f32 = core::f32::consts::PI;
/// Full circle in radians (2π).
pub const TAU: f32 = 2.0 * PI;
