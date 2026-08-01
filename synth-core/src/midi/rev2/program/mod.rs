//! Shared Rev2 SysEx program image constants and types.

use crate::{LayerMode, Patch, midi::prophet::packed_program_len};

pub mod decode;
pub mod encode;

pub const SYSEX_START: u8 = 0xf0;
pub const SYSEX_END: u8 = 0xf7;
pub const SYSEX_MANUFACTURER: u8 = 0x01;
pub const SYSEX_MODEL: u8 = 0x2f;
pub const PROGRAM_DATA_COMMAND: u8 = 0x02;
pub const EDIT_BUFFER_COMMAND: u8 = 0x03;
pub const BANK_OFFSET: usize = 4;
pub const PROGRAM_OFFSET: usize = 5;
pub const PROGRAM_DATA_PAYLOAD_OFFSET: usize = 6;
pub const EDIT_BUFFER_PAYLOAD_OFFSET: usize = 4;
pub const MAX_BANK: u8 = 7;

/// Raw program-mode values follow the official [Prophet '08 manual], [Edisyn],
/// a working [Electra One implementation], and Sequential's official Rev2 factory bank.
///
/// [Prophet '08 manual]: https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf
/// [Edisyn]: https://github.com/eclab/edisyn
/// [Electra One implementation]: https://forum.electra.one/t/dsi-sequential-prophet-rev-2/130
const LAYER_MODES_BY_RAW_VALUE: [LayerMode; 3] =
    [LayerMode::Normal, LayerMode::Stack, LayerMode::Split];

pub const PROGRAM_DATA_LEN: usize = 2046;
pub const PROGRAM_PACKED_LEN: usize = packed_program_len(PROGRAM_DATA_LEN);
pub const PROGRAM_DATA_SYSEX_LEN: usize = PROGRAM_DATA_PAYLOAD_OFFSET + PROGRAM_PACKED_LEN + 1;
pub const PROGRAM_EDIT_BUFFER_SYSEX_LEN: usize =
    EDIT_BUFFER_PAYLOAD_OFFSET + PROGRAM_PACKED_LEN + 1;

/// Program-mode byte offset, verified against Sequential's official [Rev2 factory bank].
///
/// [Rev2 factory bank]: https://sequential.com/support/download/prophet-rev2-sounds/
pub(super) const LAYER_MODE_OFFSET: usize = 231;

/// Split-point byte offset for the range documented in the official [Rev2 User's Guide].
///
/// [Rev2 User's Guide]: https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf
pub(super) const SPLIT_POINT_OFFSET: usize = 232;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysexError {
    InvalidLength,
    InvalidFraming,
    InvalidManufacturer,
    InvalidModel,
    UnsupportedCommand,
    InvalidBank,
    NonSevenBitData,
    InvalidProgramData,
    OutputTooSmall,
}

#[derive(Debug, Clone)]
pub struct ProgramData {
    pub bank: u8,
    pub program: u8,
    pub patch: Patch,
}

pub(super) fn layer_mode_from_raw(raw: u8) -> Option<LayerMode> {
    LAYER_MODES_BY_RAW_VALUE.get(usize::from(raw)).copied()
}

pub(super) fn layer_mode_raw(mode: LayerMode) -> u8 {
    LAYER_MODES_BY_RAW_VALUE
        .iter()
        .position(|candidate| *candidate == mode)
        .unwrap_or(0) as u8
}
