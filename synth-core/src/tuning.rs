//! Note tuning helpers.

/// Converts a MIDI note number to frequency in Hz (A4 = 69 → 440 Hz).
#[inline]
pub fn midi_to_hz(note: u8) -> f32 {
    440.0 * crate::math::exp2((note as f32 - 69.0) / 12.0)
}
