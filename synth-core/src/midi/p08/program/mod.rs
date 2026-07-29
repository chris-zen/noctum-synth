//! Shared Prophet '08 SysEx program image constants and types.

use crate::LayerMode;
use crate::Patch;
use crate::midi::prophet::packed_program_len;

pub mod decode;

pub const PROGRAM_DATA_LEN: usize = 384;
pub const PROGRAM_PACKED_LEN: usize = packed_program_len(PROGRAM_DATA_LEN);
pub const PROGRAM_DATA_SYSEX_LEN: usize = 446;
pub const PROGRAM_EDIT_BUFFER_SYSEX_LEN: usize = 444;

/// Program-level split point in the official Prophet '08 program image.
pub(super) const SPLIT_POINT_OFFSET: usize = 118;
/// Program-level keyboard mode in the official Prophet '08 program image.
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
