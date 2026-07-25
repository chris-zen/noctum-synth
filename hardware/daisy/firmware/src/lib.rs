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
pub mod patch_transition;
#[cfg(target_arch = "arm")]
pub mod pending_releases;
#[cfg(all(target_arch = "arm", feature = "audio-profiling"))]
pub mod profiling;
pub mod program;
#[cfg(target_arch = "arm")]
pub mod synth;
#[cfg(target_arch = "arm")]
pub mod usb_audio;
mod usb_audio_core;

#[cfg(target_arch = "arm")]
pub fn fatal(reason: &'static str) -> ! {
    defmt::error!("fatal firmware initialization failure: {=str}", reason);
    loop {
        cortex_m::asm::wfi();
    }
}
