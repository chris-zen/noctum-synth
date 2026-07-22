//! MIDI clock, program import, and instrument SysEx codecs.

pub mod clock;
pub mod p08;
pub(crate) mod program;
pub mod rev2;

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
