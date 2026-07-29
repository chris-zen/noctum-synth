//! Compact, versioned patch records for non-volatile program storage.

use crate::patch::{
    CHORD_MEMORY_CAPACITY, DedicatedModSource, MOD_MATRIX_FREE_SLOT_COUNT, ModDestination,
    ModSource, PATCH_NAME_CAPACITY,
};
use crate::{LayerPatch, ParamId};

pub const LAYER_PATCH_RECORD_SIZE: usize = 512;

const MAGIC: [u8; 4] = *b"ASPR";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 12;
const PARAM_COUNT: usize = PARAM_IDS.len();
const PAYLOAD_LEN: usize = PARAM_COUNT * 2
    + MOD_MATRIX_FREE_SLOT_COUNT * 5
    + DedicatedModSource::COUNT * 4
    + 1
    + PATCH_NAME_CAPACITY
    + 1
    + CHORD_MEMORY_CAPACITY;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerPatchRecordError {
    InvalidMagic,
    UnsupportedVersion,
    InvalidLength,
    ChecksumMismatch,
    InvalidPayload,
    NonFiniteValue,
    ValueOutOfRange,
    CodecDrift,
}

pub struct LayerPatchRecord;

impl LayerPatchRecord {
    /// Encode one patch into a complete flash slot. Unused bytes remain erased.
    pub fn encode(
        patch: &LayerPatch,
        output: &mut [u8; LAYER_PATCH_RECORD_SIZE],
    ) -> Result<(), LayerPatchRecordError> {
        output.fill(0xff);
        output[..4].copy_from_slice(&MAGIC);
        output[4] = VERSION;
        output[5] = PARAM_COUNT as u8;
        output[6..8].copy_from_slice(&(PAYLOAD_LEN as u16).to_le_bytes());

        let payload = &mut output[HEADER_LEN..HEADER_LEN + PAYLOAD_LEN];
        let mut cursor = Writer::new(payload);
        let mut index = 0;
        let mut error = None;
        patch.for_each_param(|id, value| {
            if error.is_some() {
                return;
            }
            if PARAM_IDS.get(index) != Some(&id) {
                error = Some(LayerPatchRecordError::CodecDrift);
                return;
            }
            match encode_value(value) {
                Ok(value) => cursor.u16(value),
                Err(value_error) => {
                    error = Some(value_error);
                    return;
                }
            }
            index += 1;
        });
        if let Some(error) = error {
            return Err(error);
        }
        if index != PARAM_COUNT {
            return Err(LayerPatchRecordError::CodecDrift);
        }

        for slot in patch.mod_matrix.free_slots {
            cursor.u8(u8::from(slot.enabled));
            cursor.u8(slot.source.index() as u8);
            cursor.u8(slot.destination.index() as u8);
            cursor.u16(encode_value(slot.amount)?);
        }
        for slot in patch.mod_matrix.dedicated {
            cursor.u8(u8::from(slot.enabled));
            cursor.u8(slot.destination.index() as u8);
            cursor.u16(encode_value(slot.amount)?);
        }

        let name = patch.name.as_bytes();
        cursor.u8(name.len() as u8);
        cursor.bytes(name);
        cursor.zeroes(PATCH_NAME_CAPACITY - name.len());
        let chord = patch.unison_chord.intervals();
        cursor.u8(chord.len() as u8);
        cursor.bytes(chord);
        cursor.zeroes(CHORD_MEMORY_CAPACITY - chord.len());
        if cursor.position() != PAYLOAD_LEN {
            return Err(LayerPatchRecordError::CodecDrift);
        }

        let checksum = crc32(payload);
        output[8..12].copy_from_slice(&checksum.to_le_bytes());
        Ok(())
    }

    /// Decode one flash slot. A completely erased slot is the default patch.
    pub fn decode(
        input: &[u8; LAYER_PATCH_RECORD_SIZE],
    ) -> Result<LayerPatch, LayerPatchRecordError> {
        if Self::is_erased(input) {
            return Ok(LayerPatch::default());
        }
        if input[..4] != MAGIC {
            return Err(LayerPatchRecordError::InvalidMagic);
        }
        if input[4] != VERSION {
            return Err(LayerPatchRecordError::UnsupportedVersion);
        }
        if usize::from(input[5]) != PARAM_COUNT {
            return Err(LayerPatchRecordError::CodecDrift);
        }
        let payload_len = usize::from(u16::from_le_bytes([input[6], input[7]]));
        if payload_len != PAYLOAD_LEN || HEADER_LEN + payload_len > LAYER_PATCH_RECORD_SIZE {
            return Err(LayerPatchRecordError::InvalidLength);
        }
        let payload = &input[HEADER_LEN..HEADER_LEN + payload_len];
        let expected = u32::from_le_bytes([input[8], input[9], input[10], input[11]]);
        if crc32(payload) != expected {
            return Err(LayerPatchRecordError::ChecksumMismatch);
        }

        let mut cursor = Reader::new(payload);
        let mut patch = LayerPatch::default();
        for id in PARAM_IDS {
            let value = f16_to_f32(cursor.u16()?);
            if !value.is_finite() {
                return Err(LayerPatchRecordError::NonFiniteValue);
            }
            patch.set_param(id, value);
        }
        for slot in &mut patch.mod_matrix.free_slots {
            slot.enabled = cursor.u8()? != 0;
            let source = usize::from(cursor.u8()?);
            let destination = usize::from(cursor.u8()?);
            if source >= ModSource::ALL.len() || destination >= ModDestination::COUNT {
                return Err(LayerPatchRecordError::InvalidPayload);
            }
            slot.source = ModSource::from_index(source);
            slot.destination = ModDestination::from_index(destination);
            slot.amount = f16_to_f32(cursor.u16()?);
            if !slot.amount.is_finite() {
                return Err(LayerPatchRecordError::NonFiniteValue);
            }
        }
        for slot in &mut patch.mod_matrix.dedicated {
            slot.enabled = cursor.u8()? != 0;
            let destination = usize::from(cursor.u8()?);
            if destination >= ModDestination::COUNT {
                return Err(LayerPatchRecordError::InvalidPayload);
            }
            slot.destination = ModDestination::from_index(destination);
            slot.amount = f16_to_f32(cursor.u16()?);
            if !slot.amount.is_finite() {
                return Err(LayerPatchRecordError::NonFiniteValue);
            }
        }

        let name_len = usize::from(cursor.u8()?);
        if name_len > PATCH_NAME_CAPACITY {
            return Err(LayerPatchRecordError::InvalidPayload);
        }
        let name_bytes = cursor.bytes(PATCH_NAME_CAPACITY)?;
        let name = core::str::from_utf8(&name_bytes[..name_len])
            .map_err(|_| LayerPatchRecordError::InvalidPayload)?;
        patch
            .name
            .push_str(name)
            .map_err(|_| LayerPatchRecordError::InvalidPayload)?;

        let chord_len = usize::from(cursor.u8()?);
        if chord_len > CHORD_MEMORY_CAPACITY {
            return Err(LayerPatchRecordError::InvalidPayload);
        }
        let chord = cursor.bytes(CHORD_MEMORY_CAPACITY)?;
        patch.unison_chord = crate::ChordMemory::from_intervals(&chord[..chord_len]);
        if cursor.position() != PAYLOAD_LEN {
            return Err(LayerPatchRecordError::InvalidLength);
        }
        Ok(patch)
    }

    pub fn is_erased(input: &[u8; LAYER_PATCH_RECORD_SIZE]) -> bool {
        input.iter().all(|byte| *byte == 0xff)
    }
}

struct Writer<'a> {
    bytes: &'a mut [u8],
    position: usize,
}

impl<'a> Writer<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn u8(&mut self, value: u8) {
        self.bytes[self.position] = value;
        self.position += 1;
    }

    fn u16(&mut self, value: u16) {
        self.bytes[self.position..self.position + 2].copy_from_slice(&value.to_le_bytes());
        self.position += 2;
    }

    fn bytes(&mut self, value: &[u8]) {
        self.bytes[self.position..self.position + value.len()].copy_from_slice(value);
        self.position += value.len();
    }

    fn zeroes(&mut self, count: usize) {
        self.bytes[self.position..self.position + count].fill(0);
        self.position += count;
    }

    fn position(&self) -> usize {
        self.position
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn u8(&mut self) -> Result<u8, LayerPatchRecordError> {
        let value = *self
            .bytes
            .get(self.position)
            .ok_or(LayerPatchRecordError::InvalidLength)?;
        self.position += 1;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, LayerPatchRecordError> {
        let bytes = self.bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn bytes(&mut self, count: usize) -> Result<&'a [u8], LayerPatchRecordError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(LayerPatchRecordError::InvalidLength)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(LayerPatchRecordError::InvalidLength)?;
        self.position = end;
        Ok(value)
    }

    fn position(&self) -> usize {
        self.position
    }
}

fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x007f_ffff;

    if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x0080_0000;
        let shift = (14 - exponent) as u32;
        let mut half = (mantissa >> shift) as u16;
        let remainder_mask = (1_u32 << shift) - 1;
        let remainder = mantissa & remainder_mask;
        let halfway = 1_u32 << (shift - 1);
        if remainder > halfway || (remainder == halfway && half & 1 != 0) {
            half = half.wrapping_add(1);
        }
        return sign | half;
    }
    if exponent >= 31 {
        return sign | 0x7c00;
    }

    let mut half = sign | ((exponent as u16) << 10) | ((mantissa >> 13) as u16);
    let remainder = mantissa & 0x1fff;
    if remainder > 0x1000 || (remainder == 0x1000 && half & 1 != 0) {
        half = half.wrapping_add(1);
    }
    half
}

fn encode_value(value: f32) -> Result<u16, LayerPatchRecordError> {
    if !value.is_finite() {
        return Err(LayerPatchRecordError::NonFiniteValue);
    }
    let encoded = f32_to_f16(value);
    if encoded & 0x7c00 == 0x7c00 {
        Err(LayerPatchRecordError::ValueOutOfRange)
    } else {
        Ok(encoded)
    }
}

fn f16_to_f32(value: u16) -> f32 {
    let sign = (u32::from(value & 0x8000)) << 16;
    let exponent = u32::from((value >> 10) & 0x1f);
    let mantissa = u32::from(value & 0x03ff);
    let bits = match exponent {
        0 if mantissa == 0 => sign,
        0 => {
            let leading = mantissa.leading_zeros() - 22;
            let normalized = (mantissa << (leading + 1)) & 0x03ff;
            let exponent = 127 - 15 - leading;
            sign | (exponent << 23) | (normalized << 13)
        }
        31 => sign | 0x7f80_0000 | (mantissa << 13),
        _ => sign | ((exponent + 112) << 23) | (mantissa << 13),
    };
    f32::from_bits(bits)
}

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320_u32 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

const PARAM_IDS: [ParamId; 107] = [
    ParamId::Osc1Waveform,
    ParamId::Osc1Enabled,
    ParamId::Osc1Frequency,
    ParamId::Osc1FineTune,
    ParamId::Osc1ShapeMod,
    ParamId::Osc1Level,
    ParamId::Osc1NoteReset,
    ParamId::Osc1KeyboardOn,
    ParamId::Osc1Glide,
    ParamId::Osc2Waveform,
    ParamId::Osc2Enabled,
    ParamId::Osc2Frequency,
    ParamId::Osc2FineTune,
    ParamId::Osc2ShapeMod,
    ParamId::Osc2Level,
    ParamId::Osc2NoteReset,
    ParamId::Osc2KeyboardOn,
    ParamId::Osc2Glide,
    ParamId::OscMix,
    ParamId::SubOscLevel,
    ParamId::NoiseLevel,
    ParamId::HardSync,
    ParamId::OscSlop,
    ParamId::GlideMode,
    ParamId::GlideEnabled,
    ParamId::PitchBendRange,
    ParamId::KeyMode,
    ParamId::UnisonEnabled,
    ParamId::UnisonMode,
    ParamId::UnisonDetune,
    ParamId::Bpm,
    ParamId::ClockDivide,
    ParamId::FilterCutoff,
    ParamId::FilterResonance,
    ParamId::FilterPoles,
    ParamId::FilterKeyTrack,
    ParamId::FilterEnvAmount,
    ParamId::FilterVelocity,
    ParamId::FilterAudioMod,
    ParamId::FilterEgDelay,
    ParamId::FilterEgAttack,
    ParamId::FilterEgDecay,
    ParamId::FilterEgSustain,
    ParamId::FilterEgRelease,
    ParamId::PanSpread,
    ParamId::PanModMode,
    ParamId::VcaInitialLevel,
    ParamId::AmpEnvAmount,
    ParamId::AmpVelocity,
    ParamId::AmpEgDelay,
    ParamId::AmpEgAttack,
    ParamId::AmpEgDecay,
    ParamId::AmpEgSustain,
    ParamId::AmpEgRelease,
    ParamId::AuxEgDestination,
    ParamId::AuxEgAmount,
    ParamId::AuxEgVelocity,
    ParamId::AuxEgDelay,
    ParamId::AuxEgAttack,
    ParamId::AuxEgDecay,
    ParamId::AuxEgSustain,
    ParamId::AuxEgRelease,
    ParamId::AuxEgLoop,
    ParamId::Lfo1Rate,
    ParamId::Lfo1Depth,
    ParamId::Lfo1Waveform,
    ParamId::Lfo1Destination,
    ParamId::Lfo1ClockSync,
    ParamId::Lfo1SyncDivision,
    ParamId::Lfo1KeySync,
    ParamId::Lfo2Rate,
    ParamId::Lfo2Depth,
    ParamId::Lfo2Waveform,
    ParamId::Lfo2Destination,
    ParamId::Lfo2ClockSync,
    ParamId::Lfo2SyncDivision,
    ParamId::Lfo2KeySync,
    ParamId::Lfo3Rate,
    ParamId::Lfo3Depth,
    ParamId::Lfo3Waveform,
    ParamId::Lfo3Destination,
    ParamId::Lfo3ClockSync,
    ParamId::Lfo3SyncDivision,
    ParamId::Lfo3KeySync,
    ParamId::Lfo4Rate,
    ParamId::Lfo4Depth,
    ParamId::Lfo4Waveform,
    ParamId::Lfo4Destination,
    ParamId::Lfo4ClockSync,
    ParamId::Lfo4SyncDivision,
    ParamId::Lfo4KeySync,
    ParamId::EffectEnabled,
    ParamId::EffectType,
    ParamId::EffectMix,
    ParamId::EffectClockSync,
    ParamId::EffectParam1,
    ParamId::EffectParam2,
    ParamId::ArpEnabled,
    ParamId::ArpMode,
    ParamId::ArpRange,
    ParamId::ArpRepeats,
    ParamId::ArpRelatch,
    ParamId::ArpHold,
    ParamId::ArpBeatSync,
    ParamId::ArpSustainMode,
    ParamId::MasterVolume,
    ParamId::AnalogDrift,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DedicatedModSlot, ModMatrixSlot};
    use rand_core::{Rng, SeedableRng};
    use rand_pcg::Pcg32;

    fn assert_close(expected: f32, actual: f32) {
        let tolerance = expected.abs().max(1.0) * 0.001;
        assert!(
            (expected - actual).abs() <= tolerance,
            "{expected} != {actual}"
        );
    }

    fn random_unit(rng: &mut impl Rng) -> f32 {
        rng.next_u32() as f32 / u32::MAX as f32
    }

    fn assert_patch_close(expected: &LayerPatch, actual: &LayerPatch) {
        let mut actual_values = [0.0; PARAM_COUNT];
        let mut count = 0;
        actual.for_each_param(|_, value| {
            actual_values[count] = value;
            count += 1;
        });
        let mut count = 0;
        expected.for_each_param(|_, value| {
            assert_close(value, actual_values[count]);
            count += 1;
        });
        assert_eq!(expected.name, actual.name);
        assert_eq!(expected.unison_chord, actual.unison_chord);
        for (expected, actual) in expected
            .mod_matrix
            .free_slots
            .iter()
            .zip(actual.mod_matrix.free_slots.iter())
        {
            assert_eq!(expected.enabled, actual.enabled);
            assert_eq!(expected.source, actual.source);
            assert_eq!(expected.destination, actual.destination);
            assert_close(expected.amount, actual.amount);
        }
        for (expected, actual) in expected
            .mod_matrix
            .dedicated
            .iter()
            .zip(actual.mod_matrix.dedicated.iter())
        {
            assert_eq!(expected.enabled, actual.enabled);
            assert_eq!(expected.destination, actual.destination);
            assert_close(expected.amount, actual.amount);
        }
    }

    #[test]
    fn default_patch_round_trips() {
        let mut record = [0; LAYER_PATCH_RECORD_SIZE];
        let patch = LayerPatch::default();
        LayerPatchRecord::encode(&patch, &mut record).unwrap();
        let decoded = LayerPatchRecord::decode(&record).unwrap();
        assert_patch_close(&patch, &decoded);
        assert!(
            record[HEADER_LEN + PAYLOAD_LEN..]
                .iter()
                .all(|byte| *byte == 0xff)
        );
    }

    #[test]
    fn populated_patch_round_trips() {
        let mut patch = LayerPatch::default();
        patch.name.push_str("Storage test é").unwrap();
        patch.unison_chord = crate::ChordMemory::from_intervals(&[0, 3, 7, 12]);
        patch.filter.cutoff = 4321.25;
        patch.osc1.fine_tune = -17.75;
        patch.lfos[2].rate_hz = 13.125;
        patch.mod_matrix.free_slots[3] = ModMatrixSlot {
            enabled: true,
            source: ModSource::Velocity,
            destination: ModDestination::FilterCutoff,
            amount: -0.625,
        };
        patch.mod_matrix.dedicated[1] = DedicatedModSlot {
            enabled: true,
            destination: ModDestination::FxMix,
            amount: 0.375,
        };
        let mut record = [0; LAYER_PATCH_RECORD_SIZE];
        LayerPatchRecord::encode(&patch, &mut record).unwrap();
        assert_patch_close(&patch, &LayerPatchRecord::decode(&record).unwrap());
    }

    #[test]
    fn randomized_patches_round_trip() {
        let mut rng = Pcg32::seed_from_u64(0x5eed_f1a5_1234_5678);
        for _ in 0..128 {
            let mut patch = LayerPatch::default();
            for id in PARAM_IDS {
                patch.set_param(id, random_unit(&mut rng) * 510.0 - 10.0);
            }
            for slot in &mut patch.mod_matrix.free_slots {
                slot.enabled = rng.next_u32() & 1 != 0;
                slot.source = ModSource::from_index(rng.next_u32() as usize % ModSource::ALL.len());
                slot.destination =
                    ModDestination::from_index(rng.next_u32() as usize % ModDestination::COUNT);
                slot.amount = random_unit(&mut rng) * 2.0 - 1.0;
            }
            for slot in &mut patch.mod_matrix.dedicated {
                slot.enabled = rng.next_u32() & 1 != 0;
                slot.destination =
                    ModDestination::from_index(rng.next_u32() as usize % ModDestination::COUNT);
                slot.amount = random_unit(&mut rng) * 2.0 - 1.0;
            }
            let mut record = [0; LAYER_PATCH_RECORD_SIZE];
            LayerPatchRecord::encode(&patch, &mut record).unwrap();
            assert_patch_close(&patch, &LayerPatchRecord::decode(&record).unwrap());
        }
    }

    #[test]
    fn all_factory_rev2_programs_round_trip_through_record() {
        const FACTORY_BANK: &[u8] =
            include_bytes!("../../Prophet-Rev2-Factory-Programs/Rev2_Programs_v1.0.syx");
        assert_eq!(
            FACTORY_BANK.len() % crate::midi::rev2::PROGRAM_DATA_SYSEX_LEN,
            0
        );
        for message in FACTORY_BANK.chunks_exact(crate::midi::rev2::PROGRAM_DATA_SYSEX_LEN) {
            let imported = crate::midi::rev2::decode::program_data(message).unwrap();
            let mut record = [0; LAYER_PATCH_RECORD_SIZE];
            LayerPatchRecord::encode(&imported.patch.layer_a, &mut record).unwrap_or_else(
                |error| {
                    panic!(
                        "encode bank={} program={}: {error:?}",
                        imported.bank, imported.program
                    )
                },
            );
            let decoded = LayerPatchRecord::decode(&record).unwrap_or_else(|error| {
                panic!(
                    "decode bank={} program={}: {error:?}",
                    imported.bank, imported.program
                )
            });
            assert_patch_close(&imported.patch.layer_a, &decoded);
        }
    }

    #[test]
    fn factory_rev2_program_round_trips_through_record() {
        const FACTORY_BANK: &[u8] =
            include_bytes!("../../Prophet-Rev2-Factory-Programs/Rev2_Programs_v1.0.syx");
        let imported = crate::midi::rev2::decode::program_data(
            &FACTORY_BANK[..crate::midi::rev2::PROGRAM_DATA_SYSEX_LEN],
        )
        .unwrap()
        .patch
        .layer_a;
        let mut record = [0; LAYER_PATCH_RECORD_SIZE];
        LayerPatchRecord::encode(&imported, &mut record).unwrap();
        assert_patch_close(&imported, &LayerPatchRecord::decode(&record).unwrap());
    }

    #[test]
    fn erased_slot_is_default_patch() {
        let erased = [0xff; LAYER_PATCH_RECORD_SIZE];
        let mut encoded_default = [0; LAYER_PATCH_RECORD_SIZE];
        LayerPatchRecord::encode(&LayerPatch::default(), &mut encoded_default).unwrap();
        assert_patch_close(
            &LayerPatchRecord::decode(&encoded_default).unwrap(),
            &LayerPatchRecord::decode(&erased).unwrap(),
        );
    }

    #[test]
    fn corruption_and_versions_are_rejected() {
        let mut record = [0; LAYER_PATCH_RECORD_SIZE];
        LayerPatchRecord::encode(&LayerPatch::default(), &mut record).unwrap();
        record[HEADER_LEN + 7] ^= 1;
        assert!(matches!(
            LayerPatchRecord::decode(&record),
            Err(LayerPatchRecordError::ChecksumMismatch)
        ));
        LayerPatchRecord::encode(&LayerPatch::default(), &mut record).unwrap();
        record[4] += 1;
        assert!(matches!(
            LayerPatchRecord::decode(&record),
            Err(LayerPatchRecordError::UnsupportedVersion)
        ));
    }

    #[test]
    fn values_outside_binary16_range_are_rejected_before_storage() {
        let mut patch = LayerPatch::default();
        patch.filter.cutoff = f32::MAX;
        let mut record = [0; LAYER_PATCH_RECORD_SIZE];
        assert_eq!(
            LayerPatchRecord::encode(&patch, &mut record),
            Err(LayerPatchRecordError::ValueOutOfRange)
        );
    }

    #[test]
    fn binary16_handles_storage_value_range() {
        for value in [0.0, -0.0, 0.0005, 0.022, 0.25, 1.0, 120.0, 500.0, 20_000.0] {
            assert_close(value, f16_to_f32(f32_to_f16(value)));
        }
    }

    #[test]
    fn crc32_matches_standard_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }
}
