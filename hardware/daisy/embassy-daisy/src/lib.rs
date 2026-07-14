#![no_std]
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

pub mod format;
mod wm8731;

#[cfg(target_arch = "arm")]
pub mod audio;
#[cfg(target_arch = "arm")]
pub mod board;
#[cfg(target_arch = "arm")]
pub mod clocks;
#[cfg(target_arch = "arm")]
mod memory;
#[cfg(target_arch = "arm")]
pub mod pins;
#[cfg(target_arch = "arm")]
pub mod qspi;
#[cfg(target_arch = "arm")]
pub mod sdram;
#[cfg(target_arch = "arm")]
pub mod usb;

#[cfg(target_arch = "arm")]
pub use board::{Board, BoardParts, TakeError};
