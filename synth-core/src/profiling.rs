//! Platform-neutral hooks for measuring the synthesis render path.
//!
//! The module deliberately contains no clock or logging implementation. A host
//! or firmware supplies those details through [`RenderProfiler`].
//!
//! Sample-path call sites use [`profiler_begin!`] / [`profiler_end!`] so the
//! profiler calls compile out when the `profiling` feature is disabled while
//! stage expressions remain type-checked.

#[cfg(not(feature = "profiling"))]
use core::marker::PhantomData;

/// Individually measurable stages of the per-sample synthesis path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RenderStage {
    /// Firmware control messages applied before rendering a block.
    ControlDrain,
    EnvelopesAndModulation,
    /// Envelope generation within the modulation parent stage.
    EnvelopeAdvance,
    /// Routes which alter LFO rate or amount, using prior LFO outputs.
    LfoControlRouting,
    /// LFO waveform generation and latent phase advancement.
    LfoGeneration,
    /// Routes applied to audio destinations using current LFO outputs.
    AudioModulationRouting,
    /// Routes evaluated at the embedded control rate.
    ControlRateRouting,
    /// Per-sample interpolation of control-rate modulation values.
    ControlRateInterpolation,
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
    /// Effect parameter preparation and selected-kernel dispatch.
    EffectsPreparation,
    /// Parallel comb portion of the reverb network.
    ReverbCombs,
    /// Serial allpass diffusion portion of the reverb network.
    ReverbAllpasses,
    /// Effect wet/dry mix and output limiting.
    EffectsMix,
    MasterOutput,
    /// Firmware copy from interleaved engine output into the DMA block.
    OutputCopy,
}

impl RenderStage {
    pub const COUNT: usize = 21;
    pub const ALL: [Self; Self::COUNT] = [
        Self::ControlDrain,
        Self::EnvelopesAndModulation,
        Self::EnvelopeAdvance,
        Self::LfoControlRouting,
        Self::LfoGeneration,
        Self::AudioModulationRouting,
        Self::ControlRateRouting,
        Self::ControlRateInterpolation,
        Self::Oscillators,
        Self::OscillatorControl,
        Self::OscillatorWaveform,
        Self::OscillatorMix,
        Self::Filter,
        Self::AmplifierAndPan,
        Self::Effects,
        Self::EffectsPreparation,
        Self::ReverbCombs,
        Self::ReverbAllpasses,
        Self::EffectsMix,
        Self::MasterOutput,
        Self::OutputCopy,
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

/// Per-sample render channel for optional profiling.
///
/// When the `profiling` feature is disabled this is a ZST and [`begin`] /
/// [`end`] compile to nothing. Prefer [`profiler_begin!`] / [`profiler_end!`]
/// at call sites so stage expressions are also cfg-stripped.
///
/// Use [`create_render_context!`] for unprofiled paths (engine, tests, tools).
pub struct RenderContext<'a> {
    #[cfg(feature = "profiling")]
    profiler: Option<&'a mut dyn RenderProfiler>,
    #[cfg(not(feature = "profiling"))]
    _phantom: PhantomData<&'a ()>,
}

impl<'a> RenderContext<'a> {
    #[cfg(feature = "profiling")]
    #[inline(always)]
    pub fn new(profiler: &'a mut dyn RenderProfiler) -> Self {
        Self {
            profiler: Some(profiler),
        }
    }

    #[inline(always)]
    pub const fn unprofiled() -> RenderContext<'static> {
        #[cfg(feature = "profiling")]
        {
            RenderContext { profiler: None }
        }
        #[cfg(not(feature = "profiling"))]
        {
            RenderContext {
                _phantom: PhantomData,
            }
        }
    }

    #[inline(always)]
    pub fn begin(&mut self, stage: RenderStage) {
        #[cfg(feature = "profiling")]
        if let Some(profiler) = self.profiler.as_mut() {
            profiler.begin(stage);
        }
        #[cfg(not(feature = "profiling"))]
        let _ = stage;
    }

    #[inline(always)]
    pub fn end(&mut self, stage: RenderStage) {
        #[cfg(feature = "profiling")]
        if let Some(profiler) = self.profiler.as_mut() {
            profiler.end(stage);
        }
        #[cfg(not(feature = "profiling"))]
        let _ = stage;
    }
}

/// Creates an unprofiled [`RenderContext`].
///
/// Expands to a value suitable for `let mut ctx = create_render_context!();`.
#[macro_export]
macro_rules! create_render_context {
    () => {
        $crate::profiling::RenderContext::unprofiled()
    };
}

/// Begins a render stage. The stage expression is always type-checked; the
/// profiler call is compiled out when `profiling` is disabled.
#[macro_export]
macro_rules! profiler_begin {
    ($ctx:expr, $stage:expr) => {{
        #[cfg(feature = "profiling")]
        $ctx.begin($stage);
        #[cfg(not(feature = "profiling"))]
        {
            let _ = &mut *$ctx;
            let _ = $stage;
        }
    }};
}

/// Ends a render stage. The stage expression is always type-checked; the
/// profiler call is compiled out when `profiling` is disabled.
#[macro_export]
macro_rules! profiler_end {
    ($ctx:expr, $stage:expr) => {{
        #[cfg(feature = "profiling")]
        $ctx.end($stage);
        #[cfg(not(feature = "profiling"))]
        {
            let _ = &mut *$ctx;
            let _ = $stage;
        }
    }};
}
