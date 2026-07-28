//! Prophet Rev2 / '08 filter cutoff and key-track scaling.
//!
//! Program/NRPN cutoff is converted to Hz at the MIDI frontier; [`Patch`] stores
//! Hz only. Official docs: 0–164 in semitone steps over more than 13 octaves
//! (Prophet '08 / Prophet 12). That span is ~1 Hz–13.3 kHz with A4 at raw 105
//! (community self-oscillation calibration). DSI oscillator freq likewise starts
//! below 20 Hz (8 Hz). The DSP processing floor of 20 Hz applies to the
//! modulated cutoff, not to this program decode.

use crate::math::F32;

pub const FILTER_CUTOFF_RAW_MAX: u16 = 164;
pub const FILTER_CUTOFF_A4_RAW: f32 = 105.0;
pub const FILTER_KEY_TRACK_UNITY_RAW: u16 = 64;
pub const FILTER_KEY_TRACK_RAW_MAX: u16 = 127;
pub const FILTER_KEY_TRACK_MAX: f32 =
    FILTER_KEY_TRACK_RAW_MAX as f32 / FILTER_KEY_TRACK_UNITY_RAW as f32;

pub fn cutoff_raw_to_hz(raw: u16) -> f32 {
    440.0
        * F32((f32::from(raw) - FILTER_CUTOFF_A4_RAW) / 12.0)
            .exp2()
            .as_f32()
}

pub fn cutoff_hz_to_raw(hz: f32, raw_max: u16) -> u16 {
    let hz = hz.max(f32::MIN_POSITIVE);
    let raw = F32(hz / 440.0).ln().as_f32() / core::f32::consts::LN_2 * 12.0 + FILTER_CUTOFF_A4_RAW;
    F32(raw.clamp(0.0, f32::from(raw_max))).round().as_f32() as u16
}

pub fn filter_cutoff_max_hz() -> f32 {
    cutoff_raw_to_hz(FILTER_CUTOFF_RAW_MAX)
}

pub fn key_track_from_raw(raw: u16) -> f32 {
    f32::from(raw.min(FILTER_KEY_TRACK_RAW_MAX)) / f32::from(FILTER_KEY_TRACK_UNITY_RAW)
}

pub fn key_track_to_raw(value: f32) -> u16 {
    F32(value.clamp(0.0, FILTER_KEY_TRACK_MAX) * f32::from(FILTER_KEY_TRACK_UNITY_RAW))
        .round()
        .as_f32()
        .clamp(0.0, f32::from(FILTER_KEY_TRACK_RAW_MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoff_raw_maps_known_semitone_anchors() {
        assert!((cutoff_raw_to_hz(0) - 1.021_975).abs() < 0.01);
        assert!((cutoff_raw_to_hz(96) - 261.625_55).abs() < 0.05);
        assert!((cutoff_raw_to_hz(105) - 440.0).abs() < 0.01);
        assert!((cutoff_raw_to_hz(164) - 13_289.75).abs() < 1.0);
    }

    #[test]
    fn cutoff_hz_round_trips_through_raw() {
        for raw in [0_u16, 24, 96, 105, 127, 140, 164] {
            let hz = cutoff_raw_to_hz(raw);
            assert_eq!(cutoff_hz_to_raw(hz, FILTER_CUTOFF_RAW_MAX), raw);
        }
    }

    #[test]
    fn cc_max_matches_nrpn_127_not_full_open() {
        let cc_max = cutoff_raw_to_hz(127);
        let nrpn_127 = cutoff_raw_to_hz(127);
        let nrpn_max = cutoff_raw_to_hz(FILTER_CUTOFF_RAW_MAX);
        assert!((cc_max - nrpn_127).abs() < f32::EPSILON);
        assert!(cc_max < nrpn_max * 0.2);
    }

    #[test]
    fn key_track_64_is_unity() {
        assert!((key_track_from_raw(64) - 1.0).abs() < f32::EPSILON);
        assert!((key_track_from_raw(32) - 0.5).abs() < f32::EPSILON);
        assert!((key_track_from_raw(127) - FILTER_KEY_TRACK_MAX).abs() < 0.001);
        assert_eq!(key_track_to_raw(1.0), 64);
        assert_eq!(key_track_to_raw(0.5), 32);
    }
}
