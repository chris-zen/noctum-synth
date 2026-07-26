//! Note tuning helpers.

use crate::math::F32;

/// Converts a MIDI note number to frequency in Hz (A4 = 69 → 440 Hz).
#[inline]
pub fn midi_to_hz(note: u8) -> f32 {
    440.0 * F32((note as f32 - 69.0) / 12.0).exp2().as_f32()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_to_hz_maps_standard_notes() {
        assert!((midi_to_hz(69) - 440.0).abs() < 0.01);
        assert!((midi_to_hz(60) - 261.6256).abs() < 0.01);
        assert!((midi_to_hz(81) - 880.0).abs() < 0.01);
    }
}
