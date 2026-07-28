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
pub(crate) mod lookahead_limiter;
pub mod noise;
pub mod parameter_smoother;
pub(crate) mod rng;
#[cfg(any(test, feature = "downsampling"))]
pub(crate) mod upsampler;
pub mod wavetable;

pub use analog_oscillator::WavetableOscillator;
pub use analog_oscillator::{AnalogOscillator, SawMethod, Waveform};
pub use analog_sub_oscillator::AnalogSubOscillator;
pub use dc_blocker::DcBlocker;
pub use envelope::{
    DEFAULT_ATTACK_SECONDS, DEFAULT_DECAY_SECONDS, DEFAULT_RELEASE_SECONDS, DEFAULT_SUSTAIN_LEVEL,
    DadsrEnvelope,
};
pub use filter::{Filter, FilterOversampling, FilterType, LadderFilter};
pub use lfo::{Lfo, LfoWaveform, MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
pub use noise::WhiteNoise;
pub use parameter_smoother::{
    DEFAULT_PARAMETER_SMOOTHING_SECONDS, ParameterSmoother, WideParameterSmoother,
    smoothing_coefficient, smoothing_coefficient_euler_approx,
};
pub use wavetable::{
    WAVETABLE_BANK_SAMPLES, WavetableBank, WavetableBankError, WavetableBankReport,
    generate_wavetable_bank,
};
