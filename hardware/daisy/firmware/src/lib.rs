#![no_std]

#[cfg(test)]
extern crate std;

pub mod audio;
pub mod diagnostics;
pub mod indicator;
pub mod midi;
#[cfg(feature = "audio-profiling")]
pub mod profiling;
pub mod synth;
