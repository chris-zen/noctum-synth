//! Feature-gated generated wavetable assets compiled into the desktop binary.

use crate::dsp::{
    MONOLOGUE_WAVETABLE_BANK_PROFILE, PROPHET5_WAVETABLE_BANK_PROFILE, WavetableBank,
};

use super::BankId;

const MONOLOGUE_SAMPLE_COUNT: usize = 221_184;
const PROPHET5_SAMPLE_COUNT: usize = 227_328;

static MONOLOGUE_SAMPLES: [f32; MONOLOGUE_SAMPLE_COUNT] = decode_f32le::<
    MONOLOGUE_SAMPLE_COUNT,
    { MONOLOGUE_SAMPLE_COUNT * 4 },
>(include_bytes!("monologue.f32le"));

static PROPHET5_SAMPLES: [f32; PROPHET5_SAMPLE_COUNT] = decode_f32le::<
    PROPHET5_SAMPLE_COUNT,
    { PROPHET5_SAMPLE_COUNT * 4 },
>(include_bytes!("prophet5.f32le"));

pub(super) const fn bank(id: BankId) -> WavetableBank {
    match id {
        BankId::Monologue => {
            WavetableBank::from_compiled(&MONOLOGUE_SAMPLES, &MONOLOGUE_WAVETABLE_BANK_PROFILE)
        }
        BankId::Prophet5 => {
            WavetableBank::from_compiled(&PROPHET5_SAMPLES, &PROPHET5_WAVETABLE_BANK_PROFILE)
        }
    }
}

const fn decode_f32le<const SAMPLES: usize, const BYTES: usize>(
    bytes: &[u8; BYTES],
) -> [f32; SAMPLES] {
    assert!(BYTES == SAMPLES * 4);
    let mut output = [0.0; SAMPLES];
    let mut index = 0;
    while index < SAMPLES {
        let offset = index * 4;
        output[index] = f32::from_bits(u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]));
        index += 1;
    }
    output
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
                    BankId::Monologue => &MONOLOGUE_SAMPLES,
                    BankId::Prophet5 => &PROPHET5_SAMPLES,
                },
                compiled.profile(),
            )
            .expect("compiled samples must match their generated profile");
            assert_eq!(validated.report(), compiled.report());
        }
    }
}
