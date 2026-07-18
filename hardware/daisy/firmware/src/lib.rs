#![no_std]

#[cfg(test)]
extern crate std;

pub mod audio;
pub mod diagnostics;
pub mod indicator;
pub mod midi;
pub mod patch_transition;
#[cfg(feature = "audio-profiling")]
pub mod profiling;
pub mod synth;
