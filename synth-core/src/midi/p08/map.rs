//! Prophet '08 NRPN parameter maps and program-field layout.

use crate::{
    DedicatedModSource, LfoSyncDivision, ModDestination, ModRoute, ModSource, ModulationParam,
    ParamId,
    dsp::{MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ},
    math::F32,
    midi::{
        p08::ids::*,
        prophet::{
            FILTER_CUTOFF_RAW_MAX, attack_decay_seconds, cutoff_raw_to_hz, key_track_from_raw,
            release_seconds,
        },
    },
};

const P08_LFO_FREE_RATE_MAX: u16 = 150;

#[derive(Clone, Copy)]
pub(super) struct ProgramField {
    value_offset: usize,
    msb_offset: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum MidiUpdate {
    Param(ParamId, f32),
    Modulation {
        route: ModRoute,
        parameter: ModulationParam,
    },
}

/// Program-field and borrowed-MSB offsets from the
/// [Prophet '08 User's Guide, Program Parameter Data](https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf).
pub(super) fn program_field(number: u16, layer_offset: usize) -> Option<ProgramField> {
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
        value_offset: value_offset + layer_offset,
        msb_offset: msb_offset.map(|offset| offset + layer_offset),
    })
}

pub(super) fn program_nrpn_value(raw: &[u8], number: u16, layer_offset: usize) -> Option<u16> {
    let field = program_field(number, layer_offset)?;
    let value = *raw.get(field.value_offset)?;
    if let Some(msb_offset) = field.msb_offset {
        Some(u16::from(value & 0x7f) | u16::from(*raw.get(msb_offset)? & 0x80))
    } else if nrpn_max(number).is_some_and(|maximum| maximum > 127) {
        Some(u16::from(value))
    } else {
        Some(u16::from(value & 0x7f))
    }
}

pub(super) fn unit(raw: u16, max: u16) -> f32 {
    f32::from(raw.min(max)) / f32::from(max)
}

pub(super) fn bipolar(raw: u16, max: u16) -> f32 {
    unit(raw, max) * 2.0 - 1.0
}

pub(super) fn ranged(raw: u16, raw_max: u16, min: f32, max: f32) -> f32 {
    min + unit(raw, raw_max) * (max - min)
}

pub(super) fn logarithmic(raw: u16, raw_max: u16, min: f32, max: f32) -> f32 {
    min * F32(max / min).powf(F32(unit(raw, raw_max))).as_f32()
}

/// Raw destination order from the
/// [Prophet '08 User's Guide, Modulation Destinations](https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf).
const MOD_DESTINATIONS: [ModDestination; 44] = [
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

pub(super) fn p08_mod_destination(raw: u16) -> ModDestination {
    MOD_DESTINATIONS
        .get(usize::from(raw.min(43)))
        .copied()
        .unwrap_or(ModDestination::Off)
}

pub(super) fn p08_lfo_waveform(raw: u16) -> f32 {
    match raw.min(4) {
        0 => 0.0,
        1 => 2.0,
        2 => 1.0,
        3 => 3.0,
        4 => 4.0,
        _ => 0.0,
    }
}

pub(super) fn p08_lfo_rate_hz(raw: u16) -> f32 {
    logarithmic(
        raw.min(P08_LFO_FREE_RATE_MAX),
        P08_LFO_FREE_RATE_MAX,
        MIN_LFO_RATE_HZ,
        MAX_LFO_RATE_HZ,
    )
}

pub(super) fn emit_osc_shape(emit: &mut impl FnMut(MidiUpdate), osc1: bool, raw: u16) {
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
    emit(MidiUpdate::Param(enabled_param, f32::from(raw != 0)));
    if raw == 0 {
        return;
    }
    match raw {
        1 => {
            emit(MidiUpdate::Param(waveform_param, 0.0));
            emit(MidiUpdate::Param(shape_param, 0.0));
        }
        2 => {
            emit(MidiUpdate::Param(waveform_param, 2.0));
            emit(MidiUpdate::Param(shape_param, 0.0));
        }
        3 => {
            emit(MidiUpdate::Param(waveform_param, 1.0));
            emit(MidiUpdate::Param(shape_param, 0.0));
        }
        _ => {
            emit(MidiUpdate::Param(waveform_param, 3.0));
            emit(MidiUpdate::Param(
                shape_param,
                unit(raw.saturating_sub(4), 99),
            ));
        }
    }
}

pub(super) fn map_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(MidiUpdate)) {
    match number {
        NRPN_OSC1_FREQUENCY => emit(MidiUpdate::Param(
            ParamId::Osc1Frequency,
            f32::from(raw.min(120)),
        )),
        NRPN_OSC1_FINE_TUNE => emit(MidiUpdate::Param(
            ParamId::Osc1FineTune,
            f32::from(raw.min(100)) - 50.0,
        )),
        NRPN_OSC1_SHAPE => emit_osc_shape(emit, true, raw.min(103)),
        NRPN_OSC1_GLIDE => emit(MidiUpdate::Param(ParamId::Osc1Glide, unit(raw, 127))),
        NRPN_OSC1_KEYBOARD => emit(MidiUpdate::Param(
            ParamId::Osc1KeyboardOn,
            f32::from(raw != 0),
        )),
        NRPN_OSC2_FREQUENCY => emit(MidiUpdate::Param(
            ParamId::Osc2Frequency,
            f32::from(raw.min(120)),
        )),
        NRPN_OSC2_FINE_TUNE => emit(MidiUpdate::Param(
            ParamId::Osc2FineTune,
            f32::from(raw.min(100)) - 50.0,
        )),
        NRPN_OSC2_SHAPE => emit_osc_shape(emit, false, raw.min(103)),
        NRPN_OSC2_GLIDE => emit(MidiUpdate::Param(ParamId::Osc2Glide, unit(raw, 127))),
        NRPN_OSC2_KEYBOARD => emit(MidiUpdate::Param(
            ParamId::Osc2KeyboardOn,
            f32::from(raw != 0),
        )),
        NRPN_HARD_SYNC => emit(MidiUpdate::Param(ParamId::HardSync, f32::from(raw != 0))),
        NRPN_GLIDE_MODE => emit(MidiUpdate::Param(ParamId::GlideMode, f32::from(raw.min(3)))),
        NRPN_OSC_SLOP => emit(MidiUpdate::Param(ParamId::OscSlop, unit(raw, 5))),
        NRPN_OSC_MIX => emit(MidiUpdate::Param(ParamId::OscMix, unit(raw, 127))),
        NRPN_NOISE_LEVEL => emit(MidiUpdate::Param(ParamId::NoiseLevel, unit(raw, 127))),
        NRPN_FILTER_CUTOFF => emit(MidiUpdate::Param(
            ParamId::FilterCutoff,
            cutoff_raw_to_hz(raw.min(FILTER_CUTOFF_RAW_MAX)),
        )),
        NRPN_FILTER_RESONANCE => emit(MidiUpdate::Param(ParamId::FilterResonance, unit(raw, 127))),
        NRPN_FILTER_KEY_TRACK => emit(MidiUpdate::Param(
            ParamId::FilterKeyTrack,
            key_track_from_raw(raw),
        )),
        NRPN_FILTER_AUDIO_MOD => emit(MidiUpdate::Param(ParamId::FilterAudioMod, unit(raw, 127))),
        NRPN_FILTER_POLES => emit(MidiUpdate::Param(ParamId::FilterPoles, f32::from(raw != 0))),
        NRPN_FILTER_ENV_AMOUNT => emit(MidiUpdate::Param(
            ParamId::FilterEnvAmount,
            bipolar(raw, 254),
        )),
        NRPN_FILTER_VELOCITY => emit(MidiUpdate::Param(ParamId::FilterVelocity, unit(raw, 127))),
        NRPN_FILTER_EG_DELAY => emit(MidiUpdate::Param(
            ParamId::FilterEgDelay,
            ranged(raw, 127, 0.0, 5.0),
        )),
        NRPN_FILTER_EG_ATTACK => emit(MidiUpdate::Param(
            ParamId::FilterEgAttack,
            attack_decay_seconds(raw),
        )),
        NRPN_FILTER_EG_DECAY => emit(MidiUpdate::Param(
            ParamId::FilterEgDecay,
            attack_decay_seconds(raw),
        )),
        NRPN_FILTER_EG_SUSTAIN => emit(MidiUpdate::Param(ParamId::FilterEgSustain, unit(raw, 127))),
        NRPN_FILTER_EG_RELEASE => emit(MidiUpdate::Param(
            ParamId::FilterEgRelease,
            release_seconds(raw),
        )),
        NRPN_VCA_INITIAL_LEVEL => emit(MidiUpdate::Param(ParamId::VcaInitialLevel, unit(raw, 127))),
        NRPN_PAN_SPREAD => emit(MidiUpdate::Param(ParamId::PanSpread, unit(raw, 127))),
        NRPN_PROGRAM_VOLUME => emit(MidiUpdate::Param(ParamId::ProgramVolume, unit(raw, 127))),
        NRPN_AMP_ENV_AMOUNT => emit(MidiUpdate::Param(ParamId::AmpEnvAmount, unit(raw, 127))),
        NRPN_AMP_VELOCITY => emit(MidiUpdate::Param(ParamId::AmpVelocity, unit(raw, 127))),
        NRPN_AMP_EG_DELAY => emit(MidiUpdate::Param(
            ParamId::AmpEgDelay,
            ranged(raw, 127, 0.0, 5.0),
        )),
        NRPN_AMP_EG_ATTACK => emit(MidiUpdate::Param(
            ParamId::AmpEgAttack,
            attack_decay_seconds(raw),
        )),
        NRPN_AMP_EG_DECAY => emit(MidiUpdate::Param(
            ParamId::AmpEgDecay,
            attack_decay_seconds(raw),
        )),
        NRPN_AMP_EG_SUSTAIN => emit(MidiUpdate::Param(ParamId::AmpEgSustain, unit(raw, 127))),
        NRPN_AMP_EG_RELEASE => emit(MidiUpdate::Param(
            ParamId::AmpEgRelease,
            release_seconds(raw),
        )),
        NRPN_LFO_BLOCK_START..=NRPN_LFO_BLOCK_END => map_lfo_nrpn(number, raw, emit),
        NRPN_AUX_EG_DESTINATION => emit(MidiUpdate::Param(
            ParamId::AuxEgDestination,
            p08_mod_destination(raw).index() as f32,
        )),
        NRPN_AUX_EG_AMOUNT => emit(MidiUpdate::Param(ParamId::AuxEgAmount, bipolar(raw, 254))),
        NRPN_AUX_EG_VELOCITY => emit(MidiUpdate::Param(ParamId::AuxEgVelocity, unit(raw, 127))),
        NRPN_AUX_EG_DELAY => emit(MidiUpdate::Param(
            ParamId::AuxEgDelay,
            ranged(raw, 127, 0.0, 5.0),
        )),
        NRPN_AUX_EG_ATTACK => emit(MidiUpdate::Param(
            ParamId::AuxEgAttack,
            attack_decay_seconds(raw),
        )),
        NRPN_AUX_EG_DECAY => emit(MidiUpdate::Param(
            ParamId::AuxEgDecay,
            attack_decay_seconds(raw),
        )),
        NRPN_AUX_EG_SUSTAIN => emit(MidiUpdate::Param(ParamId::AuxEgSustain, unit(raw, 127))),
        NRPN_AUX_EG_RELEASE => emit(MidiUpdate::Param(
            ParamId::AuxEgRelease,
            release_seconds(raw),
        )),
        NRPN_KEY_MODE => emit(MidiUpdate::Param(ParamId::KeyMode, f32::from(raw.min(5)))),
        NRPN_UNISON_MODE => {
            let (mode, detune) = match raw.min(4) {
                0 => (crate::UnisonMode::V1, 0.0),
                1 => (crate::UnisonMode::V8, 0.0),
                2 => (crate::UnisonMode::V8, 16.0 / 3.0),
                3 => (crate::UnisonMode::V8, 32.0 / 3.0),
                _ => (crate::UnisonMode::V8, 16.0),
            };
            emit(MidiUpdate::Param(ParamId::UnisonMode, mode.index() as f32));
            emit(MidiUpdate::Param(ParamId::UnisonDetune, detune));
        }
        NRPN_UNISON_ENABLED => emit(MidiUpdate::Param(
            ParamId::UnisonEnabled,
            f32::from(raw != 0),
        )),
        NRPN_PITCH_BEND_RANGE => emit(MidiUpdate::Param(
            ParamId::PitchBendRange,
            f32::from(raw.min(12)),
        )),
        NRPN_FREE_MOD_START..=NRPN_FREE_MOD_END => map_free_mod_nrpn(number, raw, emit),
        NRPN_DEDICATED_MOD_START..=NRPN_DEDICATED_MOD_END => {
            map_dedicated_mod_nrpn(number, raw, emit)
        }
        _ => {}
    }
}

pub(super) fn map_lfo_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(MidiUpdate)) {
    let lfo = usize::from((number - NRPN_LFO_BLOCK_START) / NRPN_LFO_FIELD_COUNT);
    let field = (number - NRPN_LFO_BLOCK_START) % NRPN_LFO_FIELD_COUNT;
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
            let synced = raw > P08_LFO_FREE_RATE_MAX;
            emit(MidiUpdate::Param(clock_sync[lfo], f32::from(synced)));
            if synced {
                emit(MidiUpdate::Param(
                    sync_division[lfo],
                    LfoSyncDivision::from_p08_raw(raw).index() as f32,
                ));
            } else {
                emit(MidiUpdate::Param(params[lfo][0], p08_lfo_rate_hz(raw)));
            }
        }
        1 => emit(MidiUpdate::Param(params[lfo][1], p08_lfo_waveform(raw))),
        2 => emit(MidiUpdate::Param(params[lfo][2], unit(raw, 127))),
        3 => emit(MidiUpdate::Param(
            params[lfo][3],
            p08_mod_destination(raw).index() as f32,
        )),
        _ => emit(MidiUpdate::Param(params[lfo][4], f32::from(raw != 0))),
    }
}

pub(super) fn map_free_mod_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(MidiUpdate)) {
    let index = usize::from((number - NRPN_FREE_MOD_START) / 3);
    let parameter = match (number - NRPN_FREE_MOD_START) % 3 {
        0 => ModulationParam::Source(ModSource::from_index(usize::from(raw.min(20)))),
        1 => ModulationParam::Amount(bipolar(raw, 254)),
        _ => ModulationParam::Destination(p08_mod_destination(raw)),
    };
    emit(MidiUpdate::Modulation {
        route: ModRoute::Free(index),
        parameter,
    });
}

pub(super) fn map_dedicated_mod_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(MidiUpdate)) {
    let index = usize::from((number - NRPN_DEDICATED_MOD_START) / 2);
    let Some(source) = DedicatedModSource::ALL.get(index) else {
        return;
    };
    let parameter = if (number - NRPN_DEDICATED_MOD_START) % 2 == 0 {
        ModulationParam::Amount(bipolar(raw, 254))
    } else {
        ModulationParam::Destination(p08_mod_destination(raw))
    };
    emit(MidiUpdate::Modulation {
        route: ModRoute::Dedicated(*source),
        parameter,
    });
}

pub(super) fn nrpn_max(number: u16) -> Option<u16> {
    Some(match number {
        NRPN_OSC1_FREQUENCY | NRPN_OSC2_FREQUENCY => 120,
        NRPN_OSC1_FINE_TUNE | NRPN_OSC2_FINE_TUNE => 100,
        NRPN_OSC1_SHAPE | NRPN_OSC2_SHAPE => 103,
        NRPN_OSC1_KEYBOARD | 9 | 10 | 19 | 41 | 46 | 51 | 56 => 1,
        NRPN_OSC_SLOP => 5,
        NRPN_FILTER_CUTOFF => 164,
        NRPN_FILTER_ENV_AMOUNT | 58 | 66 | 69 | 72 | 75 | 81 | 83 | 85 | 87 | 89 => 254,
        37 | 42 | 47 | 52 => 166,
        38 | 43 | 48 | 53 => 4,
        40 | 45 | 50 | 55 | NRPN_AUX_EG_DESTINATION => 43,
        NRPN_PITCH_BEND_RANGE => 12,
        _ => 127,
    })
}
