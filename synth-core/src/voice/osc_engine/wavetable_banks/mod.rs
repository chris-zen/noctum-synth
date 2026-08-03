//! Feature-gated generated wavetable assets compiled into the desktop binary.

mod monologue_profile;
mod prophet5_profile;

use crate::dsp::WavetableBank;

use super::BankId;

pub use monologue_profile::MONOLOGUE_WAVETABLE_BANK_PROFILE;
pub use prophet5_profile::PROPHET5_WAVETABLE_BANK_PROFILE;

const MONOLOGUE_SAMPLE_COUNT: usize = MONOLOGUE_WAVETABLE_BANK_PROFILE.sample_count;
const PROPHET5_SAMPLE_COUNT: usize = PROPHET5_WAVETABLE_BANK_PROFILE.sample_count;

static MONOLOGUE_BYTES: AlignedBytes<{ MONOLOGUE_SAMPLE_COUNT * 4 }> =
    AlignedBytes(*include_bytes!("monologue.f32le"));

static PROPHET5_BYTES: AlignedBytes<{ PROPHET5_SAMPLE_COUNT * 4 }> =
    AlignedBytes(*include_bytes!("prophet5.f32le"));

pub(super) fn bank(id: BankId) -> WavetableBank {
    match id {
        BankId::Monologue => WavetableBank::from_compiled(
            samples(&MONOLOGUE_BYTES),
            &MONOLOGUE_WAVETABLE_BANK_PROFILE,
        ),
        BankId::Prophet5 => {
            WavetableBank::from_compiled(samples(&PROPHET5_BYTES), &PROPHET5_WAVETABLE_BANK_PROFILE)
        }
    }
}

#[repr(C, align(4))]
struct AlignedBytes<const BYTES: usize>([u8; BYTES]);

fn samples<const BYTES: usize>(bytes: &'static AlignedBytes<BYTES>) -> &'static [f32] {
    assert!(cfg!(target_endian = "little"));
    assert!(BYTES % core::mem::size_of::<f32>() == 0);
    // AlignedBytes guarantees f32 alignment; every f32 bit pattern is valid.
    unsafe {
        core::slice::from_raw_parts(
            bytes.0.as_ptr().cast::<f32>(),
            BYTES / core::mem::size_of::<f32>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_assets_match_their_generated_profiles() {
        for id in BankId::ALL.iter().map(|(_, id)| *id) {
            let compiled = bank(id);
            let validated = WavetableBank::new(
                match id {
                    BankId::Monologue => samples(&MONOLOGUE_BYTES),
                    BankId::Prophet5 => samples(&PROPHET5_BYTES),
                },
                compiled.profile(),
            )
            .expect("compiled samples must match their generated profile");
            assert_eq!(validated.report(), compiled.report());
        }
    }
}
