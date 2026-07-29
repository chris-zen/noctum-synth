//! Sequential Prophet Rev2-compatible CC and NRPN parameter codec.
//!
//! Envelope attack, decay, and release use the lookup-table-shaped timing curve
//! from the
//! [measured Rev2 envelope table](https://forum.sequential.com/index.php?topic=3203.0),
//! not a linear interpolation over seconds. The approximately 25-second
//! attack/decay and 40-second release maxima are independently reported in the
//! [Sound On Sound Rev2 review](https://www.soundonsound.com/reviews/dsi-prophet-rev-2).

mod controller;
mod encoder;
mod layer;
mod map;
pub mod program;

#[cfg(test)]
mod tests;

pub use controller::{ControllerDecoder, MidiUpdate};
pub use encoder::ControllerEncoder;
pub use program::{PROGRAM_DATA_SYSEX_LEN, PROGRAM_EDIT_BUFFER_SYSEX_LEN, ProgramData, SysexError};
pub use program::{decode, encode};
