#[inline]
pub(crate) fn exp(x: f32) -> f32 {
    libm::expf(x)
}

#[inline]
pub(crate) fn ln(x: f32) -> f32 {
    libm::logf(x)
}

#[inline]
pub(crate) fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

#[inline]
pub(crate) fn round(x: f32) -> f32 {
    libm::roundf(x)
}

#[inline]
pub(crate) fn tan(x: f32) -> f32 {
    libm::tanf(x)
}
