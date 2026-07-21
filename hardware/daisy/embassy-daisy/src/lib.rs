#![no_std]
#![cfg_attr(test, no_main)]
#![doc = include_str!("../README.md")]

#[cfg(not(any(
    feature = "seed",
    feature = "seed_1_1",
    feature = "seed_1_2",
    feature = "patch_sm"
)))]
compile_error!("select exactly one Daisy board feature; currently supported: `seed_1_1`");

#[cfg(any(
    all(feature = "seed", feature = "seed_1_1"),
    all(feature = "seed", feature = "seed_1_2"),
    all(feature = "seed", feature = "patch_sm"),
    all(feature = "seed_1_1", feature = "seed_1_2"),
    all(feature = "seed_1_1", feature = "patch_sm"),
    all(feature = "seed_1_2", feature = "patch_sm"),
))]
compile_error!("Daisy board features are mutually exclusive; select exactly one");

#[cfg(any(
    all(
        feature = "seed",
        not(any(feature = "seed_1_1", feature = "seed_1_2", feature = "patch_sm"))
    ),
    all(
        feature = "seed_1_2",
        not(any(feature = "seed", feature = "seed_1_1", feature = "patch_sm"))
    ),
    all(
        feature = "patch_sm",
        not(any(feature = "seed", feature = "seed_1_1", feature = "seed_1_2"))
    ),
))]
compile_error!("this Daisy board is reserved but not implemented; currently supported: `seed_1_1`");

#[cfg(feature = "sampling_rate_96khz")]
compile_error!("96 kHz audio is reserved but has not been validated yet");

#[cfg(feature = "block_length_64")]
compile_error!("64-frame audio blocks are reserved but have not been validated yet");

#[cfg(not(target_arch = "arm"))]
compile_error!("embassy-daisy targets Cortex-M7 only; build with thumbv7em-none-eabihf");

mod wm8731;

pub mod audio;
pub mod board;
pub mod clocks;
pub mod led;
mod memory;
pub mod pins;
pub mod pwm;
pub mod qspi;
pub mod sdram;
pub mod usb;
pub mod usb_audio;

pub use board::{Board, BoardParts, TakeError};
pub use led::{PwmUserLed, UserLed, UserLedPin};
pub use pwm::{PwmChannel, PwmChannels, PwmFrequency, PwmOutput};

#[cfg(test)]
#[embedded_test::setup]
fn setup() {
    use defmt_rtt as _;
}

#[cfg(test)]
#[embedded_test::tests]
mod link_hack {
    use defmt_rtt as _;
    use embassy_stm32 as _;
    use panic_probe as _;

    #[test]
    fn embassy_stm32_linked() {}
}
