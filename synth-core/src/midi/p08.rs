//! Sequential Prophet '08-compatible program SysEx decoder.

use super::rev2::Rev2SysexError;
use crate::dsp::{MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
use crate::math::F32;
use crate::patch::decode_patch_name;
use crate::{
    DedicatedModSource, LfoSyncDivision, ModDestination, ModRoute, ModSource, ModulationParam,
    ParamId, Patch,
};

const LAYER_A_NAME_RANGE: core::ops::Range<usize> = 184..200;

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
    patch.glide_enabled = patch.osc1.glide > 0.0 || patch.osc2.glide > 0.0;
    patch.name = decode_patch_name(&raw[LAYER_A_NAME_RANGE]);
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
    if number > 119 && number != 100 {
        return None;
    }
    let value_offset = match number {
        100 => 93,
        _ => number as usize,
    };
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
    min * F32(max / min).powf(F32(unit(raw, raw_max))).as_f32()
}

const P08_MOD_DESTINATIONS: [ModDestination; 44] = [
    ModDestination::Off,
    ModDestination::Osc1Frequency,
    ModDestination::Osc2Frequency,
    ModDestination::OscAllFrequency,
    ModDestination::OscMix,
    ModDestination::NoiseLevel,
    ModDestination::Osc1ShapeMod,
    ModDestination::Osc2ShapeMod,
    ModDestination::OscAllShapeMod,
    ModDestination::FilterCutoff,
    ModDestination::FilterResonance,
    ModDestination::FilterAudioMod,
    ModDestination::Vca,
    ModDestination::Pan,
    ModDestination::Lfo1Frequency,
    ModDestination::Lfo2Frequency,
    ModDestination::Lfo3Frequency,
    ModDestination::Lfo4Frequency,
    ModDestination::LfoAllFrequency,
    ModDestination::Lfo1Amount,
    ModDestination::Lfo2Amount,
    ModDestination::Lfo3Amount,
    ModDestination::Lfo4Amount,
    ModDestination::LfoAllAmount,
    ModDestination::LpFilterEnvAmount,
    ModDestination::AmpEnvAmount,
    ModDestination::Env3Amount,
    ModDestination::EnvAllAmount,
    ModDestination::LpFilterAttack,
    ModDestination::VcaAttack,
    ModDestination::Env3Attack,
    ModDestination::EnvAllAttack,
    ModDestination::LpFilterDecay,
    ModDestination::VcaDecay,
    ModDestination::Env3Decay,
    ModDestination::EnvAllDecay,
    ModDestination::LpFilterRelease,
    ModDestination::VcaRelease,
    ModDestination::Env3Release,
    ModDestination::EnvAllRelease,
    ModDestination::Mod1Amount,
    ModDestination::Mod2Amount,
    ModDestination::Mod3Amount,
    ModDestination::Mod4Amount,
];

fn p08_mod_destination(raw: u16) -> ModDestination {
    P08_MOD_DESTINATIONS
        .get(usize::from(raw.min(43)))
        .copied()
        .unwrap_or(ModDestination::Off)
}

fn p08_lfo_waveform(raw: u16) -> f32 {
    match raw.min(4) {
        0 => 0.0,
        1 => 2.0,
        2 => 1.0,
        3 => 3.0,
        4 => 4.0,
        _ => 0.0,
    }
}

fn p08_lfo_rate_hz(raw: u16) -> f32 {
    logarithmic(raw.min(150), 150, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ)
}

fn emit_osc_shape(emit: &mut impl FnMut(P08MidiUpdate), osc1: bool, raw: u16) {
    let (enabled_param, waveform_param, shape_param) = if osc1 {
        (
            ParamId::Osc1Enabled,
            ParamId::Osc1Waveform,
            ParamId::Osc1ShapeMod,
        )
    } else {
        (
            ParamId::Osc2Enabled,
            ParamId::Osc2Waveform,
            ParamId::Osc2ShapeMod,
        )
    };
    emit(P08MidiUpdate::Param(enabled_param, f32::from(raw != 0)));
    if raw == 0 {
        return;
    }
    match raw {
        1 => {
            emit(P08MidiUpdate::Param(waveform_param, 0.0));
            emit(P08MidiUpdate::Param(shape_param, 0.0));
        }
        2 => {
            emit(P08MidiUpdate::Param(waveform_param, 2.0));
            emit(P08MidiUpdate::Param(shape_param, 0.0));
        }
        3 => {
            emit(P08MidiUpdate::Param(waveform_param, 1.0));
            emit(P08MidiUpdate::Param(shape_param, 0.0));
        }
        _ => {
            emit(P08MidiUpdate::Param(waveform_param, 3.0));
            emit(P08MidiUpdate::Param(
                shape_param,
                unit(raw.saturating_sub(4), 99),
            ));
        }
    }
}

fn map_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(P08MidiUpdate)) {
    match number {
        0 => emit(P08MidiUpdate::Param(
            ParamId::Osc1Frequency,
            f32::from(raw.min(120)),
        )),
        1 => emit(P08MidiUpdate::Param(
            ParamId::Osc1FineTune,
            f32::from(raw.min(100)) - 50.0,
        )),
        2 => emit_osc_shape(emit, true, raw.min(103)),
        3 => emit(P08MidiUpdate::Param(ParamId::Osc1Glide, unit(raw, 127))),
        4 => emit(P08MidiUpdate::Param(
            ParamId::Osc1KeyboardOn,
            f32::from(raw != 0),
        )),
        5 => emit(P08MidiUpdate::Param(
            ParamId::Osc2Frequency,
            f32::from(raw.min(120)),
        )),
        6 => emit(P08MidiUpdate::Param(
            ParamId::Osc2FineTune,
            f32::from(raw.min(100)) - 50.0,
        )),
        7 => emit_osc_shape(emit, false, raw.min(103)),
        8 => emit(P08MidiUpdate::Param(ParamId::Osc2Glide, unit(raw, 127))),
        9 => emit(P08MidiUpdate::Param(
            ParamId::Osc2KeyboardOn,
            f32::from(raw != 0),
        )),
        10 => emit(P08MidiUpdate::Param(ParamId::HardSync, f32::from(raw != 0))),
        11 => emit(P08MidiUpdate::Param(
            ParamId::GlideMode,
            f32::from(raw.min(3)),
        )),
        12 => emit(P08MidiUpdate::Param(ParamId::OscSlop, unit(raw, 5))),
        13 => emit(P08MidiUpdate::Param(ParamId::OscMix, unit(raw, 127))),
        14 => emit(P08MidiUpdate::Param(ParamId::NoiseLevel, unit(raw, 127))),
        15 => emit(P08MidiUpdate::Param(
            ParamId::FilterCutoff,
            logarithmic(raw, 164, 20.0, 20_000.0),
        )),
        16 => emit(P08MidiUpdate::Param(
            ParamId::FilterResonance,
            unit(raw, 127),
        )),
        17 => emit(P08MidiUpdate::Param(
            ParamId::FilterKeyTrack,
            unit(raw, 127),
        )),
        18 => emit(P08MidiUpdate::Param(
            ParamId::FilterAudioMod,
            unit(raw, 127),
        )),
        19 => emit(P08MidiUpdate::Param(
            ParamId::FilterPoles,
            f32::from(raw != 0),
        )),
        20 => emit(P08MidiUpdate::Param(
            ParamId::FilterEnvAmount,
            bipolar(raw, 254),
        )),
        21 => emit(P08MidiUpdate::Param(
            ParamId::FilterVelocity,
            unit(raw, 127),
        )),
        22 => emit(P08MidiUpdate::Param(
            ParamId::FilterEgDelay,
            ranged(raw, 127, 0.0, 5.0),
        )),
        23 => emit(P08MidiUpdate::Param(
            ParamId::FilterEgAttack,
            ranged(raw, 127, 0.0005, 5.0),
        )),
        24 => emit(P08MidiUpdate::Param(
            ParamId::FilterEgDecay,
            ranged(raw, 127, 0.0005, 5.0),
        )),
        25 => emit(P08MidiUpdate::Param(
            ParamId::FilterEgSustain,
            unit(raw, 127),
        )),
        26 => emit(P08MidiUpdate::Param(
            ParamId::FilterEgRelease,
            ranged(raw, 127, 0.0005, 10.0),
        )),
        27 => emit(P08MidiUpdate::Param(
            ParamId::VcaInitialLevel,
            unit(raw, 127),
        )),
        28 => emit(P08MidiUpdate::Param(ParamId::PanSpread, unit(raw, 127))),
        29 => emit(P08MidiUpdate::Param(ParamId::MasterVolume, unit(raw, 127))),
        30 => emit(P08MidiUpdate::Param(ParamId::AmpEnvAmount, unit(raw, 127))),
        31 => emit(P08MidiUpdate::Param(ParamId::AmpVelocity, unit(raw, 127))),
        32 => emit(P08MidiUpdate::Param(
            ParamId::AmpEgDelay,
            ranged(raw, 127, 0.0, 5.0),
        )),
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
            p08_mod_destination(raw).index() as f32,
        )),
        58 => emit(P08MidiUpdate::Param(
            ParamId::AuxEgAmount,
            bipolar(raw, 254),
        )),
        59 => emit(P08MidiUpdate::Param(ParamId::AuxEgVelocity, unit(raw, 127))),
        60 => emit(P08MidiUpdate::Param(
            ParamId::AuxEgDelay,
            ranged(raw, 127, 0.0, 5.0),
        )),
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
        95 => emit(P08MidiUpdate::Param(
            ParamId::KeyMode,
            f32::from(raw.min(5)),
        )),
        96 => {
            let (mode, detune) = match raw.min(4) {
                0 => (crate::UnisonMode::V1, 0.0),
                1 => (crate::UnisonMode::V8, 0.0),
                2 => (crate::UnisonMode::V8, 16.0 / 3.0),
                3 => (crate::UnisonMode::V8, 32.0 / 3.0),
                _ => (crate::UnisonMode::V8, 16.0),
            };
            emit(P08MidiUpdate::Param(
                ParamId::UnisonMode,
                mode.index() as f32,
            ));
            emit(P08MidiUpdate::Param(ParamId::UnisonDetune, detune));
        }
        99 => emit(P08MidiUpdate::Param(
            ParamId::UnisonEnabled,
            f32::from(raw != 0),
        )),
        100 => emit(P08MidiUpdate::Param(
            ParamId::PitchBendRange,
            f32::from(raw.min(12)),
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
    let clock_sync = [
        ParamId::Lfo1ClockSync,
        ParamId::Lfo2ClockSync,
        ParamId::Lfo3ClockSync,
        ParamId::Lfo4ClockSync,
    ];
    let sync_division = [
        ParamId::Lfo1SyncDivision,
        ParamId::Lfo2SyncDivision,
        ParamId::Lfo3SyncDivision,
        ParamId::Lfo4SyncDivision,
    ];
    match field {
        0 => {
            let synced = raw > 150;
            emit(P08MidiUpdate::Param(clock_sync[lfo], f32::from(synced)));
            if synced {
                emit(P08MidiUpdate::Param(
                    sync_division[lfo],
                    LfoSyncDivision::from_p08_raw(raw).index() as f32,
                ));
            } else {
                emit(P08MidiUpdate::Param(params[lfo][0], p08_lfo_rate_hz(raw)));
            }
        }
        1 => emit(P08MidiUpdate::Param(params[lfo][1], p08_lfo_waveform(raw))),
        2 => emit(P08MidiUpdate::Param(params[lfo][2], unit(raw, 127))),
        3 => emit(P08MidiUpdate::Param(
            params[lfo][3],
            p08_mod_destination(raw).index() as f32,
        )),
        _ => emit(P08MidiUpdate::Param(params[lfo][4], f32::from(raw != 0))),
    }
}

fn map_free_mod_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(P08MidiUpdate)) {
    let index = usize::from((number - 65) / 3);
    let parameter = match (number - 65) % 3 {
        0 => ModulationParam::Source(ModSource::from_index(usize::from(raw.min(20)))),
        1 => ModulationParam::Amount(bipolar(raw, 254)),
        _ => ModulationParam::Destination(p08_mod_destination(raw)),
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
        ModulationParam::Destination(p08_mod_destination(raw))
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
        4 | 9 | 10 | 19 | 41 | 46 | 51 | 56 => 1,
        12 => 5,
        15 => 164,
        20 | 58 | 66 | 69 | 72 | 75 | 81 | 83 | 85 | 87 | 89 => 254,
        37 | 42 | 47 | 52 => 166,
        38 | 43 | 48 | 53 => 4,
        40 | 45 | 50 | 55 | 57 => 43,
        100 => 12,
        _ => 127,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::WideF32;
    use crate::{ControlMessage, VoiceManager};

    fn render_frames<const PACKS: usize>(voices: &mut VoiceManager<PACKS>, frames: usize) {
        let mut ctx = crate::create_render_context!();
        for _ in 0..frames {
            voices.next(&mut ctx);
        }
    }

    const FACTORY_SYSEX: &[u8] =
        include_bytes!("../../../Prophet_08_Programs+ReadMe/Prophet_08_Programs_v1.0.syx");

    fn factory_message(bank: usize, program: usize) -> &'static [u8] {
        let offset = (bank * 128 + program) * P08_PROGRAM_DATA_SYSEX_LEN;
        &FACTORY_SYSEX[offset..offset + P08_PROGRAM_DATA_SYSEX_LEN]
    }

    #[test]
    fn decode_patch_payload_reads_layer_a_name() {
        let decoded = P08MidiDecoder::program_data(factory_message(0, 0)).unwrap();
        assert_eq!(decoded.patch.name.as_str(), "Wagnerian");

        let decoded = P08MidiDecoder::program_data(factory_message(0, 1)).unwrap();
        assert_eq!(decoded.patch.name.as_str(), "Tom Sawyer");
        assert!(decoded.patch.unison_enabled);
        assert_eq!(decoded.patch.unison_mode, crate::UnisonMode::V8);
        assert_eq!(decoded.patch.key_mode, crate::KeyMode::HighRetrigger);
        assert!(decoded.patch.glide_enabled);
        assert_eq!(decoded.patch.glide_mode, crate::GlideMode::FixedRate);
        assert!((decoded.patch.amplifier.eg_release - 9.291_374).abs() < 0.001);

        let decoded = P08MidiDecoder::program_data(factory_message(1, 0)).unwrap();
        assert_eq!(decoded.patch.name.as_str(), "AnalogWurlyRoids");
    }

    #[test]
    fn tom_sawyer_unison_glide_releases_every_ordered_note_sequence() {
        let patch = P08MidiDecoder::program_data(factory_message(0, 1))
            .unwrap()
            .patch;
        assert_eq!(patch.name.as_str(), "Tom Sawyer");
        assert!(patch.unison_enabled && patch.glide_enabled);
        assert_eq!(patch.unison_mode, crate::UnisonMode::V8);
        assert_eq!(patch.key_mode, crate::KeyMode::HighRetrigger);

        let release_orders = [
            [48, 59, 72],
            [48, 72, 59],
            [59, 48, 72],
            [59, 72, 48],
            [72, 48, 59],
            [72, 59, 48],
        ];
        for frames_between_events in [0, 1, 32, 256] {
            for release_order in release_orders {
                let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(48_000.0);
                voices.apply_patch(&patch);
                for note in [59, 48, 72] {
                    voices.handle_control(ControlMessage::NoteOn {
                        note,
                        velocity: 1.0,
                    });
                    render_frames(&mut voices, frames_between_events);
                }
                for note in release_order {
                    voices.handle_control(ControlMessage::NoteOff { note });
                    render_frames(&mut voices, frames_between_events);
                }

                assert!(
                    voices.active_notes().is_empty(),
                    "gate or pending note survived release order {release_order:?} with delay {frames_between_events}"
                );
            }
        }
    }

    #[test]
    fn tom_sawyer_patch_rebuild_cannot_resurrect_released_pending_notes() {
        let patch = P08MidiDecoder::program_data(factory_message(0, 1))
            .unwrap()
            .patch;
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(48_000.0);

        voices.handle_control(ControlMessage::NoteOn {
            note: 59,
            velocity: 1.0,
        });
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOff { note: 72 });
        voices.handle_control(ControlMessage::NoteOff { note: 59 });
        for _ in 0..512 {
            render_frames(&mut voices, 1);
        }

        assert!(voices.active_notes().is_empty());
    }

    #[test]
    #[cfg(not(feature = "fast-math"))]
    fn tom_sawyer_last_retrigger_glides_in_place_without_pending_voices() {
        let mut patch = P08MidiDecoder::program_data(factory_message(0, 1))
            .unwrap()
            .patch;
        patch.key_mode = crate::KeyMode::LastRetrigger;
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(48_000.0);
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        let before = voices[0].oscillators().osc1_frequency_hz().to_array()[0];

        voices.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });
        let start = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
        assert!(
            (start - before).abs() < 0.1,
            "glide should start from current pitch; before={before}, start={start}"
        );
        for voice in 0..8 {
            assert!(!voices[voice / WideF32::LANES].has_pending_note(voice % WideF32::LANES));
        }

        // Glide should make progress within the first 32 samples.
        for _ in 0..32 {
            render_frames(&mut voices, 1);
        }
        let progressing = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
        assert!(progressing > start);
        assert!(progressing < before * 2.0);

        // After the glide completes (well within 1 s) the frequency must be
        // stable — no further drift between consecutive reads.
        for _ in 0..48_000 {
            render_frames(&mut voices, 1);
        }
        let pre = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
        for _ in 0..1_000 {
            render_frames(&mut voices, 1);
        }
        let post = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
        assert!(
            (post - pre).abs() / pre < 1.0e-6,
            "frequency should be stable after glide completes; pre {pre}, post {post}"
        );
    }

    #[test]
    fn tom_sawyer_final_release_reaches_idle() {
        let patch = P08MidiDecoder::program_data(factory_message(0, 1))
            .unwrap()
            .patch;
        let mut voices = VoiceManager::<2>::new(48_000.0);
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        for _ in 0..48_000 {
            render_frames(&mut voices, 1);
        }
        voices.handle_control(ControlMessage::NoteOff { note: 60 });
        assert!(voices.active_notes().is_empty());
        assert!(
            voices.active_voice_count() > 0,
            "release tail should still render"
        );

        for _ in 0..480_000 {
            render_frames(&mut voices, 1);
        }
        assert_eq!(
            voices.active_voice_count(),
            0,
            "Tom Sawyer release envelope never reached idle"
        );
    }

    #[test]
    #[cfg(not(feature = "wide-1"))]
    fn tom_sawyer_unison_glide_matches_pressed_key_model_under_adversarial_ordering() {
        let base_patch = P08MidiDecoder::program_data(factory_message(0, 1))
            .unwrap()
            .patch;
        for key_mode in crate::KeyMode::ALL {
            for glide_mode in crate::GlideMode::ALL {
                let mut patch = base_patch.clone();
                patch.key_mode = key_mode;
                patch.glide_mode = glide_mode;
                let mut voices = VoiceManager::<2>::new(48_000.0);
                voices.apply_patch(&patch);
                let mut pressed = heapless::Vec::<u8, 128>::new();
                let mut random = 0x6d2b_79f5_u32;

                for event in 0..4_096 {
                    random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let note = 48 + ((random >> 16) % 25) as u8;
                    if random & 1 == 0 {
                        if let Some(index) = pressed.iter().position(|held| *held == note) {
                            pressed.remove(index);
                        }
                        pressed.push(note).unwrap();
                        voices.handle_control(ControlMessage::NoteOn {
                            note,
                            velocity: 1.0,
                        });
                    } else {
                        if let Some(index) = pressed.iter().position(|held| *held == note) {
                            pressed.remove(index);
                        }
                        voices.handle_control(ControlMessage::NoteOff { note });
                    }
                    for _ in 0..((random >> 8) & 3) {
                        render_frames(&mut voices, 1);
                    }

                    let active = voices.active_notes();
                    if pressed.is_empty() {
                        assert!(
                            active.is_empty(),
                            "{key_mode:?}/{glide_mode:?} event {event}: unpressed note remained gated or pending"
                        );
                        continue;
                    }
                    let selected = match key_mode {
                        crate::KeyMode::Low | crate::KeyMode::LowRetrigger => {
                            *pressed.iter().min().unwrap()
                        }
                        crate::KeyMode::High | crate::KeyMode::HighRetrigger => {
                            *pressed.iter().max().unwrap()
                        }
                        crate::KeyMode::Last | crate::KeyMode::LastRetrigger => {
                            *pressed.last().unwrap()
                        }
                    };
                    assert_eq!(
                        active.len(),
                        8,
                        "{key_mode:?}/{glide_mode:?} event {event}: incomplete unison group"
                    );
                    assert!(
                        active.iter().all(|note| note == selected),
                        "{key_mode:?}/{glide_mode:?} event {event}: selected {selected}, active {active:?}, pressed={pressed:?}"
                    );
                }

                voices.handle_control(ControlMessage::AllNotesOff);
                assert!(voices.active_notes().is_empty());
            }
        }
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
    fn factory_program_decodes_vca_initial_level() {
        let decoded = P08MidiDecoder::program_data(factory_message(0, 54)).unwrap();
        assert!(
            (decoded.patch.amplifier.initial_level - 103.0 / 127.0).abs() < 0.01,
            "decoded {}",
            decoded.patch.amplifier.initial_level
        );
    }

    #[test]
    fn factory_program_pan_spread_uses_documented_program_indices() {
        let message = factory_message(0, 0);
        let mut raw = [0_u8; P08_PROGRAM_DATA_LEN];
        unpack_program_data(&message[6..6 + P08_PROGRAM_PACKED_LEN], &mut raw);
        let decoded = P08MidiDecoder::program_data(message).unwrap();

        assert_eq!(raw[28], 0, "factory fixture layer A Pan Spread changed");
        assert_eq!(raw[228], 49, "factory fixture layer B Pan Spread changed");
        assert_eq!(
            decoded.patch.amplifier.pan_spread,
            f32::from(raw[28]) / 127.0
        );
        assert_eq!(
            decoded.patch.amplifier.pan_mod_mode,
            crate::PanModMode::Alternate
        );
    }

    #[test]
    fn program_values_above_127_use_the_documented_msb_sideband() {
        let mut raw = [0_u8; P08_PROGRAM_DATA_LEN];
        raw[20] = 1;
        raw[14] = 0x80;
        assert_eq!(program_nrpn_value(&raw, 20), Some(129));
    }

    #[test]
    fn oscillator_shape_uses_p08_waveform_order() {
        let mut updates = [None; 3];
        let mut len = 0;
        emit_osc_shape(
            &mut |update| {
                updates[len] = Some(update);
                len += 1;
            },
            true,
            2,
        );
        assert_eq!(len, 3);
        assert_eq!(
            updates,
            [
                Some(P08MidiUpdate::Param(ParamId::Osc1Enabled, 1.0)),
                Some(P08MidiUpdate::Param(ParamId::Osc1Waveform, 2.0)),
                Some(P08MidiUpdate::Param(ParamId::Osc1ShapeMod, 0.0)),
            ]
        );

        let mut updates = [None; 3];
        let mut len = 0;
        emit_osc_shape(
            &mut |update| {
                updates[len] = Some(update);
                len += 1;
            },
            true,
            3,
        );
        assert_eq!(len, 3);
        assert_eq!(
            updates,
            [
                Some(P08MidiUpdate::Param(ParamId::Osc1Enabled, 1.0)),
                Some(P08MidiUpdate::Param(ParamId::Osc1Waveform, 1.0)),
                Some(P08MidiUpdate::Param(ParamId::Osc1ShapeMod, 0.0)),
            ]
        );
    }

    #[test]
    fn oscillator_keyboard_parameters_map_for_both_oscillators() {
        for (number, raw, expected) in [
            (4, 0, P08MidiUpdate::Param(ParamId::Osc1KeyboardOn, 0.0)),
            (4, 1, P08MidiUpdate::Param(ParamId::Osc1KeyboardOn, 1.0)),
            (9, 0, P08MidiUpdate::Param(ParamId::Osc2KeyboardOn, 0.0)),
            (9, 1, P08MidiUpdate::Param(ParamId::Osc2KeyboardOn, 1.0)),
        ] {
            let mut update = None;
            map_nrpn(number, raw, &mut |value| update = Some(value));
            assert_eq!(update, Some(expected));
            assert_eq!(nrpn_max(number), Some(1));
        }
    }

    #[test]
    fn glide_parameters_map_to_the_shared_patch_model() {
        for (number, raw, expected) in [
            (
                3,
                64,
                P08MidiUpdate::Param(ParamId::Osc1Glide, 64.0 / 127.0),
            ),
            (8, 127, P08MidiUpdate::Param(ParamId::Osc2Glide, 1.0)),
            (11, 3, P08MidiUpdate::Param(ParamId::GlideMode, 3.0)),
        ] {
            let mut update = None;
            map_nrpn(number, raw, &mut |value| update = Some(value));
            assert_eq!(update, Some(expected));
        }

        let decoded = (0..256)
            .map(|program| {
                P08MidiDecoder::program_data(factory_message(program / 128, program % 128)).unwrap()
            })
            .find(|program| program.patch.osc1.glide > 0.0 || program.patch.osc2.glide > 0.0)
            .expect("factory bank should contain a glide program");
        assert!(decoded.patch.glide_enabled);
    }

    #[test]
    fn p08_unison_modes_translate_to_rev2_first_patch_values() {
        let mut updates = [None; 2];
        let mut len = 0;
        map_nrpn(96, 4, &mut |update| {
            updates[len] = Some(update);
            len += 1;
        });
        assert_eq!(len, 2);
        assert_eq!(
            updates,
            [
                Some(P08MidiUpdate::Param(
                    ParamId::UnisonMode,
                    crate::UnisonMode::V8.index() as f32,
                )),
                Some(P08MidiUpdate::Param(ParamId::UnisonDetune, 16.0)),
            ]
        );

        let mut update = None;
        map_nrpn(95, 3, &mut |value| update = Some(value));
        assert_eq!(
            update,
            Some(P08MidiUpdate::Param(
                ParamId::KeyMode,
                crate::KeyMode::HighRetrigger.index() as f32,
            ))
        );
    }

    #[test]
    fn lfo_waveform_uses_p08_shape_order() {
        assert_eq!(p08_lfo_waveform(1), 2.0);
        assert_eq!(p08_lfo_waveform(2), 1.0);
    }

    #[test]
    fn combined_shape_raw_value_decodes_pulse_and_shape_mod() {
        let mut updates = [None; 3];
        let mut len = 0;
        emit_osc_shape(
            &mut |update| {
                updates[len] = Some(update);
                len += 1;
            },
            true,
            54,
        );
        assert_eq!(len, 3);
        assert_eq!(
            updates,
            [
                Some(P08MidiUpdate::Param(ParamId::Osc1Enabled, 1.0)),
                Some(P08MidiUpdate::Param(ParamId::Osc1Waveform, 3.0)),
                Some(P08MidiUpdate::Param(ParamId::Osc1ShapeMod, 50.0 / 99.0)),
            ]
        );
    }

    #[test]
    fn mod_destination_maps_p08_indices_to_internal_destinations() {
        assert_eq!(p08_mod_destination(3), ModDestination::OscAllFrequency);
        assert_eq!(p08_mod_destination(4), ModDestination::OscMix);
        assert_eq!(p08_mod_destination(9), ModDestination::FilterCutoff);
        assert_eq!(p08_mod_destination(25), ModDestination::AmpEnvAmount);
    }

    #[test]
    fn p08_lfo_rate_decodes_free_rate_and_all_sync_divisions() {
        assert!(p08_lfo_rate_hz(37) > MIN_LFO_RATE_HZ);
        for raw in 151..=166 {
            let division = LfoSyncDivision::from_p08_raw(raw);
            assert_eq!(division.p08_raw(), raw);
            let mut updates = [None; 2];
            let mut len = 0;
            map_lfo_nrpn(37, raw, &mut |update| {
                updates[len] = Some(update);
                len += 1;
            });
            assert_eq!(
                updates,
                [
                    Some(P08MidiUpdate::Param(ParamId::Lfo1ClockSync, 1.0)),
                    Some(P08MidiUpdate::Param(
                        ParamId::Lfo1SyncDivision,
                        division.index() as f32,
                    )),
                ]
            );
        }
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
