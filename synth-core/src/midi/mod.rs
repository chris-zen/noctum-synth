//! MIDI clock, program import, and instrument SysEx codecs.

pub mod clock;
pub mod p08;
pub(crate) mod program;
pub mod rev2;
pub(crate) mod scale;

pub use clock::{MidiClockMode, MidiClockStatus, MidiRealtimeEvent, MidiTransportState};
pub use p08::{
    P08_PROGRAM_DATA_LEN, P08_PROGRAM_DATA_SYSEX_LEN, P08_PROGRAM_EDIT_BUFFER_SYSEX_LEN,
    P08_PROGRAM_PACKED_LEN, P08MidiDecoder, P08ProgramData,
};
pub use program::{MidiProgramImport, MidiProgramSource};
pub use rev2::{
    REV2_PROGRAM_DATA_LEN, REV2_PROGRAM_DATA_SYSEX_LEN, REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN,
    REV2_PROGRAM_PACKED_LEN, Rev2MidiDecoder, Rev2MidiEncoder, Rev2MidiUpdate, Rev2ProgramData,
    Rev2SysexError,
};
pub use scale::{
    FILTER_CUTOFF_RAW_MAX, FILTER_KEY_TRACK_MAX, FILTER_KEY_TRACK_UNITY_RAW, cutoff_hz_to_raw,
    cutoff_raw_to_hz, filter_cutoff_max_hz, key_track_from_raw, key_track_to_raw,
};
