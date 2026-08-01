//! Prophet '08 SysEx program dump decode.

use crate::{
    Patch,
    midi::{
        p08::layer::{Layer, LayerA, LayerB, LayerDecoder},
        prophet::unpack_program_data,
        rev2::SysexError,
    },
    patch::decode_patch_name,
};

use super::{
    LAYER_MODE_OFFSET, PROGRAM_DATA_LEN, PROGRAM_DATA_SYSEX_LEN, PROGRAM_EDIT_BUFFER_SYSEX_LEN,
    PROGRAM_PACKED_LEN, ProgramData, SPLIT_POINT_OFFSET, layer_mode_from_raw,
};

const SYSEX_MANUFACTURER: u8 = 0x01;
const SYSEX_MODEL: u8 = 0x23;

/// Decode a stored Prophet '08 Program Data dump.
pub fn program_data(message: &[u8]) -> Result<ProgramData, SysexError> {
    validate_header(message, PROGRAM_DATA_SYSEX_LEN, 0x02)?;
    let bank = message[4];
    let program = message[5];
    if bank > 1 {
        return Err(SysexError::InvalidBank);
    }
    if program & 0x80 != 0 {
        return Err(SysexError::NonSevenBitData);
    }
    let patch = program_payload(&message[6..6 + PROGRAM_PACKED_LEN])?;
    Ok(ProgramData {
        bank,
        program,
        patch,
    })
}

/// Decode a Prophet '08 Program Edit Buffer data dump.
pub fn program_edit_buffer(message: &[u8]) -> Result<Patch, SysexError> {
    validate_header(message, PROGRAM_EDIT_BUFFER_SYSEX_LEN, 0x03)?;
    program_payload(&message[4..4 + PROGRAM_PACKED_LEN])
}

fn program_payload(packed: &[u8]) -> Result<Patch, SysexError> {
    if packed.iter().any(|byte| byte & 0x80 != 0) {
        return Err(SysexError::NonSevenBitData);
    }

    let mut raw = [0_u8; PROGRAM_DATA_LEN];
    unpack_program_data(packed, &mut raw);
    let mut layer_a = LayerDecoder::<LayerA>::decode(&raw);
    let mut layer_b = LayerDecoder::<LayerB>::decode(&raw);
    let program_name = decode_patch_name(&raw[LayerA::NAME_RANGE]);
    layer_a.name = program_name.clone();
    layer_b.name = program_name;
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
    if message[0] != 0xf0 || message[expected_len - 1] != 0xf7 {
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
