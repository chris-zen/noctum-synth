//! Generic, reusable signal-processing building blocks.
//!
//! Nothing in this module imports patch parameters, modulation destinations, or
//! voice allocation — those live in [`crate::voice`] / [`crate::patch`].

pub mod analog_oscillator;
pub mod analog_sub_oscillator;
pub mod blep;
pub mod dc_blocker;
pub mod envelope;
pub mod filter;
#[cfg(feature = "oscillator-research")]
mod gray_box_oscillator;
#[cfg(feature = "oscillator-research")]
mod gray_box_profile;
pub mod lfo;
#[cfg(feature = "osc-wavetable")]
pub(crate) mod live_wavetable;
pub(crate) mod lookahead_limiter;
pub mod noise;
#[cfg(feature = "oscillator-research")]
pub mod oscillator_research;
pub mod parameter_smoother;
pub(crate) mod rng;
#[cfg(feature = "oscillator-research")]
mod target_conditioned_oscillator;
#[cfg(feature = "oscillator-research")]
mod target_conditioned_profile;
#[cfg(feature = "oscillator-research")]
mod target_conditioned_profile_v2;
#[cfg(any(test, feature = "downsampling"))]
pub(crate) mod upsampler;
pub mod wavetable;
#[cfg(feature = "osc-wavetable")]
mod wavetable_bank;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WavetableSupportStatus {
    Measured,
    TransitionToFallback,
    AboveCapturedRange,
    UnsupportedPlaybackRate,
    InvalidFrequency,
    FundamentalAboveNyquistGuard,
}

impl WavetableSupportStatus {
    pub const fn uses_measured(self) -> bool {
        matches!(self, Self::Measured | Self::TransitionToFallback)
    }

    pub const fn is_warning(self) -> bool {
        !matches!(self, Self::Measured)
    }
}

pub use analog_oscillator::{AnalogOscillator, Waveform, WavetableOscillator};
pub use blep::SawMethod;
pub use filter::{Filter, FilterOversampling, FilterType};
pub use lfo::{LfoWaveform, MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
pub use noise::WhiteNoise;
#[cfg(feature = "oscillator-research")]
pub use oscillator_research::{
    OscillatorResearchModel, RegisteredResearchModel, ResearchComparisonMetrics,
    ResearchDiagnosticFrame, ResearchError, ResearchEvent, ResearchModelCapabilities,
    ResearchModelDescriptor, ResearchModelFamily, ResearchModelId, ResearchParameterDescriptor,
    ResearchParameterScale, ResearchRegistry, ResearchRenderCase, ResearchRenderSummary,
    ResearchSignalMetrics, render_research_case,
};
pub use parameter_smoother::DEFAULT_PARAMETER_SMOOTHING_SECONDS;
pub use wavetable::{MipWavetableBank, WAVETABLE_BANK_SAMPLES, generate_wavetable_bank};
#[cfg(feature = "osc-wavetable")]
pub use wavetable_bank::{
    WAVETABLE_MAX_HARMONIC, WAVETABLE_MIP_COUNT, WAVETABLE_MIP_HARMONIC_LIMITS,
    WAVETABLE_WAVEFORMS, WavetableBank, WavetableBankError, WavetableBankReport, WavetableProfile,
};
