#![no_std]

#[cfg(all(test, not(target_arch = "arm")))]
extern crate std;

#[cfg(target_arch = "arm")]
pub mod audio;
#[cfg(target_arch = "arm")]
pub mod diagnostics;
#[cfg(target_arch = "arm")]
pub mod indicator;
#[cfg(target_arch = "arm")]
pub mod midi;
#[cfg(target_arch = "arm")]
pub mod pending_releases;
#[cfg(target_arch = "arm")]
pub mod patch_transition;
#[cfg(all(target_arch = "arm", feature = "audio-profiling"))]
pub mod profiling;
#[cfg(target_arch = "arm")]
pub mod synth;
#[cfg(target_arch = "arm")]
pub mod usb_audio;
mod usb_audio_core;
