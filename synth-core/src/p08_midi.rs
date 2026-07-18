//! Sequential Prophet '08-compatible program SysEx decoder.

use crate::{
    DedicatedModSource, MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ, ModDestination, ModRoute, ModSource,
    ModulationParam, ParamId, Patch,
};
use crate::rev2_midi::Rev2SysexError;

pub const P08_PROGRAM_DATA_LEN: usize = 384;
pub const P08_PROGRAM_PACKED_LEN: usize = 439;
pub const P08_PROGRAM_DATA_SYSEX_LEN: usize = 446;
pub const P08_PROGRAM_EDIT_BUFFER_SYSEX_LEN: usize = 444;
const P08_SYSEX_MANUFACTURER: u8 = 0x01;
const P08_SYSEX_MODEL: u8 = 0x23;

#[derive(Debug, Clone)]
pub struct P08ProgramData {
    pub bank: u8,
    pub program: u8,
    pub patch: Patch,
}

#[derive(Clone, Copy)]
struct ProgramField {
    value_offset: usize,
    msb_offset: Option<usize>,
}

pub struct P08MidiDecoder;

impl P08MidiDecoder {
    pub fn program_data(message: &[u8]) -> Result<P08ProgramData, Rev2SysexError> {
        validate_header(message, P08_PROGRAM_DATA_SYSEX_LEN, 0x02)?;
        let bank = message[4];
        let program = message[5];
        if bank > 1 {
            return Err(Rev2SysexError::InvalidBank);
        }
        if program & 0x80 != 0 {
            return Err(Rev2SysexError::NonSevenBitData);
        }
        let patch = decode_patch_payload(&message[6..6 + P08_PROGRAM_PACKED_LEN])?;
        Ok(P08ProgramData {
            bank,
            program,
            patch,
        })
    }

    pub fn program_edit_buffer(message: &[u8]) -> Result<Patch, Rev2SysexError> {
        validate_header(message, P08_PROGRAM_EDIT_BUFFER_SYSEX_LEN, 0x03)?;
        decode_patch_payload(&message[4..4 + P08_PROGRAM_PACKED_LEN])
    }
}

fn validate_header(
    message: &[u8],
    expected_len: usize,
    expected_command: u8,
) -> Result<(), Rev2SysexError> {
    if message.len() != expected_len {
        return Err(Rev2SysexError::InvalidLength);
    }
    if message[0] != 0xf0 || message[expected_len - 1] != 0xf7 {
        return Err(Rev2SysexError::InvalidFraming);
    }
    if message[1] != P08_SYSEX_MANUFACTURER {
        return Err(Rev2SysexError::InvalidManufacturer);
    }
    if message[2] != P08_SYSEX_MODEL {
        return Err(Rev2SysexError::InvalidModel);
    }
    if message[3] != expected_command {
        return Err(Rev2SysexError::UnsupportedCommand);
    }
    Ok(())
}

fn decode_patch_payload(packed: &[u8]) -> Result<Patch, Rev2SysexError> {
    if packed.iter().any(|byte| byte & 0x80 != 0) {
        return Err(Rev2SysexError::NonSevenBitData);
    }

    let mut raw = [0_u8; P08_PROGRAM_DATA_LEN];
    unpack_program_data(packed, &mut raw);
    let mut patch = Patch::default();
    for number in 0..=119 {
        if let Some(value) = program_nrpn_value(&raw, number) {
            map_nrpn(number, value, &mut |update| match update {
                P08MidiUpdate::Param(param, value) => patch.set_param(param, value),
                P08MidiUpdate::Modulation { route, parameter } => {
                    patch.set_modulation_param(route, parameter);
                }
            });
        }
    }
    Ok(patch)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum P08MidiUpdate {
    Param(ParamId, f32),
    Modulation {
        route: ModRoute,
        parameter: ModulationParam,
    },
}

fn program_field(number: u16) -> Option<ProgramField> {
    if number > 119 {
        return None;
    }
    let value_offset = number as usize;
    let msb_offset = match number {
        15 => Some(19),
        20 => Some(14),
        37 => Some(39),
        42 => Some(48),
        47 => Some(43),
        52 => Some(52),
        58 => Some(60),
        69 => Some(63),
        72 => Some(74),
        75 => Some(71),
        81 => Some(19),
        83 => Some(52),
        85 => Some(89),
        87 => Some(87),
        89 => Some(85),
        _ => None,
    };
    Some(ProgramField {
        value_offset,
        msb_offset,
    })
}

fn program_nrpn_value(raw: &[u8], number: u16) -> Option<u16> {
    let field = program_field(number)?;
    let value = *raw.get(field.value_offset)?;
    if let Some(msb_offset) = field.msb_offset {
        Some(u16::from(value & 0x7f) | u16::from(*raw.get(msb_offset)? & 0x80))
    } else if nrpn_max(number).is_some_and(|maximum| maximum > 127) {
        Some(u16::from(value))
    } else {
        Some(u16::from(value & 0x7f))
    }
}

fn unpack_program_data(packed: &[u8], raw: &mut [u8; P08_PROGRAM_DATA_LEN]) {
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
    debug_assert_eq!(input, P08_PROGRAM_PACKED_LEN);
}

fn unit(raw: u16, max: u16) -> f32 {
    f32::from(raw.min(max)) / f32::from(max)
}

fn bipolar(raw: u16, max: u16) -> f32 {
    unit(raw, max) * 2.0 - 1.0
}

fn ranged(raw: u16, raw_max: u16, min: f32, max: f32) -> f32 {
    min + unit(raw, raw_max) * (max - min)
}

fn logarithmic(raw: u16, raw_max: u16, min: f32, max: f32) -> f32 {
    min * crate::math::powf(max / min, unit(raw, raw_max))
}

fn emit_osc_shape(emit: &mut impl FnMut(P08MidiUpdate), osc1: bool, raw: u16) {
    let (enabled_param, waveform_param, shape_param) = if osc1 {
        (ParamId::Osc1Enabled, ParamId::Osc1Waveform, ParamId::Osc1Shape)
    } else {
        (ParamId::Osc2Enabled, ParamId::Osc2Waveform, ParamId::Osc2Shape)
    };
    emit(P08MidiUpdate::Param(enabled_param, f32::from(raw != 0)));
    if raw == 0 {
        return;
    }
    if raw <= 3 {
        emit(P08MidiUpdate::Param(waveform_param, f32::from(raw - 1)));
        emit(P08MidiUpdate::Param(shape_param, 0.0));
        return;
    }
    emit(P08MidiUpdate::Param(waveform_param, 3.0));
    emit(P08MidiUpdate::Param(shape_param, unit(raw.saturating_sub(4), 99)));
}

fn map_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(P08MidiUpdate)) {
    match number {
        0 => emit(P08MidiUpdate::Param(ParamId::Osc1Frequency, f32::from(raw.min(120)))),
        1 => emit(P08MidiUpdate::Param(
            ParamId::Osc1FineTune,
            f32::from(raw.min(100)) - 50.0,
        )),
        2 => emit_osc_shape(emit, true, raw.min(103)),
        5 => emit(P08MidiUpdate::Param(ParamId::Osc2Frequency, f32::from(raw.min(120)))),
        6 => emit(P08MidiUpdate::Param(
            ParamId::Osc2FineTune,
            f32::from(raw.min(100)) - 50.0,
        )),
        7 => emit_osc_shape(emit, false, raw.min(103)),
        10 => emit(P08MidiUpdate::Param(ParamId::HardSync, f32::from(raw != 0))),
        12 => emit(P08MidiUpdate::Param(ParamId::OscSlop, unit(raw, 5))),
        13 => emit(P08MidiUpdate::Param(ParamId::OscMix, unit(raw, 127))),
        14 => emit(P08MidiUpdate::Param(ParamId::NoiseLevel, unit(raw, 127))),
        15 => emit(P08MidiUpdate::Param(
            ParamId::FilterCutoff,
            logarithmic(raw, 164, 20.0, 20_000.0),
        )),
        16 => emit(P08MidiUpdate::Param(ParamId::FilterResonance, unit(raw, 127))),
        17 => emit(P08MidiUpdate::Param(ParamId::FilterKeyTrack, unit(raw, 127))),
        18 => emit(P08MidiUpdate::Param(ParamId::FilterAudioMod, unit(raw, 127))),
        19 => emit(P08MidiUpdate::Param(ParamId::FilterPoles, f32::from(raw != 0))),
        20 => emit(P08MidiUpdate::Param(ParamId::FilterEnvAmount, bipolar(raw, 254))),
        21 => emit(P08MidiUpdate::Param(ParamId::FilterVelocity, unit(raw, 127))),
        22 => emit(P08MidiUpdate::Param(ParamId::FilterEgDelay, ranged(raw, 127, 0.0, 5.0))),
        23 => emit(P08MidiUpdate::Param(
            ParamId::FilterEgAttack,
            ranged(raw, 127, 0.0005, 5.0),
        )),
        24 => emit(P08MidiUpdate::Param(
            ParamId::FilterEgDecay,
            ranged(raw, 127, 0.0005, 5.0),
        )),
        25 => emit(P08MidiUpdate::Param(ParamId::FilterEgSustain, unit(raw, 127))),
        26 => emit(P08MidiUpdate::Param(
            ParamId::FilterEgRelease,
            ranged(raw, 127, 0.0005, 10.0),
        )),
        28 => emit(P08MidiUpdate::Param(ParamId::PanSpread, unit(raw, 127))),
        29 => emit(P08MidiUpdate::Param(ParamId::MasterVolume, unit(raw, 127))),
        30 => emit(P08MidiUpdate::Param(ParamId::AmpEnvAmount, unit(raw, 127))),
        31 => emit(P08MidiUpdate::Param(ParamId::AmpVelocity, unit(raw, 127))),
        32 => emit(P08MidiUpdate::Param(ParamId::AmpEgDelay, ranged(raw, 127, 0.0, 5.0))),
        33 => emit(P08MidiUpdate::Param(
            ParamId::AmpEgAttack,
            ranged(raw, 127, 0.0005, 5.0),
        )),
        34 => emit(P08MidiUpdate::Param(
            ParamId::AmpEgDecay,
            ranged(raw, 127, 0.0005, 5.0),
        )),
        35 => emit(P08MidiUpdate::Param(ParamId::AmpEgSustain, unit(raw, 127))),
        36 => emit(P08MidiUpdate::Param(
            ParamId::AmpEgRelease,
            ranged(raw, 127, 0.0005, 10.0),
        )),
        37..=56 => map_lfo_nrpn(number, raw, emit),
        57 => emit(P08MidiUpdate::Param(
            ParamId::AuxEgDestination,
            f32::from(raw.min(43)),
        )),
        58 => emit(P08MidiUpdate::Param(ParamId::AuxEgAmount, bipolar(raw, 254))),
        59 => emit(P08MidiUpdate::Param(ParamId::AuxEgVelocity, unit(raw, 127))),
        60 => emit(P08MidiUpdate::Param(ParamId::AuxEgDelay, ranged(raw, 127, 0.0, 5.0))),
        61 => emit(P08MidiUpdate::Param(
            ParamId::AuxEgAttack,
            ranged(raw, 127, 0.0005, 5.0),
        )),
        62 => emit(P08MidiUpdate::Param(
            ParamId::AuxEgDecay,
            ranged(raw, 127, 0.0005, 5.0),
        )),
        63 => emit(P08MidiUpdate::Param(ParamId::AuxEgSustain, unit(raw, 127))),
        64 => emit(P08MidiUpdate::Param(
            ParamId::AuxEgRelease,
            ranged(raw, 127, 0.0005, 10.0),
        )),
        65..=76 => map_free_mod_nrpn(number, raw, emit),
        81..=90 => map_dedicated_mod_nrpn(number, raw, emit),
        _ => {}
    }
}

fn map_lfo_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(P08MidiUpdate)) {
    let lfo = usize::from((number - 37) / 5);
    let field = (number - 37) % 5;
    let params = [
        [
            ParamId::Lfo1Rate,
            ParamId::Lfo1Waveform,
            ParamId::Lfo1Depth,
            ParamId::Lfo1Destination,
            ParamId::Lfo1KeySync,
        ],
        [
            ParamId::Lfo2Rate,
            ParamId::Lfo2Waveform,
            ParamId::Lfo2Depth,
            ParamId::Lfo2Destination,
            ParamId::Lfo2KeySync,
        ],
        [
            ParamId::Lfo3Rate,
            ParamId::Lfo3Waveform,
            ParamId::Lfo3Depth,
            ParamId::Lfo3Destination,
            ParamId::Lfo3KeySync,
        ],
        [
            ParamId::Lfo4Rate,
            ParamId::Lfo4Waveform,
            ParamId::Lfo4Depth,
            ParamId::Lfo4Destination,
            ParamId::Lfo4KeySync,
        ],
    ];
    let value = match field {
        0 => logarithmic(raw, 166, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ),
        1 => f32::from(raw.min(4)),
        2 => unit(raw, 127),
        3 => f32::from(raw.min(43)),
        _ => f32::from(raw != 0),
    };
    emit(P08MidiUpdate::Param(params[lfo][field as usize], value));
}

fn map_free_mod_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(P08MidiUpdate)) {
    let index = usize::from((number - 65) / 3);
    let parameter = match (number - 65) % 3 {
        0 => ModulationParam::Source(ModSource::from_index(usize::from(raw.min(20)))),
        1 => ModulationParam::Amount(bipolar(raw, 254)),
        _ => ModulationParam::Destination(ModDestination::from_index(usize::from(raw.min(43)))),
    };
    emit(P08MidiUpdate::Modulation {
        route: ModRoute::Free(index),
        parameter,
    });
}

fn map_dedicated_mod_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(P08MidiUpdate)) {
    let index = usize::from((number - 81) / 2);
    let Some(source) = DedicatedModSource::ALL.get(index) else {
        return;
    };
    let parameter = if (number - 81) % 2 == 0 {
        ModulationParam::Amount(bipolar(raw, 254))
    } else {
        ModulationParam::Destination(ModDestination::from_index(usize::from(raw.min(43))))
    };
    emit(P08MidiUpdate::Modulation {
        route: ModRoute::Dedicated(*source),
        parameter,
    });
}

fn nrpn_max(number: u16) -> Option<u16> {
    Some(match number {
        0 | 5 => 120,
        1 | 6 => 100,
        2 | 7 => 103,
        10 | 19 | 41 | 46 | 51 | 56 => 1,
        12 => 5,
        15 => 164,
        20 | 58 | 66 | 69 | 72 | 75 | 81 | 83 | 85 | 87 | 89 => 254,
        37 | 42 | 47 | 52 => 166,
        38 | 43 | 48 | 53 => 4,
        40 | 45 | 50 | 55 | 57 => 43,
        _ => 127,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACTORY_SYSEX: &[u8] =
        include_bytes!("../../Prophet_08_Programs+ReadMe/Prophet_08_Programs_v1.0.syx");

    fn factory_message(bank: usize, program: usize) -> &'static [u8] {
        let offset = (bank * 128 + program) * P08_PROGRAM_DATA_SYSEX_LEN;
        &FACTORY_SYSEX[offset..offset + P08_PROGRAM_DATA_SYSEX_LEN]
    }

    #[test]
    fn stored_program_data_decodes_factory_metadata() {
        let decoded = P08MidiDecoder::program_data(factory_message(0, 0)).unwrap();
        assert_eq!(decoded.bank, 0);
        assert_eq!(decoded.program, 0);
        assert!(decoded.patch.osc1.enabled);

        let decoded = P08MidiDecoder::program_data(factory_message(1, 0)).unwrap();
        assert_eq!(decoded.bank, 1);
        assert_eq!(decoded.program, 0);
    }

    #[test]
    fn rejects_rev2_program_data_message() {
        let mut message = [0_u8; P08_PROGRAM_DATA_SYSEX_LEN];
        message[0] = 0xf0;
        message[1] = 0x01;
        message[2] = 0x2f;
        message[3] = 0x02;
        message[P08_PROGRAM_DATA_SYSEX_LEN - 1] = 0xf7;
        assert!(matches!(
            P08MidiDecoder::program_data(&message),
            Err(Rev2SysexError::InvalidModel)
        ));
    }
}
