//! Generic, reusable signal-processing building blocks.
//!
//! Nothing in this module imports patch parameters, modulation destinations, or
//! voice allocation — those live in [`crate::voice`] / [`crate::patch`].

pub mod analog_oscillator;
pub mod analog_sub_oscillator;
pub mod blep;
pub mod dc_blocker;
pub mod envelope;
#[cfg(feature = "experimental-oscillators")]
pub(crate) mod experimental_oscillator;
pub mod filter;
pub mod lfo;
pub(crate) mod lookahead_limiter;
#[cfg(feature = "experimental-oscillators")]
mod measured_wavetable;
#[cfg(feature = "experimental-oscillators")]
#[allow(dead_code)]
mod measured_wavetable_profile;
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

pub use analog_oscillator::{AnalogOscillator, Waveform, WavetableOscillator};
pub use blep::SawMethod;
#[cfg(feature = "experimental-oscillators")]
pub use experimental_oscillator::{
    ExperimentalOscillatorCapabilities, ExperimentalOscillatorModel,
};
pub use filter::{Filter, FilterOversampling, FilterType};
pub use lfo::{LfoWaveform, MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
#[cfg(feature = "experimental-oscillators")]
pub use measured_wavetable::{
    MEASURED_WAVETABLE_PITCHES, MEASURED_WAVETABLE_WAVEFORMS, MeasuredWavetableBank,
    MeasuredWavetableBankError, MeasuredWavetableBankReport,
};
pub use noise::WhiteNoise;
#[cfg(feature = "oscillator-research")]
pub use oscillator_research::{
    OscillatorResearchModel, RegisteredResearchModel, ResearchComparisonMetrics, ResearchError,
    ResearchEvent, ResearchModelDescriptor, ResearchModelFamily, ResearchModelId,
    ResearchParameterDescriptor, ResearchParameterScale, ResearchRegistry, ResearchRenderCase,
    ResearchRenderSummary, ResearchSignalMetrics, render_research_case,
};
pub use parameter_smoother::DEFAULT_PARAMETER_SMOOTHING_SECONDS;
pub use wavetable::{WAVETABLE_BANK_SAMPLES, WavetableBank, generate_wavetable_bank};
