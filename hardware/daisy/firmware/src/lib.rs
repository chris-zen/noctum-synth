#![no_std]

#[cfg(test)]
extern crate std;

pub mod midi;
#[cfg(all(feature = "audio-profiling", target_arch = "arm"))]
pub mod profiling;
pub mod synth;
