//! Rev2 MIDI codec tests.

use heapless::Vec;

use crate::{
    GatedDestination, GatedSequencerMode, GatedStep, LayerId, LayerMode, LayerPatch, LayerTarget,
    LfoSyncDivision, ModDestination, ModSource, ParamId, Patch, PolyLaneStep, PolyNote,
    PolyVelocity, SequenceUpdate, SequencerType,
    midi::{
        clock::MidiClockMode,
        prophet::{
            attack_decay_raw, attack_decay_seconds, cutoff_raw_to_hz, pack_program_data,
            release_raw, release_seconds, unpack_program_data,
        },
        rev2::{
            ControllerDecoder, ControllerEncoder, MidiUpdate, SysexError, decode, encode,
            layer::{Layer, LayerA, LayerB},
            map::{
                MappedUpdate, emit_nrpn, emit_osc_shape, map_cc, map_nrpn, program_nrpn_value,
                store_program_nrpn,
            },
            program::decode::program_payload as decode_program_payload,
            program::{
                PROGRAM_DATA_LEN, PROGRAM_DATA_SYSEX_LEN, PROGRAM_EDIT_BUFFER_SYSEX_LEN,
                PROGRAM_PACKED_LEN, layer_mode_from_raw, layer_mode_raw,
            },
        },
    },
};

#[test]
fn rev2_envelope_mapping_matches_measured_anchors() {
    let attack_decay_cases = [
        (0, 0.003),
        (31, 0.135),
        (63, 0.605),
        (95, 1.830),
        (127, 24.660),
    ];
    for (raw, expected_seconds) in attack_decay_cases {
        let seconds = attack_decay_seconds(raw);
        assert!(
            (seconds - expected_seconds).abs() < 1.0e-6,
            "raw {raw}: got {seconds}, expected {expected_seconds}"
        );
    }
    assert!((release_seconds(127) - 40.0).abs() < 1.0e-6);
}

#[test]
fn rev2_envelope_mapping_round_trips_every_raw_value() {
    for raw in 0..=127 {
        assert_eq!(
            attack_decay_raw(attack_decay_seconds(raw)),
            raw,
            "attack/decay raw {raw}"
        );
        assert_eq!(release_raw(release_seconds(raw)), raw, "release raw {raw}");
    }
}

#[test]
fn los_vangelis_attack_value_decodes_to_measured_time() {
    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    raw[LayerA::NAME_RANGE].copy_from_slice(b"LosVangelis2041     ");
    store_program_nrpn(&mut raw, 33, 31, 0);
    let mut packed = [0_u8; PROGRAM_PACKED_LEN];
    pack_program_data(&raw, &mut packed);

    let patch = decode_program_payload(&packed).unwrap().layer_a;

    assert_eq!(patch.name.as_str(), "LosVangelis2041");
    assert!((patch.amplifier.eg_attack - 0.135).abs() < 1.0e-6);
}

#[test]
fn nrpn_round_trips_bipolar_filter_envelope() {
    let mut encoder = ControllerEncoder::default();
    let mut decoder = ControllerDecoder::default();
    let mut decoded = None;
    assert!(encoder.param(0, ParamId::FilterEnvAmount, 1.0, |message| {
        decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
    }));
    assert_eq!(
        decoded,
        Some(param_update(
            LayerTarget::Explicit(LayerId::A),
            ParamId::FilterEnvAmount,
            1.0,
        ))
    );
}

#[test]
fn bpm_nrpn_uses_direct_rev2_values() {
    for bpm in [30.0, 120.0, 250.0] {
        let mut encoder = ControllerEncoder::default();
        let mut decoder = ControllerDecoder::default();
        let mut decoded = None;
        assert!(encoder.param(0, ParamId::Bpm, bpm, |message| {
            decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
        }));
        assert_eq!(
            decoded,
            Some(param_update(
                LayerTarget::Explicit(LayerId::A),
                ParamId::Bpm,
                bpm,
            ))
        );
    }
}

#[test]
fn filter_cutoff_nrpn_uses_semitone_ticks() {
    let mut decoder = ControllerDecoder::default();
    let cases = [
        (0_u16, cutoff_raw_to_hz(0)),
        (96, cutoff_raw_to_hz(96)),
        (105, 440.0),
        (164, cutoff_raw_to_hz(164)),
    ];
    for (raw, expected_hz) in cases {
        let mut decoded = None;
        emit_nrpn(0, 15, raw, &mut |message| {
            decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
        });
        let Some(MidiUpdate::Param {
            param: ParamId::FilterCutoff,
            value: hz,
            ..
        }) = decoded
        else {
            panic!("expected filter cutoff update for raw {raw}");
        };
        assert!(
            (hz - expected_hz).abs() < 0.05,
            "raw {raw}: got {hz}, expected {expected_hz}"
        );
    }
}

#[test]
fn filter_cutoff_cc_matches_nrpn_index_not_full_open() {
    let mut decoder = ControllerDecoder::default();
    let mut cc_hz = None;
    decoder.control_change(0, 102, 127, |update| {
        if let MidiUpdate::Param {
            param: ParamId::FilterCutoff,
            value: hz,
            ..
        } = update
        {
            cc_hz = Some(hz);
        }
    });
    let mut nrpn_hz = None;
    emit_nrpn(0, 15, 127, &mut |message| {
        decoder.control_change(0, message[1], message[2], |update| {
            if let MidiUpdate::Param {
                param: ParamId::FilterCutoff,
                value: hz,
                ..
            } = update
            {
                nrpn_hz = Some(hz);
            }
        });
    });
    let cc_hz = cc_hz.expect("cc cutoff");
    let nrpn_hz = nrpn_hz.expect("nrpn cutoff");
    assert!((cc_hz - nrpn_hz).abs() < 0.05);
    assert!((cc_hz - cutoff_raw_to_hz(127)).abs() < 0.05);
    assert!(cc_hz < cutoff_raw_to_hz(164) * 0.2);
}

#[test]
fn filter_key_track_64_decodes_to_unity() {
    let mut decoder = ControllerDecoder::default();
    let mut decoded = None;
    emit_nrpn(0, 17, 64, &mut |message| {
        decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
    });
    assert_eq!(
        decoded,
        Some(param_update(
            LayerTarget::Explicit(LayerId::A),
            ParamId::FilterKeyTrack,
            1.0,
        ))
    );
}

#[test]
fn global_midi_clock_mode_round_trips_as_nrpn_4099() {
    for mode in MidiClockMode::ALL {
        let mut encoder = ControllerEncoder::default();
        let mut decoder = ControllerDecoder::default();
        let mut decoded = None;
        encoder.midi_clock_mode(0, mode, |message| {
            decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
        });
        assert_eq!(decoded, Some(MidiUpdate::MidiClockMode(mode)));
    }
}

#[test]
fn master_volume_cc7_is_global_and_nrpn_29_is_program_volume() {
    let mut decoder = ControllerDecoder::default();
    let mut updates = Vec::<MidiUpdate, 4>::new();
    assert!(decoder.control_change(0, 7, 64, |update| {
        let _ = updates.push(update);
    }));
    assert_eq!(
        updates.as_slice(),
        &[MidiUpdate::MasterVolume(64.0 / 127.0)]
    );

    updates.clear();
    let mut encoder = ControllerEncoder::default();
    encoder.master_volume(0, 64.0 / 127.0, |message| {
        assert_eq!(message, [0xb0, 7, 64]);
        decoder.control_change(0, message[1], message[2], |update| {
            let _ = updates.push(update);
        });
    });
    assert_eq!(
        updates.as_slice(),
        &[MidiUpdate::MasterVolume(64.0 / 127.0)]
    );

    updates.clear();
    assert!(encoder.param_for_layer(
        0,
        LayerId::A,
        ParamId::ProgramVolume,
        64.0 / 127.0,
        |message| {
            decoder.control_change(0, message[1], message[2], |update| {
                let _ = updates.push(update);
            });
        }
    ));
    assert_eq!(
        updates.as_slice(),
        &[param_update(
            LayerTarget::Explicit(LayerId::A),
            ParamId::ProgramVolume,
            64.0 / 127.0,
        )]
    );
}

#[test]
fn synced_lfo_nrpn_decodes_for_either_rate_and_sync_order() {
    for sync_first in [false, true] {
        let mut decoder = ControllerDecoder::default();
        let mut decoded_division = None;
        let mut send = |number, value| {
            emit_nrpn(0, number, value, &mut |message| {
                decoder.control_change(0, message[1], message[2], |update| {
                    if let MidiUpdate::Param {
                        param: ParamId::Lfo1SyncDivision,
                        value,
                        ..
                    } = update
                    {
                        decoded_division = Some(value as usize);
                    }
                });
            });
        };
        if sync_first {
            send(41, 1);
            send(37, 72);
        } else {
            send(37, 72);
            send(41, 1);
        }
        assert_eq!(
            decoded_division,
            Some(LfoSyncDivision::StepTwoThirds.index())
        );
    }
}

#[test]
fn synced_lfo_program_round_trips_active_division() {
    for division in LfoSyncDivision::ALL {
        let mut source = LayerPatch::default();
        source.lfos[1].rate_hz = 7.25;
        source.lfos[1].clock_sync = true;
        source.lfos[1].sync_division = division;
        let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        encode::program_edit_buffer(&program_with_layer_a(source), &mut message).unwrap();
        let decoded = decode::program_edit_buffer(&message).unwrap().layer_a;
        assert!(decoded.lfos[1].clock_sync);
        assert_eq!(decoded.lfos[1].sync_division, division);
    }
}

#[test]
fn unison_nrpn_uses_rev2_ranges_and_key_mode_order() {
    let mut encoder = ControllerEncoder::default();
    let mut decoder = ControllerDecoder::default();
    let mut decoded = None;
    assert!(encoder.param(0, ParamId::UnisonDetune, 16.0, |message| {
        decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
    }));
    assert_eq!(
        decoded,
        Some(param_update(
            LayerTarget::Explicit(LayerId::A),
            ParamId::UnisonDetune,
            16.0,
        ))
    );

    let mut decoded = None;
    assert!(encoder.param(
        0,
        ParamId::KeyMode,
        crate::KeyMode::High.index() as f32,
        |message| {
            decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
        }
    ));
    assert_eq!(
        decoded,
        Some(param_update(
            LayerTarget::Explicit(LayerId::A),
            ParamId::KeyMode,
            crate::KeyMode::High.index() as f32,
        ))
    );
}

#[test]
fn program_data_round_trips_documented_unison_fields() {
    let mut patch = LayerPatch::default();
    patch.unison_enabled = true;
    patch.unison_mode = crate::UnisonMode::Chord;
    patch.unison_detune = 12.0;
    patch.key_mode = crate::KeyMode::LastRetrigger;
    let message = program_data_message(0, 0, &patch);
    let decoded = decode::program_data(&message).unwrap().patch.layer_a;
    assert!(decoded.unison_enabled);
    assert_eq!(decoded.unison_mode, crate::UnisonMode::Chord);
    assert_eq!(decoded.unison_detune, 12.0);
    assert_eq!(decoded.key_mode, crate::KeyMode::LastRetrigger);
    assert!(decoded.unison_chord.is_empty());
}

#[test]
fn program_data_encoder_round_trips_address_and_patch() {
    let mut source = LayerPatch::default();
    source.name.push_str("Stored Program").unwrap();
    source.filter.resonance = 0.75;
    let mut message = [0_u8; PROGRAM_DATA_SYSEX_LEN];

    let len =
        encode::program_data(7, 127, &program_with_layer_a(source.clone()), &mut message).unwrap();
    let decoded = decode::program_data(&message).unwrap();

    assert_eq!(len, PROGRAM_DATA_SYSEX_LEN);
    assert_eq!((decoded.bank, decoded.program), (7, 127));
    assert_eq!(decoded.patch.layer_a.name, source.name);
    assert!((decoded.patch.layer_a.filter.resonance - source.filter.resonance).abs() < 0.01);
}

#[test]
fn program_edit_buffer_round_trips_mode_and_split_point() {
    let mut source = Patch::default();
    source.mode = LayerMode::Stack;
    source.set_split_point(72);
    let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];

    encode::program_edit_buffer(&source, &mut message).unwrap();
    let decoded = decode::program_edit_buffer(&message).unwrap();

    assert_eq!(decoded.mode, LayerMode::Stack);
    assert_eq!(decoded.split_point, 72);
}

#[test]
fn program_data_encoder_validates_address_and_capacity() {
    let program = Patch::default();
    let mut message = [0_u8; PROGRAM_DATA_SYSEX_LEN];
    assert_eq!(
        encode::program_data(8, 0, &program, &mut message),
        Err(SysexError::InvalidBank)
    );
    assert_eq!(
        encode::program_data(0, 128, &program, &mut message),
        Err(SysexError::NonSevenBitData)
    );
    assert_eq!(
        encode::program_data(0, 0, &program, &mut message[..10]),
        Err(SysexError::OutputTooSmall)
    );
}

#[test]
fn pan_mod_mode_round_trips_as_cc10() {
    let mut encoder = ControllerEncoder::default();
    let mut decoder = ControllerDecoder::default();
    let mut message = [0_u8; 3];
    assert!(encoder.param(3, ParamId::PanModMode, 1.0, |encoded| { message = encoded }));
    assert_eq!(message, [0xb3, 10, 127]);

    let mut decoded = None;
    assert!(decoder.control_change(3, message[1], message[2], |update| {
        decoded = Some(update)
    }));
    assert_eq!(
        decoded,
        Some(param_update(LayerTarget::Edit, ParamId::PanModMode, 1.0,))
    );
}

#[test]
fn oscillator_shape_combines_enabled_and_waveform() {
    let mut encoder = ControllerEncoder::default();
    let mut messages = [[0; 3]; 8];
    let mut len = 0;
    encoder.param(0, ParamId::Osc1Waveform, 3.0, |message| {
        messages[len] = message;
        len += 1;
    });
    encoder.param(0, ParamId::Osc1Enabled, 0.0, |message| {
        messages[len] = message;
        len += 1;
    });
    assert_eq!(messages[3], [0xb0, 38, 4]);
    assert_eq!(messages[7], [0xb0, 38, 0]);
}

#[test]
fn program_data_pack_round_trips_high_bits_and_partial_packet() {
    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    raw[..9].copy_from_slice(&[0x80, 0x01, 0xfe, 0x7f, 0xaa, 0x55, 0xff, 0x81, 0x42]);
    raw[PROGRAM_DATA_LEN - 2..].copy_from_slice(&[0x80, 0xff]);
    let mut packed = [0_u8; PROGRAM_PACKED_LEN];
    pack_program_data(&raw, &mut packed);
    assert_eq!(packed[0], 0b0101_0101);
    assert!(packed.iter().all(|byte| *byte < 0x80));

    let mut decoded = [0_u8; PROGRAM_DATA_LEN];
    unpack_program_data(&packed, &mut decoded);
    assert_eq!(decoded, raw);
}

#[test]
fn program_data_pack_uses_rev2_msb_bit_order() {
    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    raw[0] = 0x80;
    let mut packed = [0_u8; PROGRAM_PACKED_LEN];
    pack_program_data(&raw, &mut packed);
    assert_eq!(packed[0], 0b0100_0000);

    raw.fill(0);
    raw[6] = 0x80;
    pack_program_data(&raw, &mut packed);
    assert_eq!(packed[0], 0b0000_0001);
}

#[test]
fn factory_program_prefix_uses_program_offsets_not_nrpn_offsets() {
    // First two packed packets from Sequential's Rev2 factory bank v1.0.
    let mut packed = [0_u8; PROGRAM_PACKED_LEN];
    packed[..16].copy_from_slice(&[
        0x00, 0x18, 0x18, 0x30, 0x34, 0x01, 0x04, 0x32, 0x00, 0x2b, 0x29, 0x29, 0x01, 0x01, 0x00,
        0x00,
    ]);
    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    unpack_program_data(&packed, &mut raw);

    assert_eq!(program_nrpn_value(&raw, 0, 0), Some(24));
    assert_eq!(program_nrpn_value(&raw, 1, 0), Some(48));
    assert_eq!(program_nrpn_value(&raw, 2, 0), Some(1));
    assert_eq!(program_nrpn_value(&raw, 5, 0), Some(24));
    assert_eq!(program_nrpn_value(&raw, 6, 0), Some(52));
    assert_eq!(program_nrpn_value(&raw, 7, 0), Some(4));
}

#[test]
fn program_fields_round_trip_split_msb_values() {
    for number in [
        20, 58, 66, 69, 72, 75, 78, 81, 84, 87, 116, 118, 120, 122, 124,
    ] {
        for value in [0, 127, 254] {
            let mut raw = [0x55_u8; PROGRAM_DATA_LEN];
            store_program_nrpn(&mut raw, number, value, 0);
            assert_eq!(program_nrpn_value(&raw, number, 0), Some(value));
        }
    }
}

#[test]
fn decode_program_payload_reads_layer_a_name() {
    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    raw[LayerA::NAME_RANGE].copy_from_slice(b"LosVangelis2041     ");
    let mut packed = [0_u8; PROGRAM_PACKED_LEN];
    pack_program_data(&raw, &mut packed);
    let patch = decode_program_payload(&packed).unwrap().layer_a;
    assert_eq!(patch.name.as_str(), "LosVangelis2041");
}

#[test]
fn program_edit_buffer_round_trips_vca_initial_level() {
    let mut source = LayerPatch::default();
    source.amplifier.initial_level = 103.0 / 127.0;

    let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
    encode::program_edit_buffer(&program_with_layer_a(source.clone()), &mut message).unwrap();

    let decoded = decode::program_edit_buffer(&message).unwrap().layer_a;
    assert!(
        (decoded.amplifier.initial_level - source.amplifier.initial_level).abs() < 0.01,
        "decoded {} expected {}",
        decoded.amplifier.initial_level,
        source.amplifier.initial_level
    );
}

#[test]
fn program_edit_buffer_round_trips_supported_patch_fields() {
    let mut source = LayerPatch::default();
    source.osc1.waveform = 3;
    source.osc1.enabled = true;
    source.osc2.waveform = 2;
    source.osc2.enabled = true;
    source.osc1.shape_mod = 0.42;
    source.osc1.glide = 0.25;
    source.osc2.glide = 0.75;
    source.glide_mode = crate::GlideMode::FixedTimeAuto;
    source.glide_enabled = true;
    source.filter.cutoff = 1_234.0;
    source.filter.env_amount = -0.5;
    source.lfos[2].destination = ModDestination::FilterCutoff;
    source.effects.enabled = true;
    source.effects.effect_type = crate::EffectType::Reverb;
    source.effects.param1 = 0.75;
    source.mod_matrix.free_slots[0] = crate::ModMatrixSlot {
        enabled: true,
        source: ModSource::Lfo1,
        destination: ModDestination::Osc1ShapeMod,
        amount: -0.25,
    };

    for layer in [LayerId::A, LayerId::B] {
        let mut patch = Patch::default();
        *patch.layer_mut(layer) = source.clone();
        let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        let len = encode::program_edit_buffer(&patch, &mut message).unwrap();
        assert_eq!(len, PROGRAM_EDIT_BUFFER_SYSEX_LEN);
        assert_eq!(&message[..4], &[0xf0, 0x01, 0x2f, 0x03]);
        assert_eq!(message[len - 1], 0xf7);

        let decoded_patch = decode::program_edit_buffer(&message).unwrap();
        let decoded = decoded_patch.layer(layer);
        assert_eq!(decoded.osc1.waveform, 3, "{layer:?}");
        assert!(decoded.osc1.enabled, "{layer:?}");
        assert_eq!(decoded.osc2.waveform, 2, "{layer:?}");
        assert!(decoded.osc2.enabled, "{layer:?}");
        assert!((decoded.osc1.glide - 0.25).abs() < 0.01, "{layer:?}");
        assert!((decoded.osc2.glide - 0.75).abs() < 0.01, "{layer:?}");
        assert_eq!(decoded.glide_mode, crate::GlideMode::FixedTimeAuto);
        assert!(decoded.glide_enabled, "{layer:?}");
        assert!((decoded.osc1.shape_mod - source.osc1.shape_mod).abs() < 0.02);
        assert!((decoded.filter.cutoff - source.filter.cutoff).abs() < 50.0);
        assert!((decoded.filter.env_amount - source.filter.env_amount).abs() < 0.01);
        assert_eq!(decoded.lfos[2].destination, ModDestination::FilterCutoff);
        assert!(decoded.effects.enabled, "{layer:?}");
        assert_eq!(decoded.effects.effect_type, crate::EffectType::Reverb);
        assert!((decoded.effects.param1 - source.effects.param1).abs() < 0.01);
        let slot = decoded.mod_matrix.free_slots[0];
        assert!(slot.enabled, "{layer:?}");
        assert_eq!(slot.source, ModSource::Lfo1);
        assert_eq!(slot.destination, ModDestination::Osc1ShapeMod);
        assert!((slot.amount - source.mod_matrix.free_slots[0].amount).abs() < 0.01);
    }
}

#[test]
fn program_edit_buffer_round_trips_sequencer_wire_fields() {
    let mut source = Patch::default();
    for layer in [&mut source.layer_a, &mut source.layer_b] {
        layer.sequence.sequencer_type = SequencerType::Polyphonic;
        layer.sequence.gated_mode = GatedSequencerMode::KeyStep;
        layer.sequence.gated.tracks[0].destination =
            GatedDestination::Modulation(ModDestination::FilterCutoff);
        layer.sequence.gated.tracks[1].destination = GatedDestination::Slew;
        layer.sequence.gated.tracks[0].steps[0] = GatedStep::Value(125);
        layer.sequence.gated.tracks[0].steps[1] = GatedStep::Reset;
        layer.sequence.gated.tracks[0].steps[2] = GatedStep::Rest;
        layer.sequence.poly.steps[0].lanes[0].note = PolyNote::Note(127);
        layer.sequence.poly.steps[0].lanes[0].velocity = PolyVelocity::Velocity(127);
        layer.sequence.poly.steps[1].lanes[1].note = PolyNote::Tie;
        layer.sequence.poly.steps[1].lanes[1].velocity = PolyVelocity::Rest;
        layer.sequence.poly.steps[63].lanes[4].note = PolyNote::Note(1);
        layer.sequence.poly.steps[63].lanes[4].velocity = PolyVelocity::Velocity(1);
    }

    let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
    encode::program_edit_buffer(&source, &mut message).unwrap();
    let decoded = decode::program_edit_buffer(&message).unwrap();

    for layer in [&decoded.layer_a, &decoded.layer_b] {
        assert_eq!(layer.sequence.sequencer_type, SequencerType::Polyphonic);
        assert_eq!(layer.sequence.gated_mode, GatedSequencerMode::KeyStep);
        assert_eq!(
            layer.sequence.gated.tracks[0].destination,
            GatedDestination::Modulation(ModDestination::FilterCutoff)
        );
        assert_eq!(
            layer.sequence.gated.tracks[1].destination,
            GatedDestination::Slew
        );
        assert_eq!(
            layer.sequence.gated.tracks[0].steps[0],
            GatedStep::Value(125)
        );
        assert_eq!(layer.sequence.gated.tracks[0].steps[1], GatedStep::Reset);
        assert_eq!(layer.sequence.gated.tracks[0].steps[2], GatedStep::Rest);
        assert_eq!(
            layer.sequence.poly.steps[0].lanes[0].note,
            PolyNote::Note(127)
        );
        assert_eq!(
            layer.sequence.poly.steps[0].lanes[0].velocity,
            PolyVelocity::Velocity(127)
        );
        assert_eq!(layer.sequence.poly.steps[1].lanes[1].note, PolyNote::Tie);
        assert_eq!(
            layer.sequence.poly.steps[1].lanes[1].velocity,
            PolyVelocity::Rest
        );
        assert_eq!(
            layer.sequence.poly.steps[63].lanes[4].note,
            PolyNote::Note(1)
        );
        assert_eq!(
            layer.sequence.poly.steps[63].lanes[4].velocity,
            PolyVelocity::Velocity(1)
        );
    }

    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    unpack_program_data(&message[4..4 + PROGRAM_PACKED_LEN], &mut raw);
    assert_eq!(raw[138], 4);
    assert_eq!(raw[139], 1);
    assert_eq!(raw[140], 125);
    assert_eq!(raw[141], 126);
    assert_eq!(raw[142], 127);
    assert_eq!(raw[256], 127);
    assert_eq!(raw[320], 255);
    assert_eq!(raw[1024 + 112], 53);
}

#[test]
fn program_sequencer_type_uses_enum_polarity_independently_per_layer() {
    let mut source = Patch::default();
    source.layer_a.sequence.sequencer_type = SequencerType::Gated;
    source.layer_b.sequence.sequencer_type = SequencerType::Polyphonic;

    let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
    encode::program_edit_buffer(&source, &mut message).unwrap();
    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    unpack_program_data(&message[4..4 + PROGRAM_PACKED_LEN], &mut raw);
    assert_eq!(raw[139], 0);
    assert_eq!(raw[1024 + 139], 1);

    let decoded = decode::program_edit_buffer(&message).unwrap();
    assert_eq!(
        decoded.layer_a.sequence.sequencer_type,
        SequencerType::Gated
    );
    assert_eq!(
        decoded.layer_b.sequence.sequencer_type,
        SequencerType::Polyphonic
    );
}

#[test]
fn sequencer_live_nrpn_round_trips_both_layers_and_special_values() {
    let cases = [
        SequenceUpdate::Type(SequencerType::Gated),
        SequenceUpdate::Type(SequencerType::Polyphonic),
        SequenceUpdate::GatedMode(GatedSequencerMode::NoGateNoReset),
        SequenceUpdate::GatedDestination {
            track: 1,
            destination: GatedDestination::Slew,
        },
        SequenceUpdate::GatedStep {
            track: 3,
            step: 15,
            value: GatedStep::Rest,
        },
        SequenceUpdate::PolyNote {
            step: 63,
            lane: 5,
            value: PolyNote::Tie,
        },
        SequenceUpdate::PolyVelocity {
            step: 63,
            lane: 5,
            value: PolyVelocity::Velocity(127),
        },
    ];
    for layer in [LayerId::A, LayerId::B] {
        for expected in cases {
            let mut encoder = ControllerEncoder::default();
            let mut decoder = ControllerDecoder::default();
            let mut actual = None;
            encoder.sequence(0, layer, expected, |message| {
                decoder.control_change(0, message[1], message[2], |update| actual = Some(update));
            });
            assert_eq!(
                actual,
                Some(MidiUpdate::Sequence {
                    target: LayerTarget::Explicit(layer),
                    update: expected,
                })
            );
        }
    }
}

#[test]
fn atomic_poly_lane_edit_expands_to_lossless_note_and_velocity_nrpns() {
    let value = PolyLaneStep {
        note: PolyNote::Tie,
        velocity: PolyVelocity::Velocity(91),
    };
    for layer in [LayerId::A, LayerId::B] {
        let mut encoder = ControllerEncoder::default();
        let mut decoder = ControllerDecoder::default();
        let mut actual = Vec::<MidiUpdate, 2>::new();
        encoder.sequence(
            0,
            layer,
            SequenceUpdate::PolyLaneStep {
                step: 17,
                lane: 3,
                value,
            },
            |message| {
                decoder.control_change(0, message[1], message[2], |update| {
                    actual.push(update).unwrap()
                });
            },
        );
        assert_eq!(
            actual.as_slice(),
            &[
                MidiUpdate::Sequence {
                    target: LayerTarget::Explicit(layer),
                    update: SequenceUpdate::PolyNote {
                        step: 17,
                        lane: 3,
                        value: PolyNote::Tie,
                    },
                },
                MidiUpdate::Sequence {
                    target: LayerTarget::Explicit(layer),
                    update: SequenceUpdate::PolyVelocity {
                        step: 17,
                        lane: 3,
                        value: PolyVelocity::Velocity(91),
                    },
                },
            ]
        );
    }
}

#[test]
fn live_sequencer_on_off_nrpn_uses_on_for_gated_and_off_for_polyphonic() {
    for (layer, number) in [(LayerId::A, 183_u16), (LayerId::B, 2231_u16)] {
        for (sequencer_type, expected_raw) in [
            (SequencerType::Gated, 1_u16),
            (SequencerType::Polyphonic, 0_u16),
        ] {
            let mut messages = [[0_u8; 3]; 4];
            let mut len = 0;
            ControllerEncoder::default().sequence(
                0,
                layer,
                SequenceUpdate::Type(sequencer_type),
                |message| {
                    messages[len] = message;
                    len += 1;
                },
            );
            assert_eq!(len, 4);
            assert_eq!(
                u16::from(messages[0][2]) * 128 + u16::from(messages[1][2]),
                number
            );
            assert_eq!(
                u16::from(messages[2][2]) * 128 + u16::from(messages[3][2]),
                expected_raw
            );

            let mut param_messages = [[0_u8; 3]; 4];
            let mut param_len = 0;
            assert!(ControllerEncoder::default().param_for_layer(
                0,
                layer,
                ParamId::SequencerType,
                sequencer_type.index() as f32,
                |message| {
                    param_messages[param_len] = message;
                    param_len += 1;
                },
            ));
            assert_eq!(param_len, 4);
            assert_eq!(param_messages[3][2], expected_raw as u8);
        }
    }
}

#[test]
fn sequencer_transport_nrpn_180_round_trips_both_layers() {
    for layer in [LayerId::A, LayerId::B] {
        for running in [false, true] {
            let mut encoder = ControllerEncoder::default();
            let mut decoder = ControllerDecoder::default();
            let mut actual = None;
            encoder.sequencer_running(0, layer, running, |message| {
                decoder.control_change(0, message[1], message[2], |update| actual = Some(update));
            });
            assert_eq!(
                actual,
                Some(MidiUpdate::SequencerRunning {
                    target: LayerTarget::Explicit(layer),
                    running,
                })
            );
        }
    }
}

#[test]
fn sequencer_record_nrpn_181_round_trips_both_layers() {
    for layer in [LayerId::A, LayerId::B] {
        for recording in [false, true] {
            let mut encoder = ControllerEncoder::default();
            let mut decoder = ControllerDecoder::default();
            let mut actual = None;
            encoder.sequencer_recording(0, layer, recording, |message| {
                decoder.control_change(0, message[1], message[2], |update| actual = Some(update));
            });
            assert_eq!(
                actual,
                Some(MidiUpdate::SequencerRecording {
                    target: LayerTarget::Explicit(layer),
                    recording,
                })
            );
        }
    }
}

#[test]
fn program_edit_buffer_rejects_malformed_messages() {
    let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
    encode::program_edit_buffer(&Patch::default(), &mut message).unwrap();
    assert!(matches!(
        decode::program_edit_buffer(&message[..message.len() - 1]),
        Err(SysexError::InvalidLength)
    ));
    message[1] = 2;
    assert!(matches!(
        decode::program_edit_buffer(&message),
        Err(SysexError::InvalidManufacturer)
    ));
    message[1] = 1;
    message[4] = 0x80;
    assert!(matches!(
        decode::program_edit_buffer(&message),
        Err(SysexError::NonSevenBitData)
    ));
}

#[test]
fn program_edit_buffer_round_trips_both_layers_independently() {
    let mut source = Patch::default();
    source.layer_a.name.push_str("Layer A Sound").unwrap();
    source.layer_a.filter.resonance = 0.25;
    source.layer_a.osc1.waveform = 3;
    source.layer_b.name.push_str("Layer B Sound").unwrap();
    source.layer_b.filter.resonance = 0.75;
    source.layer_b.osc1.frequency = 72.0;
    let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
    encode::program_edit_buffer(&source, &mut message).unwrap();

    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    unpack_program_data(&message[4..4 + PROGRAM_PACKED_LEN], &mut raw);
    assert_eq!(raw[4] & 0x7f, 4);
    assert_eq!(raw[LayerB::DATA_OFFSET + 4] & 0x7f, 1);
    assert_eq!(&raw[LayerA::NAME_RANGE], b"Layer A Sound       ");
    assert_eq!(&raw[LayerB::NAME_RANGE], b"Layer B Sound       ");

    let decoded = decode::program_edit_buffer(&message).unwrap();
    assert_eq!(decoded.layer_a.name, source.layer_a.name);
    assert_eq!(decoded.layer_b.name, source.layer_b.name);
    assert!((decoded.layer_a.filter.resonance - 0.25).abs() < 0.01);
    assert!((decoded.layer_b.filter.resonance - 0.75).abs() < 0.01);
    assert_eq!(decoded.layer_b.osc1.frequency, 72.0);
}

#[test]
fn program_edit_buffer_requires_complete_output_capacity() {
    let mut output = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN - 1];
    assert_eq!(
        encode::program_edit_buffer(&Patch::default(), &mut output),
        Err(SysexError::OutputTooSmall)
    );
}

#[test]
fn stored_program_data_decodes_metadata_and_patch() {
    let mut source = LayerPatch::default();
    source.filter.resonance = 1.0;
    let message = program_data_message(7, 127, &source);
    let decoded = decode::program_data(&message).unwrap();
    assert_eq!(decoded.bank, 7);
    assert_eq!(decoded.program, 127);
    assert_eq!(decoded.patch.layer_a.filter.resonance, 1.0);
}

#[test]
fn stored_program_data_rejects_invalid_metadata_and_payload() {
    let mut message = program_data_message(0, 0, &LayerPatch::default());
    message[4] = 8;
    assert!(matches!(
        decode::program_data(&message),
        Err(SysexError::InvalidBank)
    ));
    message[4] = 0;
    message[3] = 3;
    assert!(matches!(
        decode::program_data(&message),
        Err(SysexError::UnsupportedCommand)
    ));
    message[3] = 2;
    message[6] = 0x80;
    assert!(matches!(
        decode::program_data(&message),
        Err(SysexError::NonSevenBitData)
    ));
    assert!(matches!(
        decode::program_data(&message[..message.len() - 1]),
        Err(SysexError::InvalidLength)
    ));
}

#[test]
fn layer_mode_raw_values_follow_the_reference_contract() {
    assert_eq!(layer_mode_from_raw(0), Some(LayerMode::Normal));
    assert_eq!(layer_mode_from_raw(1), Some(LayerMode::Stack));
    assert_eq!(layer_mode_from_raw(2), Some(LayerMode::Split));
    assert_eq!(layer_mode_from_raw(3), None);

    assert_eq!(layer_mode_raw(LayerMode::Normal), 0);
    assert_eq!(layer_mode_raw(LayerMode::Stack), 1);
    assert_eq!(layer_mode_raw(LayerMode::Split), 2);
}

#[test]
fn live_ab_mode_and_split_point_round_trip_as_program_global_nrpns() {
    for mode in [LayerMode::Normal, LayerMode::Stack, LayerMode::Split] {
        let mut encoder = ControllerEncoder::default();
        let mut decoder = ControllerDecoder::default();
        let mut actual = None;
        encoder.layer_mode(0, mode, |message| {
            decoder.control_change(0, message[1], message[2], |update| actual = Some(update));
        });
        assert_eq!(actual, Some(MidiUpdate::LayerMode(mode)));
    }

    let mut encoder = ControllerEncoder::default();
    let mut decoder = ControllerDecoder::default();
    let mut actual = None;
    encoder.split_point(0, 72, |message| {
        decoder.control_change(0, message[1], message[2], |update| actual = Some(update));
    });
    assert_eq!(actual, Some(MidiUpdate::SplitPoint(72)));

    let mut decoder = ControllerDecoder::default();
    let mut updates = Vec::<MidiUpdate, 4>::new();
    decoder.control_change(0, 18, 1, |update| {
        updates.push(update).unwrap();
    });
    decoder.control_change(0, 39, 60, |update| {
        updates.push(update).unwrap();
    });
    assert_eq!(
        updates.as_slice(),
        &[
            MidiUpdate::LayerMode(LayerMode::Stack),
            MidiUpdate::SplitPoint(60),
        ]
    );
}

#[test]
fn layer_b_nrpn_and_edit_layer_updates_are_explicit() {
    let mut decoder = ControllerDecoder::default();
    let mut updates = Vec::<MidiUpdate, 8>::new();
    emit_nrpn(0, LayerB::NRPN_OFFSET + 16, 96, &mut |message| {
        decoder.control_change(0, message[1], message[2], |update| {
            updates.push(update).unwrap();
        });
    });
    assert_eq!(
        updates.as_slice(),
        &[param_update(
            LayerTarget::Explicit(LayerId::B),
            ParamId::FilterResonance,
            96.0 / 127.0,
        )]
    );

    updates.clear();
    emit_nrpn(0, 4190, 1, &mut |message| {
        decoder.control_change(0, message[1], message[2], |update| {
            updates.push(update).unwrap();
        });
    });
    assert_eq!(updates.as_slice(), &[MidiUpdate::EditLayer(LayerId::B)]);

    updates.clear();
    decoder.control_change(0, 71, 64, |update| updates.push(update).unwrap());
    assert_eq!(
        updates.as_slice(),
        &[param_update(
            LayerTarget::Edit,
            ParamId::FilterResonance,
            64.0 / 127.0,
        )]
    );

    updates.clear();
    emit_nrpn(0, 16, 32, &mut |message| {
        decoder.control_change(0, message[1], message[2], |update| {
            updates.push(update).unwrap();
        });
    });
    assert_eq!(
        updates.as_slice(),
        &[param_update(
            LayerTarget::Explicit(LayerId::A),
            ParamId::FilterResonance,
            32.0 / 127.0,
        )]
    );
}

#[test]
fn controller_encoder_addresses_layer_b_and_edit_layer_explicitly() {
    let mut encoder = ControllerEncoder::default();
    let mut decoder = ControllerDecoder::default();
    let mut updates = Vec::<MidiUpdate, 8>::new();

    assert!(
        encoder.param_for_layer(0, LayerId::B, ParamId::FilterResonance, 0.75, |message| {
            decoder.control_change(0, message[1], message[2], |update| {
                updates.push(update).unwrap();
            });
        },)
    );
    assert!(matches!(
        updates.as_slice(),
        [MidiUpdate::Param {
            target: LayerTarget::Explicit(LayerId::B),
            param: ParamId::FilterResonance,
            ..
        }]
    ));

    updates.clear();
    assert!(encoder.param_for_layer(0, LayerId::A, ParamId::Osc1Enabled, 0.0, |_| {}));
    assert!(
        encoder.param_for_layer(0, LayerId::B, ParamId::Osc1Waveform, 3.0, |message| {
            decoder.control_change(0, message[1], message[2], |update| {
                updates.push(update).unwrap();
            });
        },)
    );
    assert!(updates.iter().any(|update| matches!(
        update,
        MidiUpdate::Param {
            target: LayerTarget::Explicit(LayerId::B),
            param: ParamId::Osc1Enabled,
            value,
        } if *value == 1.0
    )));

    updates.clear();
    encoder.modulation_for_layer(
        0,
        LayerId::B,
        crate::ModRoute::Free(0),
        true,
        ModSource::Lfo1,
        ModDestination::FilterCutoff,
        0.5,
        |message| {
            decoder.control_change(0, message[1], message[2], |update| {
                updates.push(update).unwrap();
            });
        },
    );
    assert_eq!(updates.len(), 3);
    assert!(updates.iter().all(|update| matches!(
        update,
        MidiUpdate::Modulation {
            target: LayerTarget::Explicit(LayerId::B),
            ..
        }
    )));

    updates.clear();
    encoder.edit_layer(0, LayerId::B, |message| {
        decoder.control_change(0, message[1], message[2], |update| {
            updates.push(update).unwrap();
        });
    });
    assert_eq!(updates.as_slice(), &[MidiUpdate::EditLayer(LayerId::B)]);
}

#[test]
fn interleaved_layer_lfo_state_is_independent() {
    let mut decoder = ControllerDecoder::default();
    let mut updates = Vec::<MidiUpdate, 16>::new();

    decode_nrpn(&mut decoder, 41, 1, &mut updates);
    updates.clear();
    decode_nrpn(&mut decoder, LayerB::NRPN_OFFSET + 37, 72, &mut updates);
    assert!(matches!(
        updates.as_slice(),
        [MidiUpdate::Param {
            target: LayerTarget::Explicit(LayerId::B),
            param: ParamId::Lfo1Rate,
            ..
        }]
    ));

    updates.clear();
    decode_nrpn(&mut decoder, 37, 72, &mut updates);
    assert!(matches!(
        updates.as_slice(),
        [MidiUpdate::Param {
            target: LayerTarget::Explicit(LayerId::A),
            param: ParamId::Lfo1SyncDivision,
            ..
        }]
    ));
}

#[test]
fn rev2_oscillator_shape_uses_rev2_waveform_order() {
    for (raw, expected_waveform) in [(2, 1.0), (3, 2.0), (4, 3.0)] {
        let mut waveform = None;
        emit_osc_shape(
            &mut |update| {
                if let MappedUpdate::Param(ParamId::Osc1Waveform, value) = update {
                    waveform = Some(value);
                }
            },
            true,
            raw,
        );
        assert_eq!(waveform, Some(expected_waveform));
    }
}

#[test]
fn shape_mod_nrpn_round_trips() {
    let mut encoder = ControllerEncoder::default();
    let mut messages = [[0_u8; 3]; 4];
    let mut len = 0;
    encoder.param(0, ParamId::Osc1ShapeMod, 0.5, |message| {
        messages[len] = message;
        len += 1;
    });
    assert_eq!(len, 4);
    let number = u16::from(messages[0][2]) * 128 + u16::from(messages[1][2]);
    let value = u16::from(messages[2][2]) * 128 + u16::from(messages[3][2]);
    assert_eq!(number, 102);
    assert_eq!(value, 50);

    let mut decoded = None;
    map_nrpn(number, value, &mut |update| decoded = Some(update));
    assert_eq!(
        decoded,
        Some(MappedUpdate::Param(ParamId::Osc1ShapeMod, 50.0 / 99.0,))
    );
}

#[test]
fn shape_mod_cc_round_trips() {
    let mut decoded = None;
    map_cc(30, 64, &mut |update| decoded = Some(update));
    assert_eq!(
        decoded,
        Some(MappedUpdate::Param(ParamId::Osc1ShapeMod, 64.0 / 127.0,))
    );
}

#[test]
fn mod_destination_matches_cc_chart_indices() {
    assert_eq!(ModDestination::from_index(4), ModDestination::OscMix);
    assert_eq!(ModDestination::from_index(7), ModDestination::Osc1ShapeMod);
    assert_eq!(ModDestination::Osc1ShapeMod.index(), 7);
}

fn param_update(target: LayerTarget, param: ParamId, value: f32) -> MidiUpdate {
    MidiUpdate::Param {
        target,
        param,
        value,
    }
}

fn decode_nrpn<const N: usize>(
    decoder: &mut ControllerDecoder,
    number: u16,
    value: u16,
    updates: &mut Vec<MidiUpdate, N>,
) {
    emit_nrpn(0, number, value, &mut |message| {
        decoder.control_change(0, message[1], message[2], |update| {
            updates.push(update).unwrap();
        });
    });
}

fn program_with_layer_a(layer_a: LayerPatch) -> Patch {
    Patch {
        layer_a,
        ..Patch::default()
    }
}

fn program_data_message(bank: u8, program: u8, patch: &LayerPatch) -> [u8; PROGRAM_DATA_SYSEX_LEN] {
    let mut edit = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
    encode::program_edit_buffer(&program_with_layer_a(patch.clone()), &mut edit).unwrap();
    let mut message = [0_u8; PROGRAM_DATA_SYSEX_LEN];
    message[..4].copy_from_slice(&[0xf0, 0x01, 0x2f, 0x02]);
    message[4] = bank;
    message[5] = program;
    message[6..6 + PROGRAM_PACKED_LEN].copy_from_slice(&edit[4..4 + PROGRAM_PACKED_LEN]);
    message[PROGRAM_DATA_SYSEX_LEN - 1] = 0xf7;
    message
}
