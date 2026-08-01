//! Imported program dumps from external MIDI sources.

use crate::{
    Patch,
    midi::{p08, rev2},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiProgramSource {
    Rev2,
    P08,
}

#[derive(Debug, Clone)]
pub enum ProgramData {
    Rev2(rev2::ProgramData),
    P08(p08::ProgramData),
}

impl ProgramData {
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
