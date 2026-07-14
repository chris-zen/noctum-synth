//! Platform-neutral hooks for measuring the synthesis render path.
//!
//! The module deliberately contains no clock or logging implementation. A host
//! or firmware supplies those details through [`RenderProfiler`].

/// Individually measurable stages of the per-sample synthesis path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RenderStage {
    EnvelopesAndModulation,
    Oscillators,
    Filter,
    AmplifierAndPan,
    Effects,
    MasterOutput,
}

impl RenderStage {
    pub const COUNT: usize = 6;
    pub const ALL: [Self; Self::COUNT] = [
        Self::EnvelopesAndModulation,
        Self::Oscillators,
        Self::Filter,
        Self::AmplifierAndPan,
        Self::Effects,
        Self::MasterOutput,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Receives stage boundaries while a profiled render is in progress.
pub trait RenderProfiler {
    fn begin(&mut self, stage: RenderStage);
    fn end(&mut self, stage: RenderStage);
}

pub(crate) struct NoopProfiler;

impl RenderProfiler for NoopProfiler {
    #[inline(always)]
    fn begin(&mut self, _stage: RenderStage) {}

    #[inline(always)]
    fn end(&mut self, _stage: RenderStage) {}
}
