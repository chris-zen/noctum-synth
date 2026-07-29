//! Official Rev2 factory programs round-tripped through layer-patch storage.

use synth_core::{
    LayerPatch,
    midi::rev2::{PROGRAM_DATA_SYSEX_LEN, decode},
    patch_storage::{LAYER_PATCH_RECORD_SIZE, LayerPatchRecord},
};

const FACTORY_BANK: &[u8] =
    include_bytes!("../../../Prophet-Rev2-Factory-Programs/Rev2_Programs_v1.0.syx");

#[test]
fn all_factory_rev2_programs_round_trip_through_record() {
    assert_eq!(FACTORY_BANK.len() % PROGRAM_DATA_SYSEX_LEN, 0);
    for message in FACTORY_BANK.chunks_exact(PROGRAM_DATA_SYSEX_LEN) {
        let imported = decode::program_data(message).unwrap();
        let mut record = [0; LAYER_PATCH_RECORD_SIZE];
        LayerPatchRecord::encode(&imported.patch.layer_a, &mut record).unwrap_or_else(|error| {
            panic!(
                "encode bank={} program={}: {error:?}",
                imported.bank, imported.program
            )
        });
        let decoded = LayerPatchRecord::decode(&record).unwrap_or_else(|error| {
            panic!(
                "decode bank={} program={}: {error:?}",
                imported.bank, imported.program
            )
        });
        assert_patch_close(&imported.patch.layer_a, &decoded);
    }
}

#[test]
fn factory_rev2_program_round_trips_through_record() {
    let imported = decode::program_data(&FACTORY_BANK[..PROGRAM_DATA_SYSEX_LEN])
        .unwrap()
        .patch
        .layer_a;
    let mut record = [0; LAYER_PATCH_RECORD_SIZE];
    LayerPatchRecord::encode(&imported, &mut record).unwrap();
    assert_patch_close(&imported, &LayerPatchRecord::decode(&record).unwrap());
}

fn assert_patch_close(expected: &LayerPatch, actual: &LayerPatch) {
    let mut actual_values = Vec::new();
    actual.for_each_param(|_, value| actual_values.push(value));
    let mut index = 0;
    expected.for_each_param(|_, value| {
        let tolerance = value.abs().max(1.0) * 0.001;
        assert!(
            (value - actual_values[index]).abs() <= tolerance,
            "{value} != {}",
            actual_values[index]
        );
        index += 1;
    });
    assert_eq!(expected.name, actual.name);
    assert_eq!(expected.unison_chord, actual.unison_chord);
    for (expected_slot, actual_slot) in expected
        .mod_matrix
        .free_slots
        .iter()
        .zip(actual.mod_matrix.free_slots.iter())
    {
        assert_eq!(expected_slot.enabled, actual_slot.enabled);
        assert_eq!(expected_slot.source, actual_slot.source);
        assert_eq!(expected_slot.destination, actual_slot.destination);
        let tolerance = expected_slot.amount.abs().max(1.0) * 0.001;
        assert!((expected_slot.amount - actual_slot.amount).abs() <= tolerance);
    }
    for (expected_slot, actual_slot) in expected
        .mod_matrix
        .dedicated
        .iter()
        .zip(actual.mod_matrix.dedicated.iter())
    {
        assert_eq!(expected_slot.enabled, actual_slot.enabled);
        assert_eq!(expected_slot.destination, actual_slot.destination);
        let tolerance = expected_slot.amount.abs().max(1.0) * 0.001;
        assert!((expected_slot.amount - actual_slot.amount).abs() <= tolerance);
    }
}
