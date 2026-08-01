//! Official Sequential Prophet '08 factory-bank regressions.

use synth_core::{
    ClockDivision, GatedDestination, GatedSequencerMode, GatedStep, GlideMode, KeyMode, LayerPatch,
    PanModMode, SequencerType, UnisonMode,
    midi::{
        p08::{PROGRAM_DATA_LEN, PROGRAM_DATA_SYSEX_LEN, PROGRAM_PACKED_LEN, decode},
        prophet::unpack_program_data,
    },
};

const FACTORY_SYSEX: &[u8] =
    include_bytes!("../../../Prophet_08_Programs+ReadMe/Prophet_08_Programs_v1.0.syx");

#[test]
fn decode_patch_payload_reads_shared_name_for_both_layers() {
    let decoded = decode::program_data(factory_message(0, 0)).unwrap();
    assert_eq!(decoded.patch.layer_a.name.as_str(), "Wagnerian");
    assert_eq!(decoded.patch.layer_b.name.as_str(), "Wagnerian");

    let decoded = decode::program_data(factory_message(0, 1)).unwrap();
    assert_eq!(decoded.patch.layer_a.name.as_str(), "Tom Sawyer");
    assert!(decoded.patch.layer_a.unison_enabled);
    assert_eq!(decoded.patch.layer_a.unison_mode, UnisonMode::V8);
    assert_eq!(decoded.patch.layer_a.key_mode, KeyMode::HighRetrigger);
    assert!(decoded.patch.layer_a.glide_enabled);
    assert_eq!(decoded.patch.layer_a.glide_mode, GlideMode::FixedRate);
    assert!((decoded.patch.layer_a.amplifier.eg_release - 21.415_247).abs() < 0.001);

    let decoded = decode::program_data(factory_message(1, 0)).unwrap();
    assert_eq!(decoded.patch.layer_a.name.as_str(), "AnalogWurlyRoids");
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
    unpack_program_data(&message[6..6 + PROGRAM_PACKED_LEN], &mut raw);
    let decoded = decode::program_data(message).unwrap();

    assert_eq!(raw[28], 0, "factory fixture layer A Pan Spread changed");
    assert_eq!(raw[228], 49, "factory fixture layer B Pan Spread changed");
    assert_eq!(
        decoded.patch.layer_a.amplifier.pan_spread,
        f32::from(raw[28]) / 127.0
    );
    assert_eq!(
        decoded.patch.layer_a.amplifier.pan_mod_mode,
        PanModMode::Alternate
    );
}

#[test]
fn factory_bank_contains_a_glide_enabled_program() {
    let decoded = (0..256)
        .map(|program| decode::program_data(factory_message(program / 128, program % 128)).unwrap())
        .find(|program| {
            program.patch.layer_a.osc1.glide > 0.0 || program.patch.layer_a.osc2.glide > 0.0
        })
        .expect("factory bank should contain a glide program");
    assert!(decoded.patch.layer_a.glide_enabled);
}

#[test]
fn factory_bank_decodes_gated_sequence_payloads_for_both_layers() {
    for program in 0..256 {
        let message = factory_message(program / 128, program % 128);
        let mut raw = [0_u8; PROGRAM_DATA_LEN];
        unpack_program_data(&message[6..6 + PROGRAM_PACKED_LEN], &mut raw);
        let decoded = decode::program_data(message).unwrap();

        assert_sequence_payload(&decoded.patch.layer_a, &raw, 0);
        assert_sequence_payload(&decoded.patch.layer_b, &raw, 200);
    }
}

fn assert_sequence_payload(patch: &LayerPatch, raw: &[u8; PROGRAM_DATA_LEN], offset: usize) {
    assert_eq!(
        patch.sequence.sequencer_type,
        if raw[offset + 101] == 0 {
            SequencerType::Polyphonic
        } else {
            SequencerType::Gated
        }
    );
    assert_eq!(patch.bpm, f32::from(raw[offset + 91].clamp(30, 250)));
    assert_eq!(
        patch.clock_divide,
        ClockDivision::from_index(usize::from(raw[offset + 92].min(12)))
    );
    assert_eq!(
        patch.sequence.gated_mode,
        GatedSequencerMode::from_index(usize::from(raw[offset + 94].min(4)))
    );

    for track in 0..4 {
        if raw[offset + 77 + track] == 0 {
            assert_eq!(
                patch.sequence.gated.tracks[track].destination,
                GatedDestination::Off
            );
        } else {
            assert_ne!(
                patch.sequence.gated.tracks[track].destination,
                GatedDestination::Off
            );
        }
        for step in 0..16 {
            assert_eq!(
                patch.sequence.gated.tracks[track].steps[step],
                GatedStep::from_rev2_raw(u16::from(raw[offset + 120 + track * 16 + step]))
            );
        }
    }
}

fn factory_message(bank: usize, program: usize) -> &'static [u8] {
    let offset = (bank * 128 + program) * PROGRAM_DATA_SYSEX_LEN;
    &FACTORY_SYSEX[offset..offset + PROGRAM_DATA_SYSEX_LEN]
}
