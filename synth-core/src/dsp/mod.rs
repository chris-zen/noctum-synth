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

pub use analog_oscillator::{AnalogOscillator, Waveform, WavetableOscillator};
pub use blep::SawMethod;
pub use filter::{Filter, FilterOversampling, FilterType};
pub use lfo::{LfoWaveform, MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
pub use parameter_smoother::DEFAULT_PARAMETER_SMOOTHING_SECONDS;
pub use wavetable::{WAVETABLE_BANK_SAMPLES, WavetableBank, generate_wavetable_bank};
