//! Official Sequential Rev2 factory-bank regressions.

use synth_core::{
    ControlMessage, LayerId, LayerMode, LayerTarget, ModDestination, PolyVelocity,
    SynthEngineWithMemory, VOICE_PACKS,
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
fn documented_factory_layer_programs_render_without_parameter_workarounds() {
    for program in [1, 5, 18, 37] {
        let patch = decode::program_data(factory_message(program))
            .unwrap()
            .patch;
        let effects_memory = vec![0.0; 48_000 * 4].into_boxed_slice();
        let mut engine = SynthEngineWithMemory::<_, VOICE_PACKS, 2>::new_with_effects_memory(
            48_000.0,
            effects_memory,
        )
        .unwrap();
        engine.apply_patch(&patch);
        engine.note_on(60, 1.0);

        let mut output = vec![0.0; 48_000 * 2];
        engine.process(&mut output);
        let mean_square = output
            .iter()
            .skip(8_192)
            .map(|sample| sample * sample)
            .sum::<f32>()
            / (output.len() - 8_192) as f32;
        let rms = mean_square.sqrt();
        assert!(
            rms > 0.000_01,
            "factory program {program} should render without changing cutoff or envelope amount; RMS {rms}"
        );
    }
}

#[test]
fn f1_001_poly_sequence_plays_when_an_unused_lane_resets() {
    let patch = decode::program_data(factory_message(0)).unwrap().patch;
    assert_eq!(patch.layer_a.name.as_str(), "LosVangelis2041");
    let first_step = patch.layer_a.sequence.poly.steps[0];
    assert_eq!(first_step.lanes[0].velocity, PolyVelocity::Reset);
    assert!(matches!(
        first_step.lanes[1].velocity,
        PolyVelocity::Velocity(_)
    ));
    let first_full_reset = patch
        .layer_a
        .sequence
        .poly
        .steps
        .iter()
        .position(|step| step.is_reset());
    assert_eq!(first_full_reset, Some(23));

    let effects_memory = vec![0.0; 48_000 * 4].into_boxed_slice();
    let mut engine = SynthEngineWithMemory::<_, VOICE_PACKS, 2>::new_with_effects_memory(
        48_000.0,
        effects_memory,
    )
    .unwrap();
    engine.apply_patch(&patch);
    engine.handle_control(ControlMessage::SetSequencerRunning {
        target: LayerTarget::Explicit(LayerId::A),
        running: true,
    });

    let mut output = [0.0; 2];
    engine.process(&mut output);
    assert_eq!(engine.layer_active_voice_count(LayerId::A), 1);
}

#[test]
fn f1_002_stack_sequence_plays_from_layer_b() {
    let patch = decode::program_data(factory_message(1)).unwrap().patch;
    assert_eq!(patch.mode, LayerMode::Stack);
    assert_eq!(patch.layer_a.name.as_str(), "All That Glitter");
    assert_eq!(patch.layer_b.name.as_str(), "All That Glitter B");

    let effects_memory = vec![0.0; 48_000 * 4].into_boxed_slice();
    let mut engine = SynthEngineWithMemory::<_, VOICE_PACKS, 2>::new_with_effects_memory(
        48_000.0,
        effects_memory,
    )
    .unwrap();
    engine.apply_patch(&patch);
    for layer in [LayerId::A, LayerId::B] {
        engine.handle_control(ControlMessage::SetSequencerRunning {
            target: LayerTarget::Explicit(layer),
            running: true,
        });
    }

    let mut output = [0.0; 2];
    engine.process(&mut output);
    assert_eq!(engine.layer_active_voice_count(LayerId::A), 0);
    assert_eq!(engine.layer_active_voice_count(LayerId::B), 1);
}

#[test]
fn f1_004_gated_sequence_advances_while_a_key_is_held() {
    let patch = decode::program_data(factory_message(3)).unwrap().patch;
    assert_eq!(patch.layer_a.name.as_str(), "Balalaika2017");

    let effects_memory = vec![0.0; 48_000 * 4].into_boxed_slice();
    let mut engine = SynthEngineWithMemory::<_, VOICE_PACKS, 2>::new_with_effects_memory(
        48_000.0,
        effects_memory,
    )
    .unwrap();
    engine.apply_patch(&patch);
    engine.note_on(60, 1.0);

    let mut output = vec![0.0; 48_000 * 2];
    engine.process(&mut output);
    assert!(
        engine.sequencer_active_step(LayerId::A).is_some()
            || engine.sequencer_active_step(LayerId::B).is_some(),
        "F1-004 gated sequence should advance while a key is held"
    );
    let rms =
        (output.iter().map(|sample| sample * sample).sum::<f32>() / output.len() as f32).sqrt();
    assert!(rms > 0.000_01, "F1-004 should produce audio; rms={rms}");
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
