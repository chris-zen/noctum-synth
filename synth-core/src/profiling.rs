//! Platform-neutral hooks for measuring the synthesis render path.
//!
//! The module deliberately contains no clock or logging implementation. A host
//! or firmware supplies those details through [`RenderProfiler`].

/// Individually measurable stages of the per-sample synthesis path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RenderStage {
    EnvelopesAndModulation,
    /// Envelope generation within the modulation parent stage.
    EnvelopeAdvance,
    /// Routes which alter LFO rate or amount, using prior LFO outputs.
    LfoControlRouting,
    /// LFO waveform generation and latent phase advancement.
    LfoGeneration,
    /// Routes applied to audio destinations using current LFO outputs.
    AudioModulationRouting,
    /// Complete oscillator section, including the nested stages below.
    Oscillators,
    /// Frequency and shape modulation updates.
    OscillatorControl,
    /// Waveform-specific generation and post-processing across enabled oscillators.
    OscillatorWaveform,
    /// Sub/noise generation, level calculation, and final oscillator mix.
    OscillatorMix,
    Filter,
    AmplifierAndPan,
    Effects,
    MasterOutput,
}

impl RenderStage {
    pub const COUNT: usize = 13;
    pub const ALL: [Self; Self::COUNT] = [
        Self::EnvelopesAndModulation,
        Self::EnvelopeAdvance,
        Self::LfoControlRouting,
        Self::LfoGeneration,
        Self::AudioModulationRouting,
        Self::Oscillators,
        Self::OscillatorControl,
        Self::OscillatorWaveform,
        Self::OscillatorMix,
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
