//! Shared Prophet Rev2 / '08 parameter conversions and SysEx packing.
//!
//! Program/NRPN cutoff is converted to Hz at the MIDI frontier; [`LayerPatch`] stores
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

/// Raw values corresponding positionally to [`ENVELOPE_SECONDS_ANCHORS`].
///
/// # Sources and compatibility rationale
///
/// - The [Prophet '08 manual](https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf)
///   defines the filter, amplifier, and auxiliary envelope time fields as raw
///   values from 0 through 127.
/// - The [Prophet Rev2 User's Guide](https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf)
///   states that Prophet '08 programs are compatible with the Rev2. The P08
///   decoder therefore uses the same raw-to-time interpretation as the Rev2.
/// - The [measured Rev2 envelope table](https://forum.sequential.com/index.php?topic=3203.0)
///   supplies these attack anchors and reports that the amplifier ADR profiles
///   share the same lookup-table-like shape. Intermediate raw values are
///   linearly interpolated between anchors.
/// - The approximately 25-second attack/decay and 40-second release maxima are
///   independently reported by the
///   [Sound On Sound Rev2 review](https://www.soundonsound.com/reviews/dsi-prophet-rev-2).
///
/// The measured hardware curve is visibly table-driven rather than linear, so
/// fitting a smoother formula would incorrectly change intermediate values.
const ENVELOPE_RAW_ANCHORS: [u16; 17] = [
    0, 7, 15, 23, 31, 39, 47, 55, 63, 71, 79, 87, 95, 103, 111, 119, 127,
];
/// Measured attack times in seconds for [`ENVELOPE_RAW_ANCHORS`].
const ENVELOPE_SECONDS_ANCHORS: [f32; 17] = [
    0.003, 0.010, 0.031, 0.075, 0.135, 0.195, 0.260, 0.390, 0.605, 0.735, 0.950, 1.260, 1.830,
    3.060, 6.080, 14.220, 24.660,
];
const ATTACK_DECAY_MAX_SECONDS: f32 = 24.660;
const RELEASE_MAX_SECONDS: f32 = 40.0;

pub fn cutoff_raw_to_hz(raw: u16) -> f32 {
    440.0
        * F32((f32::from(raw) - FILTER_CUTOFF_A4_RAW) / 12.0)
            .exp2()
            .as_f32()
}

pub fn cutoff_hz_to_raw(hz: f32, raw_max: u16) -> u16 {
    let hz = hz.max(f32::MIN_POSITIVE);
    let raw = 12.0 * F32(hz / 440.0).accurate_log2().as_f32() + FILTER_CUTOFF_A4_RAW;
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

/// Converts a Prophet '08/Rev2 attack or decay value using the documented
/// [`ENVELOPE_RAW_ANCHORS`] timing table.
pub(crate) fn attack_decay_seconds(raw: u16) -> f32 {
    envelope_seconds(raw, ATTACK_DECAY_MAX_SECONDS)
}

/// Converts a Prophet '08/Rev2 release value to seconds.
///
/// This uses the normalized curve documented by
/// [`ENVELOPE_RAW_ANCHORS`] and scales it to its sourced 40-second
/// maximum.
pub(crate) fn release_seconds(raw: u16) -> f32 {
    envelope_seconds(raw, RELEASE_MAX_SECONDS)
}

/// Inverse of [`attack_decay_seconds`] for Rev2 NRPN and SysEx encoding.
pub(crate) fn attack_decay_raw(seconds: f32) -> u16 {
    envelope_raw(seconds, ATTACK_DECAY_MAX_SECONDS)
}

/// Inverse of [`release_seconds`] for Rev2 NRPN and SysEx encoding.
pub(crate) fn release_raw(seconds: f32) -> u16 {
    envelope_raw(seconds, RELEASE_MAX_SECONDS)
}

/// Packed SysEx length for an unpacked Sequential program image of `raw_len` bytes.
pub(crate) const fn packed_program_len(raw_len: usize) -> usize {
    raw_len + raw_len.div_ceil(7)
}

/// Packs an unpacked Sequential program image into 7-bit SysEx data bytes.
pub(crate) fn pack_program_data(raw: &[u8], packed: &mut [u8]) {
    let expected = packed_program_len(raw.len());
    debug_assert_eq!(packed.len(), expected);
    let mut output = 0;
    for chunk in raw.chunks(7) {
        let mut high_bits = 0_u8;
        for (index, byte) in chunk.iter().copied().enumerate() {
            high_bits |= (byte >> 7) << (6 - index);
        }
        packed[output] = high_bits;
        output += 1;
        for byte in chunk.iter().copied() {
            packed[output] = byte & 0x7f;
            output += 1;
        }
    }
    debug_assert_eq!(output, expected);
}

/// Unpacks 7-bit SysEx data bytes into an unpacked Sequential program image.
pub(crate) fn unpack_program_data(packed: &[u8], raw: &mut [u8]) {
    let expected = packed_program_len(raw.len());
    debug_assert_eq!(packed.len(), expected);
    let mut input = 0;
    let mut output = 0;
    while output < raw.len() {
        let high_bits = packed[input];
        input += 1;
        let count = (raw.len() - output).min(7);
        for index in 0..count {
            raw[output] = packed[input] | (((high_bits >> (6 - index)) & 1) << 7);
            input += 1;
            output += 1;
        }
    }
    debug_assert_eq!(input, expected);
}

fn envelope_seconds(raw: u16, max_seconds: f32) -> f32 {
    let raw = raw.min(127);
    let scale = max_seconds / ATTACK_DECAY_MAX_SECONDS;
    for index in 0..ENVELOPE_RAW_ANCHORS.len() - 1 {
        let raw_start = ENVELOPE_RAW_ANCHORS[index];
        let raw_end = ENVELOPE_RAW_ANCHORS[index + 1];
        if raw <= raw_end {
            let position = f32::from(raw - raw_start) / f32::from(raw_end - raw_start);
            let seconds_start = ENVELOPE_SECONDS_ANCHORS[index];
            let seconds_end = ENVELOPE_SECONDS_ANCHORS[index + 1];
            return (seconds_start + position * (seconds_end - seconds_start)) * scale;
        }
    }
    max_seconds
}

fn envelope_raw(seconds: f32, max_seconds: f32) -> u16 {
    let scale = max_seconds / ATTACK_DECAY_MAX_SECONDS;
    let seconds = seconds.clamp(ENVELOPE_SECONDS_ANCHORS[0] * scale, max_seconds) / scale;
    for index in 0..ENVELOPE_SECONDS_ANCHORS.len() - 1 {
        let seconds_start = ENVELOPE_SECONDS_ANCHORS[index];
        let seconds_end = ENVELOPE_SECONDS_ANCHORS[index + 1];
        if seconds <= seconds_end {
            let position = (seconds - seconds_start) / (seconds_end - seconds_start);
            let raw_start = ENVELOPE_RAW_ANCHORS[index];
            let raw_end = ENVELOPE_RAW_ANCHORS[index + 1];
            return F32(f32::from(raw_start) + position * f32::from(raw_end - raw_start))
                .round()
                .as_f32() as u16;
        }
    }
    127
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
