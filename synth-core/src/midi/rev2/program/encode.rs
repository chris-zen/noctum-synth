//! Rev2 SysEx program dump encode.

use crate::{
    MAX_SPLIT_POINT, Patch,
    midi::{
        prophet::pack_program_data,
        rev2::{
            layer::{LayerA, LayerB, LayerDecoder},
            program::{
                EDIT_BUFFER_COMMAND, EDIT_BUFFER_PAYLOAD_OFFSET, LAYER_MODE_OFFSET, MAX_BANK,
                PROGRAM_DATA_COMMAND, PROGRAM_DATA_LEN, PROGRAM_DATA_PAYLOAD_OFFSET,
                PROGRAM_DATA_SYSEX_LEN, PROGRAM_EDIT_BUFFER_SYSEX_LEN, PROGRAM_PACKED_LEN,
                SPLIT_POINT_OFFSET, SYSEX_END, SYSEX_MANUFACTURER, SYSEX_MODEL, SYSEX_START,
                SysexError, layer_mode_raw,
            },
        },
    },
};

const EDIT_BUFFER_SYSEX_HEADER: [u8; EDIT_BUFFER_PAYLOAD_OFFSET] = [
    SYSEX_START,
    SYSEX_MANUFACTURER,
    SYSEX_MODEL,
    EDIT_BUFFER_COMMAND,
];

/// Encode a synthesizer program as a stored Prophet Rev2 Program Data dump.
pub fn program_data(
    bank: u8,
    program: u8,
    patch: &Patch,
    output: &mut [u8],
) -> Result<usize, SysexError> {
    if bank > MAX_BANK {
        return Err(SysexError::InvalidBank);
    }
    if program & 0x80 != 0 {
        return Err(SysexError::NonSevenBitData);
    }
    if output.len() < PROGRAM_DATA_SYSEX_LEN {
        return Err(SysexError::OutputTooSmall);
    }

    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    program_layers(patch, &mut raw);
    output[..PROGRAM_DATA_PAYLOAD_OFFSET].copy_from_slice(&[
        SYSEX_START,
        SYSEX_MANUFACTURER,
        SYSEX_MODEL,
        PROGRAM_DATA_COMMAND,
        bank,
        program,
    ]);
    pack_program_data(
        &raw,
        &mut output[PROGRAM_DATA_PAYLOAD_OFFSET..PROGRAM_DATA_PAYLOAD_OFFSET + PROGRAM_PACKED_LEN],
    );
    output[PROGRAM_DATA_SYSEX_LEN - 1] = SYSEX_END;
    Ok(PROGRAM_DATA_SYSEX_LEN)
}

/// Encode a synthesizer program as a Prophet Rev2 Program Edit Buffer data dump.
pub fn program_edit_buffer(patch: &Patch, output: &mut [u8]) -> Result<usize, SysexError> {
    if output.len() < PROGRAM_EDIT_BUFFER_SYSEX_LEN {
        return Err(SysexError::OutputTooSmall);
    }

    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    program_layers(patch, &mut raw);

    output[..EDIT_BUFFER_PAYLOAD_OFFSET].copy_from_slice(&EDIT_BUFFER_SYSEX_HEADER);
    pack_program_data(
        &raw,
        &mut output[EDIT_BUFFER_PAYLOAD_OFFSET..EDIT_BUFFER_PAYLOAD_OFFSET + PROGRAM_PACKED_LEN],
    );
    output[PROGRAM_EDIT_BUFFER_SYSEX_LEN - 1] = SYSEX_END;
    Ok(PROGRAM_EDIT_BUFFER_SYSEX_LEN)
}

fn program_layers(program: &Patch, raw: &mut [u8; PROGRAM_DATA_LEN]) {
    LayerDecoder::<LayerA>::encode(&program.layer_a, raw);
    LayerDecoder::<LayerB>::encode(&program.layer_b, raw);
    raw[LAYER_MODE_OFFSET] = layer_mode_raw(program.mode);
    raw[SPLIT_POINT_OFFSET] = program.split_point.min(MAX_SPLIT_POINT);
}
