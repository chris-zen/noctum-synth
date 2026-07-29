use super::layer::{Layer, LayerA};
use super::map::{
    MidiUpdate, emit_osc_shape, map_lfo_nrpn, map_nrpn, nrpn_max, p08_lfo_rate_hz,
    p08_lfo_waveform, p08_mod_destination, program_nrpn_value,
};
use super::program::{PROGRAM_DATA_LEN, PROGRAM_DATA_SYSEX_LEN, PROGRAM_PACKED_LEN};
use super::*;
use crate::dsp::MIN_LFO_RATE_HZ;
#[cfg(not(feature = "fast-math"))]
use crate::math::WideF32;
use crate::midi::rev2::SysexError;
use crate::{ControlMessage, LfoSyncDivision, ModDestination, ParamId, VoiceManager};

fn render_frames<const PACKS: usize>(voices: &mut VoiceManager<PACKS>, frames: usize) {
    let mut ctx = crate::create_render_context!();
    for _ in 0..frames {
        voices.next(&mut ctx);
    }
}

const FACTORY_SYSEX: &[u8] =
    include_bytes!("../../../../Prophet_08_Programs+ReadMe/Prophet_08_Programs_v1.0.syx");

fn factory_message(bank: usize, program: usize) -> &'static [u8] {
    let offset = (bank * 128 + program) * PROGRAM_DATA_SYSEX_LEN;
    &FACTORY_SYSEX[offset..offset + PROGRAM_DATA_SYSEX_LEN]
}

#[test]
fn decode_patch_payload_reads_shared_name_for_both_layers() {
    let decoded = decode::program_data(factory_message(0, 0)).unwrap();
    assert_eq!(decoded.patch.layer_a.name.as_str(), "Wagnerian");
    assert_eq!(decoded.patch.layer_b.name.as_str(), "Wagnerian");

    let decoded = decode::program_data(factory_message(0, 1)).unwrap();
    assert_eq!(decoded.patch.layer_a.name.as_str(), "Tom Sawyer");
    assert!(decoded.patch.layer_a.unison_enabled);
    assert_eq!(decoded.patch.layer_a.unison_mode, crate::UnisonMode::V8);
    assert_eq!(
        decoded.patch.layer_a.key_mode,
        crate::KeyMode::HighRetrigger
    );
    assert!(decoded.patch.layer_a.glide_enabled);
    assert_eq!(
        decoded.patch.layer_a.glide_mode,
        crate::GlideMode::FixedRate
    );
    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    crate::midi::prophet::unpack_program_data(
        &factory_message(0, 1)[6..6 + PROGRAM_PACKED_LEN],
        &mut raw,
    );
    assert_eq!(program_nrpn_value(&raw, 36, LayerA::DATA_OFFSET), Some(118));
    assert!((decoded.patch.layer_a.amplifier.eg_release - 21.415_247).abs() < 0.001);

    let decoded = decode::program_data(factory_message(1, 0)).unwrap();
    assert_eq!(decoded.patch.layer_a.name.as_str(), "AnalogWurlyRoids");
}

#[test]
fn all_p08_envelope_stages_use_prophet_timing_conversion() {
    for (number, param) in [
        (23, ParamId::FilterEgAttack),
        (24, ParamId::FilterEgDecay),
        (33, ParamId::AmpEgAttack),
        (34, ParamId::AmpEgDecay),
        (61, ParamId::AuxEgAttack),
        (62, ParamId::AuxEgDecay),
    ] {
        let mut decoded = None;
        map_nrpn(number, 31, &mut |update| decoded = Some(update));
        let Some(MidiUpdate::Param(decoded_param, seconds)) = decoded else {
            panic!("parameter {number} did not decode");
        };
        assert_eq!(decoded_param, param);
        assert!((seconds - 0.135).abs() < 1.0e-6);
    }

    for (number, param) in [
        (26, ParamId::FilterEgRelease),
        (36, ParamId::AmpEgRelease),
        (64, ParamId::AuxEgRelease),
    ] {
        let mut decoded = None;
        map_nrpn(number, 127, &mut |update| decoded = Some(update));
        let Some(MidiUpdate::Param(decoded_param, seconds)) = decoded else {
            panic!("parameter {number} did not decode");
        };
        assert_eq!(decoded_param, param);
        assert!((seconds - 40.0).abs() < 1.0e-6);
    }
}

#[test]
fn tom_sawyer_unison_glide_releases_every_ordered_note_sequence() {
    let patch = decode::program_data(factory_message(0, 1))
        .unwrap()
        .patch
        .layer_a;
    assert_eq!(patch.name.as_str(), "Tom Sawyer");
    assert!(patch.unison_enabled && patch.glide_enabled);
    assert_eq!(patch.unison_mode, crate::UnisonMode::V8);
    assert_eq!(patch.key_mode, crate::KeyMode::HighRetrigger);

    let release_orders = [
        [48, 59, 72],
        [48, 72, 59],
        [59, 48, 72],
        [59, 72, 48],
        [72, 48, 59],
        [72, 59, 48],
    ];
    for frames_between_events in [0, 1, 32, 256] {
        for release_order in release_orders {
            let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(48_000.0);
            voices.apply_patch(&patch);
            for note in [59, 48, 72] {
                voices.handle_control(ControlMessage::NoteOn {
                    note,
                    velocity: 1.0,
                });
                render_frames(&mut voices, frames_between_events);
            }
            for note in release_order {
                voices.handle_control(ControlMessage::NoteOff { note });
                render_frames(&mut voices, frames_between_events);
            }

            assert!(
                voices.active_notes().is_empty(),
                "gate or pending note survived release order {release_order:?} with delay {frames_between_events}"
            );
        }
    }
}

#[test]
fn tom_sawyer_patch_rebuild_cannot_resurrect_released_pending_notes() {
    let patch = decode::program_data(factory_message(0, 1))
        .unwrap()
        .patch
        .layer_a;
    let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(48_000.0);

    voices.handle_control(ControlMessage::NoteOn {
        note: 59,
        velocity: 1.0,
    });
    voices.apply_patch(&patch);
    voices.handle_control(ControlMessage::NoteOn {
        note: 72,
        velocity: 1.0,
    });
    voices.apply_patch(&patch);
    voices.handle_control(ControlMessage::NoteOff { note: 72 });
    voices.handle_control(ControlMessage::NoteOff { note: 59 });
    for _ in 0..512 {
        render_frames(&mut voices, 1);
    }

    assert!(voices.active_notes().is_empty());
}

#[test]
#[cfg(not(feature = "fast-math"))]
fn tom_sawyer_last_retrigger_glides_in_place_without_pending_voices() {
    let mut patch = decode::program_data(factory_message(0, 1))
        .unwrap()
        .patch
        .layer_a;
    patch.key_mode = crate::KeyMode::LastRetrigger;
    let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(48_000.0);
    voices.apply_patch(&patch);
    voices.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });
    let before = voices[0].oscillators().osc1_frequency_hz().to_array()[0];

    voices.handle_control(ControlMessage::NoteOn {
        note: 72,
        velocity: 1.0,
    });
    let start = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
    assert!(
        (start - before).abs() < 0.1,
        "glide should start from current pitch; before={before}, start={start}"
    );
    for voice in 0..8 {
        assert!(!voices[voice / WideF32::LANES].has_pending_note(voice % WideF32::LANES));
    }

    // Glide should make progress within the first 32 samples.
    for _ in 0..32 {
        render_frames(&mut voices, 1);
    }
    let progressing = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
    assert!(progressing > start);
    assert!(progressing < before * 2.0);

    // After the glide completes (well within 1 s) the frequency must be
    // stable — no further drift between consecutive reads.
    for _ in 0..48_000 {
        render_frames(&mut voices, 1);
    }
    let pre = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
    for _ in 0..1_000 {
        render_frames(&mut voices, 1);
    }
    let post = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
    assert!(
        (post - pre).abs() / pre < 1.0e-6,
        "frequency should be stable after glide completes; pre {pre}, post {post}"
    );
}

#[test]
fn tom_sawyer_final_release_reaches_idle() {
    let patch = decode::program_data(factory_message(0, 1))
        .unwrap()
        .patch
        .layer_a;
    const SAMPLE_RATE: f32 = 1_000.0;
    let mut voices = VoiceManager::<2>::new(SAMPLE_RATE);
    voices.apply_patch(&patch);
    voices.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });
    for _ in 0..SAMPLE_RATE as usize {
        render_frames(&mut voices, 1);
    }
    voices.handle_control(ControlMessage::NoteOff { note: 60 });
    assert!(voices.active_notes().is_empty());
    assert!(
        voices.active_voice_count() > 0,
        "release tail should still render"
    );

    let release_frames = ((patch.amplifier.eg_release + 1.0) * SAMPLE_RATE) as usize;
    for _ in 0..release_frames {
        render_frames(&mut voices, 1);
    }
    assert_eq!(
        voices.active_voice_count(),
        0,
        "Tom Sawyer release envelope never reached idle"
    );
}

#[test]
#[cfg(not(feature = "wide-1"))]
fn tom_sawyer_unison_glide_matches_pressed_key_model_under_adversarial_ordering() {
    let base_patch = decode::program_data(factory_message(0, 1))
        .unwrap()
        .patch
        .layer_a;
    for key_mode in crate::KeyMode::ALL {
        for glide_mode in crate::GlideMode::ALL {
            let mut patch = base_patch.clone();
            patch.key_mode = key_mode;
            patch.glide_mode = glide_mode;
            let mut voices = VoiceManager::<2>::new(48_000.0);
            voices.apply_patch(&patch);
            let mut pressed = heapless::Vec::<u8, 128>::new();
            let mut random = 0x6d2b_79f5_u32;

            for event in 0..4_096 {
                random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let note = 48 + ((random >> 16) % 25) as u8;
                if random & 1 == 0 {
                    if let Some(index) = pressed.iter().position(|held| *held == note) {
                        pressed.remove(index);
                    }
                    pressed.push(note).unwrap();
                    voices.handle_control(ControlMessage::NoteOn {
                        note,
                        velocity: 1.0,
                    });
                } else {
                    if let Some(index) = pressed.iter().position(|held| *held == note) {
                        pressed.remove(index);
                    }
                    voices.handle_control(ControlMessage::NoteOff { note });
                }
                for _ in 0..((random >> 8) & 3) {
                    render_frames(&mut voices, 1);
                }

                let active = voices.active_notes();
                if pressed.is_empty() {
                    assert!(
                        active.is_empty(),
                        "{key_mode:?}/{glide_mode:?} event {event}: unpressed note remained gated or pending"
                    );
                    continue;
                }
                let selected = match key_mode {
                    crate::KeyMode::Low | crate::KeyMode::LowRetrigger => {
                        *pressed.iter().min().unwrap()
                    }
                    crate::KeyMode::High | crate::KeyMode::HighRetrigger => {
                        *pressed.iter().max().unwrap()
                    }
                    crate::KeyMode::Last | crate::KeyMode::LastRetrigger => {
                        *pressed.last().unwrap()
                    }
                };
                assert_eq!(
                    active.len(),
                    8,
                    "{key_mode:?}/{glide_mode:?} event {event}: incomplete unison group"
                );
                assert!(
                    active.iter().all(|note| note == selected),
                    "{key_mode:?}/{glide_mode:?} event {event}: selected {selected}, active {active:?}, pressed={pressed:?}"
                );
            }

            voices.handle_control(ControlMessage::AllNotesOff);
            assert!(voices.active_notes().is_empty());
        }
    }
}

#[test]
fn stored_program_data_decodes_factory_metadata() {
    let decoded = decode::program_data(factory_message(0, 0)).unwrap();
    assert_eq!(decoded.bank, 0);
    assert_eq!(decoded.program, 0);
    assert!(decoded.patch.layer_a.osc1.enabled);

    let decoded = decode::program_data(factory_message(1, 0)).unwrap();
    assert_eq!(decoded.bank, 1);
    assert_eq!(decoded.program, 0);
}

#[test]
fn factory_program_decodes_vca_initial_level() {
    let decoded = decode::program_data(factory_message(0, 54)).unwrap();
    assert!(
        (decoded.patch.layer_a.amplifier.initial_level - 103.0 / 127.0).abs() < 0.01,
        "decoded {}",
        decoded.patch.layer_a.amplifier.initial_level
    );
}

#[test]
fn factory_program_pan_spread_uses_documented_program_indices() {
    let message = factory_message(0, 0);
    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    crate::midi::prophet::unpack_program_data(&message[6..6 + PROGRAM_PACKED_LEN], &mut raw);
    let decoded = decode::program_data(message).unwrap();

    assert_eq!(raw[28], 0, "factory fixture layer A Pan Spread changed");
    assert_eq!(raw[228], 49, "factory fixture layer B Pan Spread changed");
    assert_eq!(
        decoded.patch.layer_a.amplifier.pan_spread,
        f32::from(raw[28]) / 127.0
    );
    assert_eq!(
        decoded.patch.layer_a.amplifier.pan_mod_mode,
        crate::PanModMode::Alternate
    );
}

#[test]
fn program_values_above_127_use_the_documented_msb_sideband() {
    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    raw[20] = 1;
    raw[14] = 0x80;
    assert_eq!(program_nrpn_value(&raw, 20, LayerA::DATA_OFFSET), Some(129));
}

#[test]
fn oscillator_shape_uses_p08_waveform_order() {
    let mut updates = [None; 3];
    let mut len = 0;
    emit_osc_shape(
        &mut |update| {
            updates[len] = Some(update);
            len += 1;
        },
        true,
        2,
    );
    assert_eq!(len, 3);
    assert_eq!(
        updates,
        [
            Some(MidiUpdate::Param(ParamId::Osc1Enabled, 1.0)),
            Some(MidiUpdate::Param(ParamId::Osc1Waveform, 2.0)),
            Some(MidiUpdate::Param(ParamId::Osc1ShapeMod, 0.0)),
        ]
    );

    let mut updates = [None; 3];
    let mut len = 0;
    emit_osc_shape(
        &mut |update| {
            updates[len] = Some(update);
            len += 1;
        },
        true,
        3,
    );
    assert_eq!(len, 3);
    assert_eq!(
        updates,
        [
            Some(MidiUpdate::Param(ParamId::Osc1Enabled, 1.0)),
            Some(MidiUpdate::Param(ParamId::Osc1Waveform, 1.0)),
            Some(MidiUpdate::Param(ParamId::Osc1ShapeMod, 0.0)),
        ]
    );
}

#[test]
fn oscillator_keyboard_parameters_map_for_both_oscillators() {
    for (number, raw, expected) in [
        (4, 0, MidiUpdate::Param(ParamId::Osc1KeyboardOn, 0.0)),
        (4, 1, MidiUpdate::Param(ParamId::Osc1KeyboardOn, 1.0)),
        (9, 0, MidiUpdate::Param(ParamId::Osc2KeyboardOn, 0.0)),
        (9, 1, MidiUpdate::Param(ParamId::Osc2KeyboardOn, 1.0)),
    ] {
        let mut update = None;
        map_nrpn(number, raw, &mut |value| update = Some(value));
        assert_eq!(update, Some(expected));
        assert_eq!(nrpn_max(number), Some(1));
    }
}

#[test]
fn glide_parameters_map_to_the_shared_patch_model() {
    for (number, raw, expected) in [
        (3, 64, MidiUpdate::Param(ParamId::Osc1Glide, 64.0 / 127.0)),
        (8, 127, MidiUpdate::Param(ParamId::Osc2Glide, 1.0)),
        (11, 3, MidiUpdate::Param(ParamId::GlideMode, 3.0)),
    ] {
        let mut update = None;
        map_nrpn(number, raw, &mut |value| update = Some(value));
        assert_eq!(update, Some(expected));
    }

    let decoded = (0..256)
        .map(|program| decode::program_data(factory_message(program / 128, program % 128)).unwrap())
        .find(|program| {
            program.patch.layer_a.osc1.glide > 0.0 || program.patch.layer_a.osc2.glide > 0.0
        })
        .expect("factory bank should contain a glide program");
    assert!(decoded.patch.layer_a.glide_enabled);
}

#[test]
fn p08_unison_modes_translate_to_rev2_first_patch_values() {
    let mut updates = [None; 2];
    let mut len = 0;
    map_nrpn(96, 4, &mut |update| {
        updates[len] = Some(update);
        len += 1;
    });
    assert_eq!(len, 2);
    assert_eq!(
        updates,
        [
            Some(MidiUpdate::Param(
                ParamId::UnisonMode,
                crate::UnisonMode::V8.index() as f32,
            )),
            Some(MidiUpdate::Param(ParamId::UnisonDetune, 16.0)),
        ]
    );

    let mut update = None;
    map_nrpn(95, 3, &mut |value| update = Some(value));
    assert_eq!(
        update,
        Some(MidiUpdate::Param(
            ParamId::KeyMode,
            crate::KeyMode::HighRetrigger.index() as f32,
        ))
    );
}

#[test]
fn lfo_waveform_uses_p08_shape_order() {
    assert_eq!(p08_lfo_waveform(1), 2.0);
    assert_eq!(p08_lfo_waveform(2), 1.0);
}

#[test]
fn combined_shape_raw_value_decodes_pulse_and_shape_mod() {
    let mut updates = [None; 3];
    let mut len = 0;
    emit_osc_shape(
        &mut |update| {
            updates[len] = Some(update);
            len += 1;
        },
        true,
        54,
    );
    assert_eq!(len, 3);
    assert_eq!(
        updates,
        [
            Some(MidiUpdate::Param(ParamId::Osc1Enabled, 1.0)),
            Some(MidiUpdate::Param(ParamId::Osc1Waveform, 3.0)),
            Some(MidiUpdate::Param(ParamId::Osc1ShapeMod, 50.0 / 99.0)),
        ]
    );
}

#[test]
fn mod_destination_maps_p08_indices_to_internal_destinations() {
    assert_eq!(p08_mod_destination(3), ModDestination::OscAllFrequency);
    assert_eq!(p08_mod_destination(4), ModDestination::OscMix);
    assert_eq!(p08_mod_destination(9), ModDestination::FilterCutoff);
    assert_eq!(p08_mod_destination(25), ModDestination::AmpEnvAmount);
}

#[test]
fn p08_lfo_rate_decodes_free_rate_and_all_sync_divisions() {
    assert!(p08_lfo_rate_hz(37) > MIN_LFO_RATE_HZ);
    for raw in 151..=166 {
        let division = LfoSyncDivision::from_p08_raw(raw);
        assert_eq!(division.p08_raw(), raw);
        let mut updates = [None; 2];
        let mut len = 0;
        map_lfo_nrpn(37, raw, &mut |update| {
            updates[len] = Some(update);
            len += 1;
        });
        assert_eq!(
            updates,
            [
                Some(MidiUpdate::Param(ParamId::Lfo1ClockSync, 1.0)),
                Some(MidiUpdate::Param(
                    ParamId::Lfo1SyncDivision,
                    division.index() as f32,
                )),
            ]
        );
    }
}

#[test]
fn rejects_rev2_program_data_message() {
    let mut message = [0_u8; PROGRAM_DATA_SYSEX_LEN];
    message[0] = 0xf0;
    message[1] = 0x01;
    message[2] = 0x2f;
    message[3] = 0x02;
    message[PROGRAM_DATA_SYSEX_LEN - 1] = 0xf7;
    assert!(matches!(
        decode::program_data(&message),
        Err(SysexError::InvalidModel)
    ));
}
