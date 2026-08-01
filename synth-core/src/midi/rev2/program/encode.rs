//! Rev2 SysEx program dump encode.

use crate::{
    MAX_SPLIT_POINT, Patch,
    midi::{
        prophet::pack_program_data,
        rev2::{
            layer::{LayerA, LayerB, LayerDecoder},
            program::{
                LAYER_MODE_OFFSET, PROGRAM_DATA_LEN, PROGRAM_DATA_SYSEX_LEN,
                PROGRAM_EDIT_BUFFER_SYSEX_LEN, PROGRAM_PACKED_LEN, SPLIT_POINT_OFFSET, SysexError,
                layer_mode_raw,
            },
        },
    },
};

const SYSEX_HEADER: [u8; 4] = [0xf0, 0x01, 0x2f, 0x03];

/// Encode a synthesizer program as a stored Prophet Rev2 Program Data dump.
pub fn program_data(
    bank: u8,
    program: u8,
    patch: &Patch,
    output: &mut [u8],
) -> Result<usize, SysexError> {
    if bank > 7 {
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
    output[..6].copy_from_slice(&[0xf0, 0x01, 0x2f, 0x02, bank, program]);
    pack_program_data(&raw, &mut output[6..6 + PROGRAM_PACKED_LEN]);
    output[PROGRAM_DATA_SYSEX_LEN - 1] = 0xf7;
    Ok(PROGRAM_DATA_SYSEX_LEN)
}

/// Encode a synthesizer program as a Prophet Rev2 Program Edit Buffer data dump.
pub fn program_edit_buffer(patch: &Patch, output: &mut [u8]) -> Result<usize, SysexError> {
    if output.len() < PROGRAM_EDIT_BUFFER_SYSEX_LEN {
        return Err(SysexError::OutputTooSmall);
    }

    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    program_layers(patch, &mut raw);

    output[..4].copy_from_slice(&SYSEX_HEADER);
    pack_program_data(&raw, &mut output[4..4 + PROGRAM_PACKED_LEN]);
    output[PROGRAM_EDIT_BUFFER_SYSEX_LEN - 1] = 0xf7;
    Ok(PROGRAM_EDIT_BUFFER_SYSEX_LEN)
}

fn program_layers(program: &Patch, raw: &mut [u8; PROGRAM_DATA_LEN]) {
    LayerDecoder::<LayerA>::encode(&program.layer_a, raw);
    LayerDecoder::<LayerB>::encode(&program.layer_b, raw);
    raw[LAYER_MODE_OFFSET] = layer_mode_raw(program.mode);
    raw[SPLIT_POINT_OFFSET] = program.split_point.min(MAX_SPLIT_POINT);
}
