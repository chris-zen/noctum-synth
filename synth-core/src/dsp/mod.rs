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
#[cfg(feature = "osc-wavetable")]
#[allow(dead_code)]
mod wavetable_bank_profile;
#[cfg(feature = "osc-wavetable")]
#[allow(dead_code)]
mod wavetable_bank_profile_prophet5;

pub use analog_oscillator::{AnalogOscillator, Waveform, WavetableOscillator};
pub use blep::SawMethod;
pub use filter::{Filter, FilterOversampling, FilterType};
pub use lfo::{LfoWaveform, MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
pub use noise::WhiteNoise;
#[cfg(feature = "oscillator-research")]
pub use oscillator_research::{
    OscillatorResearchModel, RegisteredResearchModel, ResearchComparisonMetrics, ResearchError,
    ResearchEvent, ResearchModelCapabilities, ResearchModelDescriptor, ResearchModelFamily,
    ResearchModelId, ResearchParameterDescriptor, ResearchParameterScale, ResearchRegistry,
    ResearchRenderCase, ResearchRenderSummary, ResearchSignalMetrics, render_research_case,
};
pub use parameter_smoother::DEFAULT_PARAMETER_SMOOTHING_SECONDS;
pub use wavetable::{MipWavetableBank, WAVETABLE_BANK_SAMPLES, generate_wavetable_bank};
#[cfg(feature = "osc-wavetable")]
pub use wavetable_bank::{
    WAVETABLE_WAVEFORMS, WavetableBank, WavetableBankError, WavetableBankReport, WavetableProfile,
};
#[cfg(feature = "osc-wavetable")]
pub use wavetable_bank_profile::MONOLOGUE_WAVETABLE_BANK_PROFILE;
#[cfg(feature = "osc-wavetable")]
pub use wavetable_bank_profile_prophet5::PROPHET5_WAVETABLE_BANK_PROFILE;
