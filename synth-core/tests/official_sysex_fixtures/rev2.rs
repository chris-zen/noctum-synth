//! Official Sequential Rev2 factory-bank regressions.

use synth_core::{
    LayerId, LayerMode, ModDestination,
    midi::{
        prophet::unpack_program_data,
        rev2::{
            PROGRAM_DATA_LEN, PROGRAM_DATA_SYSEX_LEN, PROGRAM_EDIT_BUFFER_SYSEX_LEN,
            PROGRAM_PACKED_LEN, decode, encode,
        },
    },
};

const FACTORY_SYSEX: &[u8] =
    include_bytes!("../../../Prophet-Rev2-Factory-Programs/Rev2_Programs_v1.0.syx");

#[test]
fn official_factory_bank_has_the_verified_layer_mode_distribution() {
    let mut counts = [0_usize; 3];

    for message in FACTORY_SYSEX.chunks_exact(PROGRAM_DATA_SYSEX_LEN) {
        let decoded = decode::program_data(message).unwrap();
        let index = match decoded.patch.mode {
            LayerMode::Normal => 0,
            LayerMode::Stack => 1,
            LayerMode::Split => 2,
        };
        counts[index] += 1;
    }

    assert_eq!(counts, [174, 266, 72]);
}

#[test]
fn official_factory_bank_decodes_two_finite_layers() {
    assert_eq!(FACTORY_SYSEX.len() / PROGRAM_DATA_SYSEX_LEN, 512);
    for (index, message) in FACTORY_SYSEX
        .chunks_exact(PROGRAM_DATA_SYSEX_LEN)
        .enumerate()
    {
        let patch = decode::program_data(message).unwrap().patch;
        for layer in [LayerId::A, LayerId::B] {
            patch.layer(layer).for_each_param(|param, value| {
                assert!(value.is_finite(), "program {index} {layer:?} {param:?}");
            });
            patch.layer(layer).for_each_modulation(|route, slot| {
                assert!(
                    slot.amount.is_finite(),
                    "program {index} {layer:?} {route:?}"
                );
            });
        }
    }
}

#[test]
fn documented_factory_regressions_preserve_layer_b_identity() {
    let cases = [
        (
            1,
            "All That Glitter",
            "All That Glitter B",
            24.0,
            74.0 / 127.0,
        ),
        (5, "BoiteMusique", "BoiteMusique", 62.0, 0.0),
        (18, "Horn Busker", "League Brass", 24.0, 37.0 / 127.0),
        (37, "Sitcom Piano", "Sitcom Pad", 43.0, 90.0 / 127.0),
    ];
    for (program, name_a, name_b, osc1_frequency_b, resonance_b) in cases {
        let patch = decode::program_data(factory_message(program))
            .unwrap()
            .patch;
        assert_eq!(patch.layer_a.name.as_str(), name_a);
        assert_eq!(patch.layer_b.name.as_str(), name_b);
        assert_eq!(patch.layer_b.osc1.frequency, osc1_frequency_b);
        assert!((patch.layer_b.filter.resonance - resonance_b).abs() < 0.001);
    }
}

#[test]
fn factory_decode_encode_decode_preserves_both_layers_and_mode() {
    for program in [1, 5, 18, 37] {
        let source = decode::program_data(factory_message(program))
            .unwrap()
            .patch;
        let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        encode::program_edit_buffer(&source, &mut message).unwrap();
        let decoded = decode::program_edit_buffer(&message).unwrap();

        assert_eq!(decoded.mode, source.mode, "program {program}");
        assert_eq!(decoded.split_point, source.split_point, "program {program}");
        for layer in [LayerId::A, LayerId::B] {
            let expected = source.layer(layer);
            let actual = decoded.layer(layer);
            assert_eq!(actual.name, expected.name, "program {program} {layer:?}");
            assert_eq!(
                actual.osc1.frequency, expected.osc1.frequency,
                "program {program} {layer:?}"
            );
            assert!(
                (actual.filter.cutoff - expected.filter.cutoff).abs() < 0.05,
                "program {program} {layer:?}"
            );
            assert!(
                (actual.filter.resonance - expected.filter.resonance).abs() < 0.001,
                "program {program} {layer:?}"
            );
        }
    }
}

#[test]
fn factory_program_decodes_mod_destination_indices() {
    let message = &FACTORY_SYSEX[..PROGRAM_DATA_SYSEX_LEN];
    let decoded = decode::program_data(message).unwrap();
    assert_eq!(
        decoded.patch.layer_a.lfos[2].destination,
        ModDestination::Osc1ShapeMod
    );

    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    unpack_program_data(&message[6..6 + PROGRAM_PACKED_LEN], &mut raw);
    assert_eq!(raw[67] & 0x7f, 7);
    assert_eq!(raw[93] & 0x7f, 3);
}

fn factory_message(program: usize) -> &'static [u8] {
    let offset = program * PROGRAM_DATA_SYSEX_LEN;
    &FACTORY_SYSEX[offset..offset + PROGRAM_DATA_SYSEX_LEN]
}
