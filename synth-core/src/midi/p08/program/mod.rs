//! Shared Prophet '08 SysEx program image constants and types.

use crate::{LayerMode, Patch, midi::prophet::packed_program_len};

pub mod decode;

pub const SYSEX_START: u8 = 0xf0;
pub const SYSEX_END: u8 = 0xf7;
pub const SYSEX_MANUFACTURER: u8 = 0x01;
pub const SYSEX_MODEL: u8 = 0x23;
pub const PROGRAM_DATA_COMMAND: u8 = 0x02;
pub const EDIT_BUFFER_COMMAND: u8 = 0x03;
pub const BANK_OFFSET: usize = 4;
pub const PROGRAM_OFFSET: usize = 5;
pub const PROGRAM_DATA_PAYLOAD_OFFSET: usize = 6;
pub const EDIT_BUFFER_PAYLOAD_OFFSET: usize = 4;
pub const MAX_BANK: u8 = 1;

pub const PROGRAM_DATA_LEN: usize = 384;
pub const PROGRAM_PACKED_LEN: usize = packed_program_len(PROGRAM_DATA_LEN);
pub const PROGRAM_DATA_SYSEX_LEN: usize = PROGRAM_DATA_PAYLOAD_OFFSET + PROGRAM_PACKED_LEN + 1;
pub const PROGRAM_EDIT_BUFFER_SYSEX_LEN: usize =
    EDIT_BUFFER_PAYLOAD_OFFSET + PROGRAM_PACKED_LEN + 1;

/// Program-level split point in the official
/// [Prophet '08 program image](https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf).
pub(super) const SPLIT_POINT_OFFSET: usize = 118;
/// Program-level keyboard mode in the official
/// [Prophet '08 program image](https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf).
pub(super) const LAYER_MODE_OFFSET: usize = 119;

#[derive(Debug, Clone)]
pub struct ProgramData {
    pub bank: u8,
    pub program: u8,
    pub patch: Patch,
}

pub(super) fn layer_mode_from_raw(raw: u8) -> Option<LayerMode> {
    match raw {
        0 => Some(LayerMode::Normal),
        1 => Some(LayerMode::Stack),
        2 => Some(LayerMode::Split),
        _ => None,
    }
}
