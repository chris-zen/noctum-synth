//! Shared Rev2 NRPN/CC parameter maps and program-image field layout.

use crate::dsp::{MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
use crate::math::F32;
use crate::midi::clock::MidiClockMode;
use crate::midi::prophet::{
    FILTER_CUTOFF_RAW_MAX, attack_decay_seconds, cutoff_raw_to_hz, key_track_from_raw,
    release_seconds,
};
use crate::{
    DedicatedModSource, LfoSyncDivision, ModDestination, ModRoute, ModSource, ModulationParam,
    ParamId,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum MappedUpdate {
    Param(ParamId, f32),
    MidiClockMode(MidiClockMode),
    Modulation {
        route: ModRoute,
        parameter: ModulationParam,
    },
}

#[derive(Clone, Copy, Default)]
pub(super) struct LfoPairingState {
    lfo_rate_raw: [Option<u16>; 4],
    lfo_clock_sync: [bool; 4],
}

pub(super) struct ProgramField {
    value_offset: usize,
    msb_offset: Option<usize>,
}

pub(super) fn program_field(number: u16, layer_offset: usize) -> Option<ProgramField> {
    // Appendix E documents the transport packing but not the internal program
    // image. These offsets are the Rev2 program-image layout, verified against
    // Sequential's v1.0 factory bank. They are intentionally not NRPN indexes.
    const OFFSETS_0_TO_26: [usize; 27] = [
        0, 2, 4, 8, 10, 1, 3, 5, 9, 11, 17, 18, 21, 14, 16, 22, 23, 24, 25, 26, 32, 35, 38, 41, 44,
        47, 50,
    ];
    const OFFSETS_28_TO_64: [usize; 37] = [
        29, 28, 33, 36, 39, 42, 45, 48, 51, 53, 57, 61, 65, 69, 54, 58, 62, 66, 70, 55, 59, 63, 67,
        71, 56, 60, 64, 68, 72, 30, 34, 37, 40, 43, 46, 49, 52,
    ];

    let value_offset = match number {
        0..=26 => OFFSETS_0_TO_26[number as usize],
        28..=64 => OFFSETS_28_TO_64[(number - 28) as usize],
        65..=88 => {
            let route = usize::from((number - 65) / 3);
            match (number - 65) % 3 {
                0 => 77 + route,
                1 => 85 + route,
                _ => 93 + route,
            }
        }
        97 => 31,
        99 => 12,
        100 => 20,
        102 => 6,
        103 => 7,
        104 => 13,
        105..=108 => 73 + usize::from(number - 105),
        110 => 15,
        111 => 19,
        116..=125 => 101 + usize::from(number - 116),
        153 => 116,
        154 => 115,
        155 => 117,
        156 => 118,
        157 => 119,
        158 => 120,
        167 => 208,
        168 => 123,
        169 => 124,
        170 => 122,
        172 => 136,
        173 => 132,
        174 => 133,
        177 => 134,
        178 => 135,
        175 => 131,
        179 => 130,
        _ => return None,
    };
    let msb_offset = match number {
        20 => Some(30),
        58 => Some(28),
        66 => Some(89),
        69 => Some(88),
        72 => Some(87),
        75 => Some(86),
        78 => Some(85),
        81 => Some(84),
        84 => Some(97),
        87 => Some(96),
        116 => Some(101),
        118 => Some(99),
        120 => Some(111),
        122 => Some(109),
        124 => Some(107),
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

pub(super) fn store_program_nrpn(raw: &mut [u8], number: u16, value: u16, layer_offset: usize) {
    let Some(field) = program_field(number, layer_offset) else {
        return;
    };
    if field.value_offset >= raw.len() {
        return;
    }
    if let Some(msb_offset) = field.msb_offset {
        if msb_offset >= raw.len() {
            return;
        }
        raw[field.value_offset] = (raw[field.value_offset] & 0x80) | value as u8 & 0x7f;
        raw[msb_offset] = (raw[msb_offset] & 0x7f) | value as u8 & 0x80;
    } else if nrpn_max(number).is_some_and(|maximum| maximum > 127) {
        raw[field.value_offset] = value as u8;
    } else {
        raw[field.value_offset] = (raw[field.value_offset] & 0x80) | value as u8 & 0x7f;
    }
}

pub(super) fn store_nrpn(raw: &mut [u8], messages: &[[u8; 3]], layer_offset: usize) {
    if messages.len() != 4 {
        return;
    }
    let number = usize::from(messages[0][2]) * 128 + usize::from(messages[1][2]);
    let value = u16::from(messages[2][2]) * 128 + u16::from(messages[3][2]);
    store_program_nrpn(raw, number as u16, value.min(255), layer_offset);
}

pub(super) fn emit_nrpn(channel: u8, number: u16, value: u16, emit: &mut impl FnMut([u8; 3])) {
    let status = 0xb0 | (channel & 0x0f);
    emit([status, 99, ((number / 128) & 0x7f) as u8]);
    emit([status, 98, (number & 0x7f) as u8]);
    emit([status, 6, ((value / 128) & 0x7f) as u8]);
    emit([status, 38, (value & 0x7f) as u8]);
}

pub(super) fn bool_raw(value: f32) -> u16 {
    u16::from(value >= 0.5)
}

pub(super) fn key_mode_raw(value: f32) -> u16 {
    match crate::KeyMode::from_index(value as usize) {
        crate::KeyMode::Low => 0,
        crate::KeyMode::High => 1,
        crate::KeyMode::Last => 2,
        crate::KeyMode::LowRetrigger => 3,
        crate::KeyMode::HighRetrigger => 4,
        crate::KeyMode::LastRetrigger => 5,
    }
}

pub(super) fn key_mode_index(raw: u16) -> f32 {
    (match raw.min(5) {
        0 => crate::KeyMode::Low.index(),
        1 => crate::KeyMode::High.index(),
        2 => crate::KeyMode::Last.index(),
        3 => crate::KeyMode::LowRetrigger.index(),
        4 => crate::KeyMode::HighRetrigger.index(),
        _ => crate::KeyMode::LastRetrigger.index(),
    }) as f32
}

pub(super) fn quantize(value: f32, min: f32, max: f32, raw_max: u16) -> u16 {
    F32((value.clamp(min, max) - min) / (max - min) * raw_max as f32)
        .round()
        .as_f32() as u16
}

pub(super) fn quantize_log(value: f32, min: f32, max: f32, raw_max: u16) -> u16 {
    let normalized = F32(value.clamp(min, max) / min).ln().as_f32() / F32(max / min).ln().as_f32();
    F32(normalized * raw_max as f32).round().as_f32() as u16
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

pub(super) fn emit_param(emit: &mut impl FnMut(MappedUpdate), param: ParamId, value: f32) {
    emit(MappedUpdate::Param(param, value));
}

pub(super) fn emit_osc_shape(emit: &mut impl FnMut(MappedUpdate), osc1: bool, raw: u16) {
    let (enabled_param, waveform_param) = if osc1 {
        (ParamId::Osc1Enabled, ParamId::Osc1Waveform)
    } else {
        (ParamId::Osc2Enabled, ParamId::Osc2Waveform)
    };
    emit_param(emit, enabled_param, f32::from(raw != 0));
    if raw != 0 {
        emit_param(
            emit,
            waveform_param,
            f32::from(raw.saturating_sub(1).min(3)),
        );
    }
}

pub(super) fn map_cc(controller: u8, raw: u8, emit: &mut impl FnMut(MappedUpdate)) -> bool {
    let raw = u16::from(raw);
    match controller {
        3 => emit_param(
            emit,
            ParamId::EffectType,
            F32(ranged(raw, 127, 0.0, 12.0)).round().as_f32(),
        ),
        5 => emit_param(emit, ParamId::GlideMode, f32::from(raw.min(3))),
        7 | 37 => emit_param(emit, ParamId::MasterVolume, unit(raw, 127)),
        8 => emit_param(emit, ParamId::SubOscLevel, unit(raw, 127)),
        9 => emit_param(emit, ParamId::OscSlop, unit(raw, 127)),
        10 => emit_param(emit, ParamId::PanModMode, f32::from(raw >= 64)),
        12 => emit_param(emit, ParamId::EffectParam1, unit(raw, 127)),
        13 => emit_param(emit, ParamId::EffectParam2, unit(raw, 127)),
        14 => emit_param(emit, ParamId::Bpm, f32::from(raw.clamp(30, 250))),
        15 => emit_param(emit, ParamId::ClockDivide, f32::from(raw.min(12))),
        16 => emit_param(emit, ParamId::EffectEnabled, f32::from(raw >= 64)),
        17 => emit_param(emit, ParamId::EffectMix, unit(raw, 127)),
        20 => emit_param(emit, ParamId::Osc1Frequency, f32::from(raw.min(120))),
        21 => emit_param(emit, ParamId::Osc1FineTune, ranged(raw, 127, -50.0, 50.0)),
        22 => emit_osc_shape(
            emit,
            true,
            F32(ranged(raw, 127, 0.0, 4.0)).round().as_f32() as u16,
        ),
        23 => emit_param(emit, ParamId::Osc1Glide, unit(raw, 127)),
        24 => emit_param(emit, ParamId::Osc2Frequency, f32::from(raw.min(120))),
        25 => emit_param(emit, ParamId::Osc2FineTune, ranged(raw, 127, -50.0, 50.0)),
        26 => emit_osc_shape(
            emit,
            false,
            F32(ranged(raw, 127, 0.0, 4.0)).round().as_f32() as u16,
        ),
        27 => emit_param(emit, ParamId::Osc2Glide, unit(raw, 127)),
        28 => emit_param(emit, ParamId::OscMix, unit(raw, 127)),
        29 => emit_param(emit, ParamId::NoiseLevel, unit(raw, 127)),
        30 => emit_param(emit, ParamId::Osc1ShapeMod, unit(raw, 127)),
        31 => emit_param(emit, ParamId::Osc2ShapeMod, unit(raw, 127)),
        65 => emit_param(emit, ParamId::GlideEnabled, f32::from(raw >= 64)),
        71 | 103 => emit_param(emit, ParamId::FilterResonance, unit(raw, 127)),
        74 | 102 => emit_param(emit, ParamId::FilterCutoff, cutoff_raw_to_hz(raw.min(127))),
        75 => emit_param(emit, ParamId::AmpEgSustain, unit(raw, 127)),
        76 => emit_param(emit, ParamId::AmpEgRelease, release_seconds(raw)),
        77 => emit_param(emit, ParamId::AuxEgSustain, unit(raw, 127)),
        78 => emit_param(emit, ParamId::AuxEgRelease, release_seconds(raw)),
        85 => emit_param(
            emit,
            ParamId::AuxEgDestination,
            F32(ranged(raw, 127, 0.0, 52.0)).round().as_f32(),
        ),
        86 => emit_param(emit, ParamId::AuxEgAmount, bipolar(raw, 127)),
        87 => emit_param(emit, ParamId::AuxEgVelocity, unit(raw, 127)),
        88 => emit_param(emit, ParamId::AuxEgDelay, ranged(raw, 127, 0.0, 5.0)),
        89 => emit_param(emit, ParamId::AuxEgAttack, attack_decay_seconds(raw)),
        90 => emit_param(emit, ParamId::AuxEgDecay, attack_decay_seconds(raw)),
        104 => emit_param(emit, ParamId::FilterKeyTrack, key_track_from_raw(raw)),
        105 => emit_param(emit, ParamId::FilterAudioMod, unit(raw, 127)),
        106 => emit_param(emit, ParamId::FilterEnvAmount, bipolar(raw, 127)),
        107 => emit_param(emit, ParamId::FilterVelocity, unit(raw, 127)),
        108 => emit_param(emit, ParamId::FilterEgDelay, ranged(raw, 127, 0.0, 5.0)),
        109 => emit_param(emit, ParamId::FilterEgAttack, attack_decay_seconds(raw)),
        110 => emit_param(emit, ParamId::FilterEgDecay, attack_decay_seconds(raw)),
        111 => emit_param(emit, ParamId::FilterEgSustain, unit(raw, 127)),
        112 => emit_param(emit, ParamId::FilterEgRelease, release_seconds(raw)),
        114 => emit_param(emit, ParamId::PanSpread, unit(raw, 127)),
        113 => emit_param(emit, ParamId::VcaInitialLevel, unit(raw, 127)),
        115 => emit_param(emit, ParamId::AmpEnvAmount, unit(raw, 127)),
        116 => emit_param(emit, ParamId::AmpVelocity, unit(raw, 127)),
        117 => emit_param(emit, ParamId::AmpEgDelay, ranged(raw, 127, 0.0, 5.0)),
        118 => emit_param(emit, ParamId::AmpEgAttack, attack_decay_seconds(raw)),
        119 => emit_param(emit, ParamId::AmpEgDecay, attack_decay_seconds(raw)),
        33 => emit_param(emit, ParamId::ArpEnabled, f32::from(raw >= 64)),
        34 => emit_param(emit, ParamId::ArpMode, f32::from(raw.min(4))),
        35 => emit_param(emit, ParamId::ArpRange, f32::from(raw.min(2))),
        36 => emit_param(emit, ParamId::ArpRepeats, f32::from(raw.min(2))),
        _ => return false,
    }
    true
}

pub(super) fn map_nrpn_with_lfo(
    number: u16,
    raw: u16,
    state: &mut LfoPairingState,
    emit: &mut impl FnMut(MappedUpdate),
) {
    if number == 4099 {
        emit(MappedUpdate::MidiClockMode(MidiClockMode::from_index(
            raw as usize,
        )));
        return;
    }
    if (37..=56).contains(&number) {
        let lfo = usize::from((number - 37) / 5);
        match (number - 37) % 5 {
            0 => {
                state.lfo_rate_raw[lfo] = Some(raw);
                emit_rev2_lfo_rate(lfo, raw, state.lfo_clock_sync[lfo], emit);
                return;
            }
            4 => {
                let synced = raw != 0;
                state.lfo_clock_sync[lfo] = synced;
                emit_param(emit, lfo_clock_sync_param(lfo), f32::from(synced));
                if let Some(rate_raw) = state.lfo_rate_raw[lfo] {
                    emit_rev2_lfo_rate(lfo, rate_raw, synced, emit);
                }
                return;
            }
            _ => {}
        }
    }
    map_nrpn(number, raw, emit);
}

fn emit_rev2_lfo_rate(lfo: usize, raw: u16, synced: bool, emit: &mut impl FnMut(MappedUpdate)) {
    const RATE: [ParamId; 4] = [
        ParamId::Lfo1Rate,
        ParamId::Lfo2Rate,
        ParamId::Lfo3Rate,
        ParamId::Lfo4Rate,
    ];
    const SYNC_DIVISION: [ParamId; 4] = [
        ParamId::Lfo1SyncDivision,
        ParamId::Lfo2SyncDivision,
        ParamId::Lfo3SyncDivision,
        ParamId::Lfo4SyncDivision,
    ];
    if synced {
        emit_param(
            emit,
            SYNC_DIVISION[lfo],
            LfoSyncDivision::from_rev2_raw(raw).index() as f32,
        );
    } else {
        emit_param(
            emit,
            RATE[lfo],
            logarithmic(raw, 150, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ),
        );
    }
}

const fn lfo_clock_sync_param(lfo: usize) -> ParamId {
    match lfo {
        1 => ParamId::Lfo2ClockSync,
        2 => ParamId::Lfo3ClockSync,
        3 => ParamId::Lfo4ClockSync,
        _ => ParamId::Lfo1ClockSync,
    }
}

pub(super) fn map_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(MappedUpdate)) {
    match number {
        0 => emit_param(emit, ParamId::Osc1Frequency, f32::from(raw.min(120))),
        1 => emit_param(emit, ParamId::Osc1FineTune, f32::from(raw.min(100)) - 50.0),
        2 => emit_osc_shape(emit, true, raw.min(4)),
        3 => emit_param(emit, ParamId::Osc1Glide, unit(raw, 127)),
        4 => emit_param(emit, ParamId::Osc1KeyboardOn, f32::from(raw != 0)),
        5 => emit_param(emit, ParamId::Osc2Frequency, f32::from(raw.min(120))),
        6 => emit_param(emit, ParamId::Osc2FineTune, f32::from(raw.min(100)) - 50.0),
        7 => emit_osc_shape(emit, false, raw.min(4)),
        8 => emit_param(emit, ParamId::Osc2Glide, unit(raw, 127)),
        9 => emit_param(emit, ParamId::Osc2KeyboardOn, f32::from(raw != 0)),
        10 => emit_param(emit, ParamId::HardSync, f32::from(raw != 0)),
        11 => emit_param(emit, ParamId::GlideMode, f32::from(raw.min(3))),
        12 => emit_param(emit, ParamId::OscSlop, unit(raw, 127)),
        13 => emit_param(emit, ParamId::OscMix, unit(raw, 127)),
        14 => emit_param(emit, ParamId::NoiseLevel, unit(raw, 127)),
        15 => emit_param(
            emit,
            ParamId::FilterCutoff,
            cutoff_raw_to_hz(raw.min(FILTER_CUTOFF_RAW_MAX)),
        ),
        16 => emit_param(emit, ParamId::FilterResonance, unit(raw, 127)),
        17 => emit_param(emit, ParamId::FilterKeyTrack, key_track_from_raw(raw)),
        18 => emit_param(emit, ParamId::FilterAudioMod, unit(raw, 127)),
        19 => emit_param(emit, ParamId::FilterPoles, f32::from(raw != 0)),
        20 => emit_param(emit, ParamId::FilterEnvAmount, bipolar(raw, 254)),
        21 => emit_param(emit, ParamId::FilterVelocity, unit(raw, 127)),
        22 => emit_param(emit, ParamId::FilterEgDelay, ranged(raw, 127, 0.0, 5.0)),
        23 => emit_param(emit, ParamId::FilterEgAttack, attack_decay_seconds(raw)),
        24 => emit_param(emit, ParamId::FilterEgDecay, attack_decay_seconds(raw)),
        25 => emit_param(emit, ParamId::FilterEgSustain, unit(raw, 127)),
        26 => emit_param(emit, ParamId::FilterEgRelease, release_seconds(raw)),
        28 => emit_param(emit, ParamId::PanSpread, unit(raw, 127)),
        29 => emit_param(emit, ParamId::MasterVolume, unit(raw, 127)),
        30 => emit_param(emit, ParamId::AmpEnvAmount, unit(raw, 127)),
        31 => emit_param(emit, ParamId::AmpVelocity, unit(raw, 127)),
        32 => emit_param(emit, ParamId::AmpEgDelay, ranged(raw, 127, 0.0, 5.0)),
        33 => emit_param(emit, ParamId::AmpEgAttack, attack_decay_seconds(raw)),
        34 => emit_param(emit, ParamId::AmpEgDecay, attack_decay_seconds(raw)),
        35 => emit_param(emit, ParamId::AmpEgSustain, unit(raw, 127)),
        36 => emit_param(emit, ParamId::AmpEgRelease, release_seconds(raw)),
        37..=56 => map_lfo_nrpn(number, raw, emit),
        57 => emit_param(
            emit,
            ParamId::AuxEgDestination,
            f32::from(ModDestination::from_index(usize::from(raw.min(52))).index() as u16),
        ),
        58 => emit_param(emit, ParamId::AuxEgAmount, bipolar(raw, 254)),
        59 => emit_param(emit, ParamId::AuxEgVelocity, unit(raw, 127)),
        60 => emit_param(emit, ParamId::AuxEgDelay, ranged(raw, 127, 0.0, 5.0)),
        61 => emit_param(emit, ParamId::AuxEgAttack, attack_decay_seconds(raw)),
        62 => emit_param(emit, ParamId::AuxEgDecay, attack_decay_seconds(raw)),
        63 => emit_param(emit, ParamId::AuxEgSustain, unit(raw, 127)),
        64 => emit_param(emit, ParamId::AuxEgRelease, release_seconds(raw)),
        65..=88 => map_free_mod_nrpn(number, raw, emit),
        97 => emit_param(emit, ParamId::AuxEgLoop, f32::from(raw != 0)),
        99 => emit_param(emit, ParamId::Osc1NoteReset, f32::from(raw != 0)),
        100 => emit_param(emit, ParamId::PitchBendRange, f32::from(raw.min(12))),
        102 => emit_param(emit, ParamId::Osc1ShapeMod, unit(raw, 99)),
        103 => emit_param(emit, ParamId::Osc2ShapeMod, unit(raw, 99)),
        104 => emit_param(emit, ParamId::Osc2NoteReset, f32::from(raw != 0)),
        105 => emit_param(emit, ParamId::Lfo1KeySync, f32::from(raw != 0)),
        106 => emit_param(emit, ParamId::Lfo2KeySync, f32::from(raw != 0)),
        107 => emit_param(emit, ParamId::Lfo3KeySync, f32::from(raw != 0)),
        108 => emit_param(emit, ParamId::Lfo4KeySync, f32::from(raw != 0)),
        110 => emit_param(emit, ParamId::SubOscLevel, unit(raw, 127)),
        111 => emit_param(emit, ParamId::GlideEnabled, f32::from(raw != 0)),
        116..=125 => map_dedicated_mod_nrpn(number, raw, emit),
        153 => emit_param(emit, ParamId::EffectEnabled, f32::from(raw != 0)),
        154 => emit_param(emit, ParamId::EffectType, f32::from(raw.min(12))),
        155 => emit_param(emit, ParamId::EffectMix, unit(raw, 127)),
        156 => emit_param(emit, ParamId::EffectParam1, unit(raw, 255)),
        157 => emit_param(emit, ParamId::EffectParam2, unit(raw, 127)),
        158 => emit_param(emit, ParamId::EffectClockSync, f32::from(raw != 0)),
        167 => emit_param(emit, ParamId::UnisonDetune, f32::from(raw.min(16))),
        168 => emit_param(emit, ParamId::UnisonEnabled, f32::from(raw != 0)),
        169 => emit_param(emit, ParamId::UnisonMode, f32::from(raw.min(16))),
        170 => emit_param(emit, ParamId::KeyMode, key_mode_index(raw)),
        175 => emit_param(emit, ParamId::ClockDivide, f32::from(raw.min(12))),
        179 => emit_param(emit, ParamId::Bpm, f32::from(raw.clamp(30, 250))),
        172 => emit_param(emit, ParamId::ArpEnabled, f32::from(raw != 0)),
        173 => emit_param(emit, ParamId::ArpMode, f32::from(raw.min(4))),
        174 => emit_param(emit, ParamId::ArpRange, f32::from(raw.min(2))),
        177 => emit_param(emit, ParamId::ArpRepeats, f32::from(raw.min(2))),
        178 => emit_param(emit, ParamId::ArpRelatch, f32::from(raw != 0)),
        4099 => emit(MappedUpdate::MidiClockMode(MidiClockMode::from_index(
            raw as usize,
        ))),
        _ => {}
    }
}

fn map_lfo_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(MappedUpdate)) {
    let lfo = usize::from((number - 37) / 5);
    let field = (number - 37) % 5;
    let params = [
        [
            ParamId::Lfo1Rate,
            ParamId::Lfo1Waveform,
            ParamId::Lfo1Depth,
            ParamId::Lfo1Destination,
            ParamId::Lfo1ClockSync,
        ],
        [
            ParamId::Lfo2Rate,
            ParamId::Lfo2Waveform,
            ParamId::Lfo2Depth,
            ParamId::Lfo2Destination,
            ParamId::Lfo2ClockSync,
        ],
        [
            ParamId::Lfo3Rate,
            ParamId::Lfo3Waveform,
            ParamId::Lfo3Depth,
            ParamId::Lfo3Destination,
            ParamId::Lfo3ClockSync,
        ],
        [
            ParamId::Lfo4Rate,
            ParamId::Lfo4Waveform,
            ParamId::Lfo4Depth,
            ParamId::Lfo4Destination,
            ParamId::Lfo4ClockSync,
        ],
    ];
    let value = match field {
        0 => logarithmic(raw, 150, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ),
        1 => f32::from(raw.min(4)),
        2 => unit(raw, 127),
        3 => f32::from(ModDestination::from_index(usize::from(raw.min(52))).index() as u16),
        _ => f32::from(raw != 0),
    };
    emit_param(emit, params[lfo][field as usize], value);
}

fn map_free_mod_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(MappedUpdate)) {
    let index = usize::from((number - 65) / 3);
    let parameter = match (number - 65) % 3 {
        0 => ModulationParam::Source(ModSource::from_index(usize::from(raw.min(22)))),
        1 => ModulationParam::Amount(bipolar(raw, 254)),
        _ => ModulationParam::Destination(ModDestination::from_index(usize::from(raw.min(52)))),
    };
    emit(MappedUpdate::Modulation {
        route: ModRoute::Free(index),
        parameter,
    });
}

fn map_dedicated_mod_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(MappedUpdate)) {
    let index = usize::from((number - 116) / 2);
    let parameter = if (number - 116) % 2 == 0 {
        ModulationParam::Amount(bipolar(raw, 254))
    } else {
        ModulationParam::Destination(ModDestination::from_index(usize::from(raw.min(52))))
    };
    emit(MappedUpdate::Modulation {
        route: ModRoute::Dedicated(DedicatedModSource::ALL[index]),
        parameter,
    });
}

pub(super) fn nrpn_max(number: u16) -> Option<u16> {
    Some(match number {
        0 | 5 => 120,
        1 | 6 => 100,
        2 | 7 | 38 | 43 | 48 | 53 => 4,
        10 | 19 | 41 | 46 | 51 | 56 | 97 | 99 | 104..=108 | 153 | 158 => 1,
        11 => 3,
        111 | 168 => 1,
        167 | 169 => 16,
        170 => 5,
        175 => 12,
        179 => 250,
        172 => 1,
        173 => 4,
        174 => 2,
        177 => 2,
        178 => 1,
        4099 => 4,
        4190 => 1,
        15 => 164,
        20 | 58 | 66 | 69 | 72 | 75 | 78 | 81 | 84 | 87 | 116 | 118 | 120 | 122 | 124 => 254,
        37 | 42 | 47 | 52 => 150,
        40 | 45 | 50 | 55 | 57 | 67 | 70 | 73 | 76 | 79 | 82 | 85 | 88 | 117 | 119 | 121 | 123
        | 125 => 52,
        65 | 68 | 71 | 74 | 77 | 80 | 83 | 86 => 22,
        102 | 103 => 99,
        100 | 154 => 12,
        156 => 255,
        12..=14 | 16..=18 | 21..=26 | 28..=36 | 39 | 44 | 49 | 54 | 59..=64 | 110 | 155 | 157 => {
            127
        }
        _ => return None,
    })
}
