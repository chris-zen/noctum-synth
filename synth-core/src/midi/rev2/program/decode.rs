//! Rev2 SysEx program dump decode.

use crate::{
    Patch,
    midi::{
        prophet::unpack_program_data,
        rev2::{
            layer::{LayerA, LayerB, LayerDecoder},
            program::{
                BANK_OFFSET, EDIT_BUFFER_COMMAND, EDIT_BUFFER_PAYLOAD_OFFSET, LAYER_MODE_OFFSET,
                MAX_BANK, PROGRAM_DATA_COMMAND, PROGRAM_DATA_LEN, PROGRAM_DATA_PAYLOAD_OFFSET,
                PROGRAM_DATA_SYSEX_LEN, PROGRAM_EDIT_BUFFER_SYSEX_LEN, PROGRAM_OFFSET,
                PROGRAM_PACKED_LEN, ProgramData, SPLIT_POINT_OFFSET, SYSEX_END, SYSEX_MANUFACTURER,
                SYSEX_MODEL, SYSEX_START, SysexError, layer_mode_from_raw,
            },
        },
    },
};

/// Decode a stored Prophet Rev2 Program Data dump.
pub fn program_data(message: &[u8]) -> Result<ProgramData, SysexError> {
    validate_header(message, PROGRAM_DATA_SYSEX_LEN, PROGRAM_DATA_COMMAND)?;
    let bank = message[BANK_OFFSET];
    let program = message[PROGRAM_OFFSET];
    if bank > MAX_BANK {
        return Err(SysexError::InvalidBank);
    }
    if program & 0x80 != 0 {
        return Err(SysexError::NonSevenBitData);
    }
    let patch = program_payload(
        &message[PROGRAM_DATA_PAYLOAD_OFFSET..PROGRAM_DATA_PAYLOAD_OFFSET + PROGRAM_PACKED_LEN],
    )?;
    Ok(ProgramData {
        bank,
        program,
        patch,
    })
}

/// Decode a Prophet Rev2 Program Edit Buffer data dump.
pub fn program_edit_buffer(message: &[u8]) -> Result<Patch, SysexError> {
    validate_header(message, PROGRAM_EDIT_BUFFER_SYSEX_LEN, EDIT_BUFFER_COMMAND)?;
    program_payload(
        &message[EDIT_BUFFER_PAYLOAD_OFFSET..EDIT_BUFFER_PAYLOAD_OFFSET + PROGRAM_PACKED_LEN],
    )
}

pub(in crate::midi::rev2) fn program_payload(packed: &[u8]) -> Result<Patch, SysexError> {
    if packed.iter().any(|byte| byte & 0x80 != 0) {
        return Err(SysexError::NonSevenBitData);
    }

    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    unpack_program_data(packed, &mut raw);
    let layer_a = LayerDecoder::<LayerA>::decode(&raw);
    let layer_b = LayerDecoder::<LayerB>::decode(&raw);
    let mode = layer_mode_from_raw(raw[LAYER_MODE_OFFSET]).ok_or(SysexError::InvalidProgramData)?;
    Ok(Patch::new(layer_a, layer_b, mode, raw[SPLIT_POINT_OFFSET]))
}

fn validate_header(
    message: &[u8],
    expected_len: usize,
    expected_command: u8,
) -> Result<(), SysexError> {
    if message.len() != expected_len {
        return Err(SysexError::InvalidLength);
    }
    if message[0] != SYSEX_START || message[expected_len - 1] != SYSEX_END {
        return Err(SysexError::InvalidFraming);
    }
    if message[1] != SYSEX_MANUFACTURER {
        return Err(SysexError::InvalidManufacturer);
    }
    if message[2] != SYSEX_MODEL {
        return Err(SysexError::InvalidModel);
    }
    if message[3] != expected_command {
        return Err(SysexError::UnsupportedCommand);
    }
    Ok(())
}
