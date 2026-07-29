use crate::{
    LfoSyncDivision, ModDestination, ParamId,
    dsp::MIN_LFO_RATE_HZ,
    midi::{
        p08::{
            decode,
            layer::{Layer, LayerA},
            map::{
                MidiUpdate, emit_osc_shape, map_lfo_nrpn, map_nrpn, nrpn_max, p08_lfo_rate_hz,
                p08_lfo_waveform, p08_mod_destination, program_nrpn_value,
            },
            program::{PROGRAM_DATA_LEN, PROGRAM_DATA_SYSEX_LEN},
        },
        rev2::SysexError,
    },
};

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
