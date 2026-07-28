//! Imported program dumps from external MIDI sources.

use crate::Patch;
use crate::midi::p08::ProgramData as P08ProgramData;
use crate::midi::rev2::ProgramData as Rev2ProgramData;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiProgramSource {
    Rev2,
    P08,
}

#[derive(Debug, Clone)]
pub enum MidiProgramImport {
    Rev2(Rev2ProgramData),
    P08(P08ProgramData),
}

impl MidiProgramImport {
    pub fn source(&self) -> MidiProgramSource {
        match self {
            Self::Rev2(_) => MidiProgramSource::Rev2,
            Self::P08(_) => MidiProgramSource::P08,
        }
    }

    pub fn bank(&self) -> u8 {
        match self {
            Self::Rev2(program) => program.bank,
            Self::P08(program) => program.bank,
        }
    }

    pub fn program(&self) -> u8 {
        match self {
            Self::Rev2(program) => program.program,
            Self::P08(program) => program.program,
        }
    }

    pub fn patch(&self) -> &Patch {
        match self {
            Self::Rev2(program) => &program.patch,
            Self::P08(program) => &program.patch,
        }
    }
}
