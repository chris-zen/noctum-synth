//! Sequential Prophet '08-compatible program SysEx decoder.
//!
//! Envelope attack, decay, and release fields are stored as raw `0..=127`
//! values, as documented by the
//! [Prophet '08 manual](https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf).
//! They are translated through the shared nonlinear Prophet timing curve rather
//! than treated as linear seconds. Sequential's
//! [Prophet Rev2 User's Guide](https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf)
//! states that Prophet '08 programs are compatible with the Rev2; consequently,
//! this decoder uses the same raw-value interpretation as the Rev2 codec. The
//! timing anchors come from the
//! [measured Rev2 envelope table](https://forum.sequential.com/index.php?topic=3203.0),
//! with the release curve scaled to the approximately 40-second maximum reported
//! by the [Sound On Sound Rev2 review](https://www.soundonsound.com/reviews/dsi-prophet-rev-2).

mod layer;
mod map;
pub mod program;

#[cfg(test)]
mod tests;

pub use program::{PROGRAM_DATA_SYSEX_LEN, PROGRAM_EDIT_BUFFER_SYSEX_LEN, ProgramData, decode};
