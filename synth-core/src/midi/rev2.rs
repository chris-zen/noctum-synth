//! Sequential Prophet Rev2-compatible CC and NRPN parameter codec.

use super::scale::{
    FILTER_CUTOFF_RAW_MAX, cutoff_hz_to_raw, cutoff_raw_to_hz, key_track_from_raw, key_track_to_raw,
};
use crate::dsp::{MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
use crate::math::F32;
use crate::patch::decode_patch_name;
use crate::{
    DedicatedModSource, LfoSyncDivision, MidiClockMode, ModDestination, ModRoute, ModSource,
    ModulationParam, ParamId, Patch,
};

const LAYER_A_NAME_RANGE: core::ops::Range<usize> = 235..255;

pub const REV2_PROGRAM_DATA_LEN: usize = 2046;
pub const REV2_PROGRAM_PACKED_LEN: usize = 2339;
pub const REV2_PROGRAM_DATA_SYSEX_LEN: usize = 2346;
pub const REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN: usize = 2344;
const REV2_LAYER_DATA_LEN: usize = 1024;
const REV2_SYSEX_HEADER: [u8; 4] = [0xf0, 0x01, 0x2f, 0x03];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rev2SysexError {
    InvalidLength,
    InvalidFraming,
    InvalidManufacturer,
    InvalidModel,
    UnsupportedCommand,
    InvalidBank,
    NonSevenBitData,
    OutputTooSmall,
}

#[derive(Debug, Clone)]
pub struct Rev2ProgramData {
    pub bank: u8,
    pub program: u8,
    pub patch: Patch,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rev2MidiUpdate {
    Param(ParamId, f32),
    MidiClockMode(MidiClockMode),
    Modulation {
        route: ModRoute,
        parameter: ModulationParam,
    },
}

#[derive(Clone, Copy, Default)]
struct NrpnChannelState {
    number_msb: Option<u8>,
    number_lsb: Option<u8>,
    data_msb: Option<u8>,
    current_value: Option<u16>,
    rpn_msb: Option<u8>,
    rpn_lsb: Option<u8>,
    lfo_rate_raw: [Option<u16>; 4],
    lfo_clock_sync: [bool; 4],
}

impl NrpnChannelState {
    fn number(self) -> Option<u16> {
        Some(u16::from(self.number_msb?) * 128 + u16::from(self.number_lsb?))
    }

    fn clear_nrpn(&mut self) {
        self.number_msb = None;
        self.number_lsb = None;
        self.data_msb = None;
        self.current_value = None;
    }
}

/// Stateful Rev2 controller decoder. NRPN selection is independent per channel.
pub struct Rev2MidiDecoder {
    channels: [NrpnChannelState; 16],
}

impl Default for Rev2MidiDecoder {
    fn default() -> Self {
        Self {
            channels: [NrpnChannelState::default(); 16],
        }
    }
}

impl Rev2MidiDecoder {
    /// Decode a stored Prophet Rev2 Program Data dump.
    pub fn program_data(message: &[u8]) -> Result<Rev2ProgramData, Rev2SysexError> {
        validate_header(message, REV2_PROGRAM_DATA_SYSEX_LEN, 0x02)?;
        let bank = message[4];
        let program = message[5];
        if bank > 7 {
            return Err(Rev2SysexError::InvalidBank);
        }
        if program & 0x80 != 0 {
            return Err(Rev2SysexError::NonSevenBitData);
        }
        let patch = decode_patch_payload(&message[6..6 + REV2_PROGRAM_PACKED_LEN])?;
        Ok(Rev2ProgramData {
            bank,
            program,
            patch,
        })
    }

    /// Decode a Prophet Rev2 Program Edit Buffer data dump into Layer A of a patch.
    pub fn program_edit_buffer(message: &[u8]) -> Result<Patch, Rev2SysexError> {
        validate_header(message, REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN, 0x03)?;
        decode_patch_payload(&message[4..4 + REV2_PROGRAM_PACKED_LEN])
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
    if message[1] != 0x01 {
        return Err(Rev2SysexError::InvalidManufacturer);
    }
    if message[2] != 0x2f {
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

    let mut raw = [0_u8; REV2_PROGRAM_DATA_LEN];
    unpack_program_data(packed, &mut raw);
    let mut patch = Patch::default();
    let mut state = NrpnChannelState::default();
    for number in 0..=179 {
        if let Some(value) = program_nrpn_value(&raw, number, 0) {
            map_nrpn_stateful(number, value, &mut state, &mut |update| match update {
                Rev2MidiUpdate::Param(param, value) => patch.set_param(param, value),
                Rev2MidiUpdate::MidiClockMode(_) => {}
                Rev2MidiUpdate::Modulation { route, parameter } => {
                    patch.set_modulation_param(route, parameter);
                }
            });
        }
    }
    patch.name = decode_patch_name(&raw[LAYER_A_NAME_RANGE]);
    patch.set_param(ParamId::VcaInitialLevel, unit(u16::from(raw[27]), 127));
    Ok(patch)
}

impl Rev2MidiDecoder {
    /// Decode one CC. Returns `true` when the controller belongs to the Rev2
    /// parameter protocol, even when the sequence is not complete yet.
    pub fn control_change(
        &mut self,
        channel: u8,
        controller: u8,
        value: u8,
        mut emit: impl FnMut(Rev2MidiUpdate),
    ) -> bool {
        let Some(state) = self.channels.get_mut(usize::from(channel)) else {
            return false;
        };
        match controller {
            99 => {
                state.number_msb = Some(value);
                state.data_msb = None;
                state.current_value = None;
                state.rpn_msb = None;
                state.rpn_lsb = None;
                true
            }
            98 => {
                state.number_lsb = Some(value);
                state.data_msb = None;
                state.current_value = None;
                state.rpn_msb = None;
                state.rpn_lsb = None;
                true
            }
            6 => {
                state.data_msb = Some(value);
                true
            }
            38 => {
                if let (Some(number), Some(msb)) = (state.number(), state.data_msb) {
                    let raw = clamp_nrpn_value(number, u16::from(msb) * 128 + u16::from(value));
                    state.current_value = Some(raw);
                    map_nrpn_stateful(number, raw, state, &mut emit);
                }
                true
            }
            96 | 97 => {
                if let (Some(number), Some(current)) = (state.number(), state.current_value) {
                    let next = if controller == 96 {
                        current.saturating_add(1)
                    } else {
                        current.saturating_sub(1)
                    };
                    let next = clamp_nrpn_value(number, next);
                    state.current_value = Some(next);
                    map_nrpn_stateful(number, next, state, &mut emit);
                }
                true
            }
            101 => {
                state.rpn_msb = Some(value);
                if state.rpn_msb == Some(127) && state.rpn_lsb == Some(127) {
                    state.clear_nrpn();
                }
                true
            }
            100 => {
                state.rpn_lsb = Some(value);
                if state.rpn_msb == Some(127) && state.rpn_lsb == Some(127) {
                    state.clear_nrpn();
                }
                true
            }
            _ => map_cc(controller, value, &mut emit),
        }
    }
}

/// Stateful Rev2 NRPN encoder. Oscillator shape combines enabled/waveform state.
pub struct Rev2MidiEncoder {
    oscillator_waveforms: [u8; 2],
    oscillator_enabled: [bool; 2],
}

impl Default for Rev2MidiEncoder {
    fn default() -> Self {
        Self {
            oscillator_waveforms: [0; 2],
            oscillator_enabled: [true, false],
        }
    }
}

impl Rev2MidiEncoder {
    /// Encode a patch as a stored Prophet Rev2 Program Data dump.
    pub fn program_data(
        bank: u8,
        program: u8,
        patch: &Patch,
        output: &mut [u8],
    ) -> Result<usize, Rev2SysexError> {
        if bank > 7 {
            return Err(Rev2SysexError::InvalidBank);
        }
        if program & 0x80 != 0 {
            return Err(Rev2SysexError::NonSevenBitData);
        }
        if output.len() < REV2_PROGRAM_DATA_SYSEX_LEN {
            return Err(Rev2SysexError::OutputTooSmall);
        }

        let mut raw = [0_u8; REV2_PROGRAM_DATA_LEN];
        encode_program_layers(patch, &mut raw);
        output[..6].copy_from_slice(&[0xf0, 0x01, 0x2f, 0x02, bank, program]);
        pack_program_data(&raw, &mut output[6..6 + REV2_PROGRAM_PACKED_LEN]);
        output[REV2_PROGRAM_DATA_SYSEX_LEN - 1] = 0xf7;
        Ok(REV2_PROGRAM_DATA_SYSEX_LEN)
    }

    /// Encode a patch as a Prophet Rev2 Program Edit Buffer data dump.
    pub fn program_edit_buffer(patch: &Patch, output: &mut [u8]) -> Result<usize, Rev2SysexError> {
        if output.len() < REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN {
            return Err(Rev2SysexError::OutputTooSmall);
        }

        let mut raw = [0_u8; REV2_PROGRAM_DATA_LEN];
        encode_program_layers(patch, &mut raw);

        output[..4].copy_from_slice(&REV2_SYSEX_HEADER);
        pack_program_data(&raw, &mut output[4..4 + REV2_PROGRAM_PACKED_LEN]);
        output[REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN - 1] = 0xf7;
        Ok(REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN)
    }

    pub fn param(
        &mut self,
        channel: u8,
        param: ParamId,
        value: f32,
        mut emit: impl FnMut([u8; 3]),
    ) -> bool {
        if param == ParamId::PanModMode {
            emit([
                0xb0 | (channel & 0x0f),
                10,
                if value >= 0.5 { 127 } else { 0 },
            ]);
            return true;
        }
        let mapped = match param {
            ParamId::Osc1Waveform => {
                self.oscillator_waveforms[0] = value as u8;
                (2, u16::from(self.oscillator_shape(0)))
            }
            ParamId::Osc1Enabled => {
                self.oscillator_enabled[0] = value >= 0.5;
                (2, u16::from(self.oscillator_shape(0)))
            }
            ParamId::Osc1Frequency => (0, quantize(value, 0.0, 120.0, 120)),
            ParamId::Osc1FineTune => (1, quantize(value, -50.0, 50.0, 100)),
            ParamId::Osc1ShapeMod => (102, quantize(value, 0.0, 1.0, 99)),
            ParamId::Osc2Waveform => {
                self.oscillator_waveforms[1] = value as u8;
                (7, u16::from(self.oscillator_shape(1)))
            }
            ParamId::Osc2Enabled => {
                self.oscillator_enabled[1] = value >= 0.5;
                (7, u16::from(self.oscillator_shape(1)))
            }
            ParamId::Osc2Frequency => (5, quantize(value, 0.0, 120.0, 120)),
            ParamId::Osc2FineTune => (6, quantize(value, -50.0, 50.0, 100)),
            ParamId::Osc2ShapeMod => (103, quantize(value, 0.0, 1.0, 99)),
            ParamId::OscMix => (13, quantize(value, 0.0, 1.0, 127)),
            ParamId::SubOscLevel => (110, quantize(value, 0.0, 1.0, 127)),
            ParamId::NoiseLevel => (14, quantize(value, 0.0, 1.0, 127)),
            ParamId::HardSync => (10, bool_raw(value)),
            ParamId::OscSlop | ParamId::AnalogDrift => (12, quantize(value, 0.0, 1.0, 127)),
            ParamId::Osc1NoteReset => (99, bool_raw(value)),
            ParamId::Osc2NoteReset => (104, bool_raw(value)),
            ParamId::Osc1KeyboardOn => (4, bool_raw(value)),
            ParamId::Osc2KeyboardOn => (9, bool_raw(value)),
            ParamId::Osc1Glide => (3, quantize(value, 0.0, 1.0, 127)),
            ParamId::Osc2Glide => (8, quantize(value, 0.0, 1.0, 127)),
            ParamId::GlideMode => (11, quantize(value, 0.0, 3.0, 3)),
            ParamId::GlideEnabled => (111, bool_raw(value)),
            ParamId::KeyMode => (170, key_mode_raw(value)),
            ParamId::UnisonEnabled => (168, bool_raw(value)),
            ParamId::UnisonMode => (169, quantize(value, 0.0, 16.0, 16)),
            ParamId::UnisonDetune => (167, quantize(value, 0.0, 16.0, 16)),
            ParamId::Bpm => (179, F32(value.clamp(30.0, 250.0)).round().as_f32() as u16),
            ParamId::ClockDivide => (175, quantize(value, 0.0, 12.0, 12)),
            ParamId::FilterCutoff => (15, cutoff_hz_to_raw(value, FILTER_CUTOFF_RAW_MAX)),
            ParamId::FilterResonance => (16, quantize(value, 0.0, 1.0, 127)),
            ParamId::FilterPoles => (19, bool_raw(value)),
            ParamId::FilterKeyTrack => (17, key_track_to_raw(value)),
            ParamId::FilterEnvAmount => (20, quantize(value, -1.0, 1.0, 254)),
            ParamId::FilterVelocity => (21, quantize(value, 0.0, 1.0, 127)),
            ParamId::FilterAudioMod => (18, quantize(value, 0.0, 1.0, 127)),
            ParamId::FilterEgDelay => (22, quantize(value, 0.0, 5.0, 127)),
            ParamId::FilterEgAttack => (23, quantize(value, 0.0005, 5.0, 127)),
            ParamId::FilterEgDecay => (24, quantize(value, 0.0005, 5.0, 127)),
            ParamId::FilterEgSustain => (25, quantize(value, 0.0, 1.0, 127)),
            ParamId::FilterEgRelease => (26, quantize(value, 0.0005, 10.0, 127)),
            ParamId::PanSpread => (28, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpEnvAmount => (30, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpVelocity => (31, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpEgDelay => (32, quantize(value, 0.0, 5.0, 127)),
            ParamId::AmpEgAttack => (33, quantize(value, 0.0005, 5.0, 127)),
            ParamId::AmpEgDecay => (34, quantize(value, 0.0005, 5.0, 127)),
            ParamId::AmpEgSustain => (35, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpEgRelease => (36, quantize(value, 0.0005, 10.0, 127)),
            ParamId::AuxEgDestination => (57, quantize(value, 0.0, 52.0, 52)),
            ParamId::AuxEgAmount => (58, quantize(value, -1.0, 1.0, 254)),
            ParamId::AuxEgVelocity => (59, quantize(value, 0.0, 1.0, 127)),
            ParamId::AuxEgDelay => (60, quantize(value, 0.0, 5.0, 127)),
            ParamId::AuxEgAttack => (61, quantize(value, 0.0005, 5.0, 127)),
            ParamId::AuxEgDecay => (62, quantize(value, 0.0005, 5.0, 127)),
            ParamId::AuxEgSustain => (63, quantize(value, 0.0, 1.0, 127)),
            ParamId::AuxEgRelease => (64, quantize(value, 0.0005, 10.0, 127)),
            ParamId::AuxEgLoop => (97, bool_raw(value)),
            ParamId::Lfo1Rate => (
                37,
                quantize_log(value, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ, 150),
            ),
            ParamId::Lfo1SyncDivision => {
                (37, LfoSyncDivision::from_index(value as usize).rev2_raw())
            }
            ParamId::Lfo1Waveform => (38, quantize(value, 0.0, 4.0, 4)),
            ParamId::Lfo1Depth => (39, quantize(value, 0.0, 1.0, 127)),
            ParamId::Lfo1Destination => (40, quantize(value, 0.0, 52.0, 52)),
            ParamId::Lfo1ClockSync => (41, bool_raw(value)),
            ParamId::Lfo1KeySync => (105, bool_raw(value)),
            ParamId::Lfo2Rate => (
                42,
                quantize_log(value, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ, 150),
            ),
            ParamId::Lfo2SyncDivision => {
                (42, LfoSyncDivision::from_index(value as usize).rev2_raw())
            }
            ParamId::Lfo2Waveform => (43, quantize(value, 0.0, 4.0, 4)),
            ParamId::Lfo2Depth => (44, quantize(value, 0.0, 1.0, 127)),
            ParamId::Lfo2Destination => (45, quantize(value, 0.0, 52.0, 52)),
            ParamId::Lfo2ClockSync => (46, bool_raw(value)),
            ParamId::Lfo2KeySync => (106, bool_raw(value)),
            ParamId::Lfo3Rate => (
                47,
                quantize_log(value, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ, 150),
            ),
            ParamId::Lfo3SyncDivision => {
                (47, LfoSyncDivision::from_index(value as usize).rev2_raw())
            }
            ParamId::Lfo3Waveform => (48, quantize(value, 0.0, 4.0, 4)),
            ParamId::Lfo3Depth => (49, quantize(value, 0.0, 1.0, 127)),
            ParamId::Lfo3Destination => (50, quantize(value, 0.0, 52.0, 52)),
            ParamId::Lfo3ClockSync => (51, bool_raw(value)),
            ParamId::Lfo3KeySync => (107, bool_raw(value)),
            ParamId::Lfo4Rate => (
                52,
                quantize_log(value, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ, 150),
            ),
            ParamId::Lfo4SyncDivision => {
                (52, LfoSyncDivision::from_index(value as usize).rev2_raw())
            }
            ParamId::Lfo4Waveform => (53, quantize(value, 0.0, 4.0, 4)),
            ParamId::Lfo4Depth => (54, quantize(value, 0.0, 1.0, 127)),
            ParamId::Lfo4Destination => (55, quantize(value, 0.0, 52.0, 52)),
            ParamId::Lfo4ClockSync => (56, bool_raw(value)),
            ParamId::Lfo4KeySync => (108, bool_raw(value)),
            ParamId::EffectEnabled => (153, bool_raw(value)),
            ParamId::EffectType => (154, quantize(value, 0.0, 12.0, 12)),
            ParamId::EffectMix => (155, quantize(value, 0.0, 1.0, 127)),
            ParamId::EffectClockSync => (158, bool_raw(value)),
            ParamId::EffectParam1 => (156, quantize(value, 0.0, 1.0, 255)),
            ParamId::EffectParam2 => (157, quantize(value, 0.0, 1.0, 127)),
            ParamId::MasterVolume => (29, quantize(value, 0.0, 1.0, 127)),
            ParamId::PitchBendRange => (100, quantize(value, 0.0, 12.0, 12)),
            ParamId::ArpEnabled => (172, bool_raw(value)),
            ParamId::ArpMode => (173, quantize(value, 0.0, 4.0, 4)),
            ParamId::ArpRange => (174, quantize(value, 0.0, 2.0, 2)),
            ParamId::ArpRepeats => (177, quantize(value, 0.0, 2.0, 2)),
            ParamId::ArpRelatch => (178, bool_raw(value)),
            _ => return false,
        };
        emit_nrpn(channel, mapped.0, mapped.1, &mut emit);
        true
    }

    pub fn modulation(
        &mut self,
        channel: u8,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
        mut emit: impl FnMut([u8; 3]),
    ) {
        match route {
            ModRoute::Free(index) if index < 8 => {
                let base = 65 + index as u16 * 3;
                emit_nrpn(
                    channel,
                    base,
                    if enabled { source.index() as u16 } else { 0 },
                    &mut emit,
                );
                emit_nrpn(
                    channel,
                    base + 1,
                    quantize(amount, -1.0, 1.0, 254),
                    &mut emit,
                );
                emit_nrpn(channel, base + 2, destination.index() as u16, &mut emit);
            }
            ModRoute::Dedicated(source) => {
                let Some(index) = DedicatedModSource::ALL
                    .iter()
                    .position(|item| *item == source)
                else {
                    return;
                };
                let base = 116 + index as u16 * 2;
                emit_nrpn(channel, base, quantize(amount, -1.0, 1.0, 254), &mut emit);
                emit_nrpn(
                    channel,
                    base + 1,
                    if enabled {
                        destination.index() as u16
                    } else {
                        0
                    },
                    &mut emit,
                );
            }
            ModRoute::Free(_) => {}
        }
    }

    /// Encode the Rev2 global MIDI Clock Mode parameter (NRPN 4099).
    pub fn midi_clock_mode(
        &mut self,
        channel: u8,
        mode: MidiClockMode,
        mut emit: impl FnMut([u8; 3]),
    ) {
        emit_nrpn(channel, 4099, mode.index() as u16, &mut emit);
    }

    fn oscillator_shape(&self, index: usize) -> u8 {
        if self.oscillator_enabled[index] {
            self.oscillator_waveforms[index].min(3) + 1
        } else {
            0
        }
    }
}

fn encode_program_layers(patch: &Patch, raw: &mut [u8; REV2_PROGRAM_DATA_LEN]) {
    encode_patch_layer(patch, &mut raw[..REV2_LAYER_DATA_LEN]);
    encode_patch_layer(&Patch::default(), &mut raw[REV2_LAYER_DATA_LEN..]);
}

fn encode_patch_layer(patch: &Patch, raw: &mut [u8]) {
    let mut encoder = Rev2MidiEncoder::default();
    patch.for_each_param(|param, value| {
        let inactive_lfo_rate = match param {
            ParamId::Lfo1Rate => patch.lfos[0].clock_sync,
            ParamId::Lfo2Rate => patch.lfos[1].clock_sync,
            ParamId::Lfo3Rate => patch.lfos[2].clock_sync,
            ParamId::Lfo4Rate => patch.lfos[3].clock_sync,
            ParamId::Lfo1SyncDivision => !patch.lfos[0].clock_sync,
            ParamId::Lfo2SyncDivision => !patch.lfos[1].clock_sync,
            ParamId::Lfo3SyncDivision => !patch.lfos[2].clock_sync,
            ParamId::Lfo4SyncDivision => !patch.lfos[3].clock_sync,
            _ => false,
        };
        if inactive_lfo_rate {
            return;
        }
        let mut messages = [[0_u8; 3]; 4];
        let mut len = 0;
        if encoder.param(0, param, value, |message| {
            messages[len] = message;
            len += 1;
        }) {
            store_nrpn(raw, &messages[..len]);
        }
    });
    patch.for_each_modulation(|route, slot| {
        let mut messages = [[0_u8; 3]; 12];
        let mut len = 0;
        encoder.modulation(
            0,
            route,
            slot.enabled,
            slot.source,
            slot.destination,
            slot.amount,
            |message| {
                messages[len] = message;
                len += 1;
            },
        );
        for sequence in messages[..len].chunks_exact(4) {
            store_nrpn(raw, sequence);
        }
    });
    raw[27] = quantize(patch.amplifier.initial_level, 0.0, 1.0, 127) as u8;
    raw[LAYER_A_NAME_RANGE].fill(b' ');
    raw[LAYER_A_NAME_RANGE.start..LAYER_A_NAME_RANGE.start + patch.name.len()]
        .copy_from_slice(patch.name.as_bytes());
}

fn store_nrpn(raw: &mut [u8], messages: &[[u8; 3]]) {
    if messages.len() != 4 {
        return;
    }
    let number = usize::from(messages[0][2]) * 128 + usize::from(messages[1][2]);
    let value = u16::from(messages[2][2]) * 128 + u16::from(messages[3][2]);
    store_program_nrpn(raw, number as u16, value.min(255), 0);
}

#[derive(Clone, Copy)]
struct ProgramField {
    value_offset: usize,
    msb_offset: Option<usize>,
}

fn program_field(number: u16, layer_offset: usize) -> Option<ProgramField> {
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

fn program_nrpn_value(raw: &[u8], number: u16, layer_offset: usize) -> Option<u16> {
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

fn store_program_nrpn(raw: &mut [u8], number: u16, value: u16, layer_offset: usize) {
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

fn pack_program_data(raw: &[u8; REV2_PROGRAM_DATA_LEN], packed: &mut [u8]) {
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
    debug_assert_eq!(output, REV2_PROGRAM_PACKED_LEN);
}

fn unpack_program_data(packed: &[u8], raw: &mut [u8; REV2_PROGRAM_DATA_LEN]) {
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
    debug_assert_eq!(input, REV2_PROGRAM_PACKED_LEN);
}

fn emit_nrpn(channel: u8, number: u16, value: u16, emit: &mut impl FnMut([u8; 3])) {
    let status = 0xb0 | (channel & 0x0f);
    emit([status, 99, ((number / 128) & 0x7f) as u8]);
    emit([status, 98, (number & 0x7f) as u8]);
    emit([status, 6, ((value / 128) & 0x7f) as u8]);
    emit([status, 38, (value & 0x7f) as u8]);
}

fn bool_raw(value: f32) -> u16 {
    u16::from(value >= 0.5)
}

fn key_mode_raw(value: f32) -> u16 {
    match crate::KeyMode::from_index(value as usize) {
        crate::KeyMode::Low => 0,
        crate::KeyMode::High => 1,
        crate::KeyMode::Last => 2,
        crate::KeyMode::LowRetrigger => 3,
        crate::KeyMode::HighRetrigger => 4,
        crate::KeyMode::LastRetrigger => 5,
    }
}

fn key_mode_index(raw: u16) -> f32 {
    (match raw.min(5) {
        0 => crate::KeyMode::Low.index(),
        1 => crate::KeyMode::High.index(),
        2 => crate::KeyMode::Last.index(),
        3 => crate::KeyMode::LowRetrigger.index(),
        4 => crate::KeyMode::HighRetrigger.index(),
        _ => crate::KeyMode::LastRetrigger.index(),
    }) as f32
}

fn quantize(value: f32, min: f32, max: f32, raw_max: u16) -> u16 {
    F32((value.clamp(min, max) - min) / (max - min) * raw_max as f32)
        .round()
        .as_f32() as u16
}

fn quantize_log(value: f32, min: f32, max: f32, raw_max: u16) -> u16 {
    let normalized = F32(value.clamp(min, max) / min).ln().as_f32() / F32(max / min).ln().as_f32();
    F32(normalized * raw_max as f32).round().as_f32() as u16
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

fn emit_param(emit: &mut impl FnMut(Rev2MidiUpdate), param: ParamId, value: f32) {
    emit(Rev2MidiUpdate::Param(param, value));
}

fn emit_osc_shape(emit: &mut impl FnMut(Rev2MidiUpdate), osc1: bool, raw: u16) {
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

fn map_cc(controller: u8, raw: u8, emit: &mut impl FnMut(Rev2MidiUpdate)) -> bool {
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
        76 => emit_param(emit, ParamId::AmpEgRelease, ranged(raw, 127, 0.0005, 10.0)),
        77 => emit_param(emit, ParamId::AuxEgSustain, unit(raw, 127)),
        78 => emit_param(emit, ParamId::AuxEgRelease, ranged(raw, 127, 0.0005, 10.0)),
        85 => emit_param(
            emit,
            ParamId::AuxEgDestination,
            F32(ranged(raw, 127, 0.0, 52.0)).round().as_f32(),
        ),
        86 => emit_param(emit, ParamId::AuxEgAmount, bipolar(raw, 127)),
        87 => emit_param(emit, ParamId::AuxEgVelocity, unit(raw, 127)),
        88 => emit_param(emit, ParamId::AuxEgDelay, ranged(raw, 127, 0.0, 5.0)),
        89 => emit_param(emit, ParamId::AuxEgAttack, ranged(raw, 127, 0.0005, 5.0)),
        90 => emit_param(emit, ParamId::AuxEgDecay, ranged(raw, 127, 0.0005, 5.0)),
        104 => emit_param(emit, ParamId::FilterKeyTrack, key_track_from_raw(raw)),
        105 => emit_param(emit, ParamId::FilterAudioMod, unit(raw, 127)),
        106 => emit_param(emit, ParamId::FilterEnvAmount, bipolar(raw, 127)),
        107 => emit_param(emit, ParamId::FilterVelocity, unit(raw, 127)),
        108 => emit_param(emit, ParamId::FilterEgDelay, ranged(raw, 127, 0.0, 5.0)),
        109 => emit_param(emit, ParamId::FilterEgAttack, ranged(raw, 127, 0.0005, 5.0)),
        110 => emit_param(emit, ParamId::FilterEgDecay, ranged(raw, 127, 0.0005, 5.0)),
        111 => emit_param(emit, ParamId::FilterEgSustain, unit(raw, 127)),
        112 => emit_param(
            emit,
            ParamId::FilterEgRelease,
            ranged(raw, 127, 0.0005, 10.0),
        ),
        114 => emit_param(emit, ParamId::PanSpread, unit(raw, 127)),
        113 => emit_param(emit, ParamId::VcaInitialLevel, unit(raw, 127)),
        115 => emit_param(emit, ParamId::AmpEnvAmount, unit(raw, 127)),
        116 => emit_param(emit, ParamId::AmpVelocity, unit(raw, 127)),
        117 => emit_param(emit, ParamId::AmpEgDelay, ranged(raw, 127, 0.0, 5.0)),
        118 => emit_param(emit, ParamId::AmpEgAttack, ranged(raw, 127, 0.0005, 5.0)),
        119 => emit_param(emit, ParamId::AmpEgDecay, ranged(raw, 127, 0.0005, 5.0)),
        33 => emit_param(emit, ParamId::ArpEnabled, f32::from(raw >= 64)),
        34 => emit_param(emit, ParamId::ArpMode, f32::from(raw.min(4))),
        35 => emit_param(emit, ParamId::ArpRange, f32::from(raw.min(2))),
        36 => emit_param(emit, ParamId::ArpRepeats, f32::from(raw.min(2))),
        _ => return false,
    }
    true
}

fn map_nrpn_stateful(
    number: u16,
    raw: u16,
    state: &mut NrpnChannelState,
    emit: &mut impl FnMut(Rev2MidiUpdate),
) {
    if number == 4099 {
        emit(Rev2MidiUpdate::MidiClockMode(MidiClockMode::from_index(
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

fn emit_rev2_lfo_rate(lfo: usize, raw: u16, synced: bool, emit: &mut impl FnMut(Rev2MidiUpdate)) {
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

fn map_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(Rev2MidiUpdate)) {
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
        23 => emit_param(emit, ParamId::FilterEgAttack, ranged(raw, 127, 0.0005, 5.0)),
        24 => emit_param(emit, ParamId::FilterEgDecay, ranged(raw, 127, 0.0005, 5.0)),
        25 => emit_param(emit, ParamId::FilterEgSustain, unit(raw, 127)),
        26 => emit_param(
            emit,
            ParamId::FilterEgRelease,
            ranged(raw, 127, 0.0005, 10.0),
        ),
        28 => emit_param(emit, ParamId::PanSpread, unit(raw, 127)),
        29 => emit_param(emit, ParamId::MasterVolume, unit(raw, 127)),
        30 => emit_param(emit, ParamId::AmpEnvAmount, unit(raw, 127)),
        31 => emit_param(emit, ParamId::AmpVelocity, unit(raw, 127)),
        32 => emit_param(emit, ParamId::AmpEgDelay, ranged(raw, 127, 0.0, 5.0)),
        33 => emit_param(emit, ParamId::AmpEgAttack, ranged(raw, 127, 0.0005, 5.0)),
        34 => emit_param(emit, ParamId::AmpEgDecay, ranged(raw, 127, 0.0005, 5.0)),
        35 => emit_param(emit, ParamId::AmpEgSustain, unit(raw, 127)),
        36 => emit_param(emit, ParamId::AmpEgRelease, ranged(raw, 127, 0.0005, 10.0)),
        37..=56 => map_lfo_nrpn(number, raw, emit),
        57 => emit_param(
            emit,
            ParamId::AuxEgDestination,
            f32::from(ModDestination::from_index(usize::from(raw.min(52))).index() as u16),
        ),
        58 => emit_param(emit, ParamId::AuxEgAmount, bipolar(raw, 254)),
        59 => emit_param(emit, ParamId::AuxEgVelocity, unit(raw, 127)),
        60 => emit_param(emit, ParamId::AuxEgDelay, ranged(raw, 127, 0.0, 5.0)),
        61 => emit_param(emit, ParamId::AuxEgAttack, ranged(raw, 127, 0.0005, 5.0)),
        62 => emit_param(emit, ParamId::AuxEgDecay, ranged(raw, 127, 0.0005, 5.0)),
        63 => emit_param(emit, ParamId::AuxEgSustain, unit(raw, 127)),
        64 => emit_param(emit, ParamId::AuxEgRelease, ranged(raw, 127, 0.0005, 10.0)),
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
        4099 => emit(Rev2MidiUpdate::MidiClockMode(MidiClockMode::from_index(
            raw as usize,
        ))),
        _ => {}
    }
}

fn map_lfo_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(Rev2MidiUpdate)) {
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

fn map_free_mod_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(Rev2MidiUpdate)) {
    let index = usize::from((number - 65) / 3);
    let parameter = match (number - 65) % 3 {
        0 => ModulationParam::Source(ModSource::from_index(usize::from(raw.min(22)))),
        1 => ModulationParam::Amount(bipolar(raw, 254)),
        _ => ModulationParam::Destination(ModDestination::from_index(usize::from(raw.min(52)))),
    };
    emit(Rev2MidiUpdate::Modulation {
        route: ModRoute::Free(index),
        parameter,
    });
}

fn map_dedicated_mod_nrpn(number: u16, raw: u16, emit: &mut impl FnMut(Rev2MidiUpdate)) {
    let index = usize::from((number - 116) / 2);
    let parameter = if (number - 116) % 2 == 0 {
        ModulationParam::Amount(bipolar(raw, 254))
    } else {
        ModulationParam::Destination(ModDestination::from_index(usize::from(raw.min(52))))
    };
    emit(Rev2MidiUpdate::Modulation {
        route: ModRoute::Dedicated(DedicatedModSource::ALL[index]),
        parameter,
    });
}

fn clamp_nrpn_value(number: u16, raw: u16) -> u16 {
    raw.min(nrpn_max(number).unwrap_or(u16::MAX))
}

fn nrpn_max(number: u16) -> Option<u16> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn program_data_message(
        bank: u8,
        program: u8,
        patch: &Patch,
    ) -> [u8; REV2_PROGRAM_DATA_SYSEX_LEN] {
        let mut edit = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        Rev2MidiEncoder::program_edit_buffer(patch, &mut edit).unwrap();
        let mut message = [0_u8; REV2_PROGRAM_DATA_SYSEX_LEN];
        message[..4].copy_from_slice(&[0xf0, 0x01, 0x2f, 0x02]);
        message[4] = bank;
        message[5] = program;
        message[6..6 + REV2_PROGRAM_PACKED_LEN]
            .copy_from_slice(&edit[4..4 + REV2_PROGRAM_PACKED_LEN]);
        message[REV2_PROGRAM_DATA_SYSEX_LEN - 1] = 0xf7;
        message
    }

    #[test]
    fn nrpn_round_trips_bipolar_filter_envelope() {
        let mut encoder = Rev2MidiEncoder::default();
        let mut decoder = Rev2MidiDecoder::default();
        let mut decoded = None;
        assert!(encoder.param(0, ParamId::FilterEnvAmount, 1.0, |message| {
            decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
        }));
        assert_eq!(
            decoded,
            Some(Rev2MidiUpdate::Param(ParamId::FilterEnvAmount, 1.0))
        );
    }

    #[test]
    fn bpm_nrpn_uses_direct_rev2_values() {
        for bpm in [30.0, 120.0, 250.0] {
            let mut encoder = Rev2MidiEncoder::default();
            let mut decoder = Rev2MidiDecoder::default();
            let mut decoded = None;
            assert!(encoder.param(0, ParamId::Bpm, bpm, |message| {
                decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
            }));
            assert_eq!(decoded, Some(Rev2MidiUpdate::Param(ParamId::Bpm, bpm)));
        }
    }

    #[test]
    fn filter_cutoff_nrpn_uses_semitone_ticks() {
        let mut decoder = Rev2MidiDecoder::default();
        let cases = [
            (0_u16, cutoff_raw_to_hz(0)),
            (96, cutoff_raw_to_hz(96)),
            (105, 440.0),
            (164, cutoff_raw_to_hz(164)),
        ];
        for (raw, expected_hz) in cases {
            let mut decoded = None;
            emit_nrpn(0, 15, raw, &mut |message| {
                decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
            });
            let Some(Rev2MidiUpdate::Param(ParamId::FilterCutoff, hz)) = decoded else {
                panic!("expected filter cutoff update for raw {raw}");
            };
            assert!(
                (hz - expected_hz).abs() < 0.05,
                "raw {raw}: got {hz}, expected {expected_hz}"
            );
        }
    }

    #[test]
    fn filter_cutoff_cc_matches_nrpn_index_not_full_open() {
        let mut decoder = Rev2MidiDecoder::default();
        let mut cc_hz = None;
        decoder.control_change(0, 102, 127, |update| {
            if let Rev2MidiUpdate::Param(ParamId::FilterCutoff, hz) = update {
                cc_hz = Some(hz);
            }
        });
        let mut nrpn_hz = None;
        emit_nrpn(0, 15, 127, &mut |message| {
            decoder.control_change(0, message[1], message[2], |update| {
                if let Rev2MidiUpdate::Param(ParamId::FilterCutoff, hz) = update {
                    nrpn_hz = Some(hz);
                }
            });
        });
        let cc_hz = cc_hz.expect("cc cutoff");
        let nrpn_hz = nrpn_hz.expect("nrpn cutoff");
        assert!((cc_hz - nrpn_hz).abs() < 0.05);
        assert!((cc_hz - cutoff_raw_to_hz(127)).abs() < 0.05);
        assert!(cc_hz < cutoff_raw_to_hz(164) * 0.2);
    }

    #[test]
    fn filter_key_track_64_decodes_to_unity() {
        let mut decoder = Rev2MidiDecoder::default();
        let mut decoded = None;
        emit_nrpn(0, 17, 64, &mut |message| {
            decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
        });
        assert_eq!(
            decoded,
            Some(Rev2MidiUpdate::Param(ParamId::FilterKeyTrack, 1.0))
        );
    }

    #[test]
    fn global_midi_clock_mode_round_trips_as_nrpn_4099() {
        for mode in MidiClockMode::ALL {
            let mut encoder = Rev2MidiEncoder::default();
            let mut decoder = Rev2MidiDecoder::default();
            let mut decoded = None;
            encoder.midi_clock_mode(0, mode, |message| {
                decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
            });
            assert_eq!(decoded, Some(Rev2MidiUpdate::MidiClockMode(mode)));
        }
    }

    #[test]
    fn synced_lfo_nrpn_decodes_for_either_rate_and_sync_order() {
        for sync_first in [false, true] {
            let mut decoder = Rev2MidiDecoder::default();
            let mut decoded_division = None;
            let mut send = |number, value| {
                emit_nrpn(0, number, value, &mut |message| {
                    decoder.control_change(0, message[1], message[2], |update| {
                        if let Rev2MidiUpdate::Param(ParamId::Lfo1SyncDivision, value) = update {
                            decoded_division = Some(value as usize);
                        }
                    });
                });
            };
            if sync_first {
                send(41, 1);
                send(37, 72);
            } else {
                send(37, 72);
                send(41, 1);
            }
            assert_eq!(
                decoded_division,
                Some(LfoSyncDivision::StepTwoThirds.index())
            );
        }
    }

    #[test]
    fn synced_lfo_program_round_trips_active_division() {
        for division in LfoSyncDivision::ALL {
            let mut source = Patch::default();
            source.lfos[1].rate_hz = 7.25;
            source.lfos[1].clock_sync = true;
            source.lfos[1].sync_division = division;
            let mut message = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
            Rev2MidiEncoder::program_edit_buffer(&source, &mut message).unwrap();
            let decoded = Rev2MidiDecoder::program_edit_buffer(&message).unwrap();
            assert!(decoded.lfos[1].clock_sync);
            assert_eq!(decoded.lfos[1].sync_division, division);
        }
    }

    #[test]
    fn unison_nrpn_uses_rev2_ranges_and_key_mode_order() {
        let mut encoder = Rev2MidiEncoder::default();
        let mut decoder = Rev2MidiDecoder::default();
        let mut decoded = None;
        assert!(encoder.param(0, ParamId::UnisonDetune, 16.0, |message| {
            decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
        }));
        assert_eq!(
            decoded,
            Some(Rev2MidiUpdate::Param(ParamId::UnisonDetune, 16.0))
        );

        let mut decoded = None;
        assert!(encoder.param(
            0,
            ParamId::KeyMode,
            crate::KeyMode::High.index() as f32,
            |message| {
                decoder.control_change(0, message[1], message[2], |update| decoded = Some(update));
            }
        ));
        assert_eq!(
            decoded,
            Some(Rev2MidiUpdate::Param(
                ParamId::KeyMode,
                crate::KeyMode::High.index() as f32,
            ))
        );
    }

    #[test]
    fn program_data_round_trips_documented_unison_fields() {
        let mut patch = Patch::default();
        patch.unison_enabled = true;
        patch.unison_mode = crate::UnisonMode::Chord;
        patch.unison_detune = 12.0;
        patch.key_mode = crate::KeyMode::LastRetrigger;
        let message = program_data_message(0, 0, &patch);
        let decoded = Rev2MidiDecoder::program_data(&message).unwrap().patch;
        assert!(decoded.unison_enabled);
        assert_eq!(decoded.unison_mode, crate::UnisonMode::Chord);
        assert_eq!(decoded.unison_detune, 12.0);
        assert_eq!(decoded.key_mode, crate::KeyMode::LastRetrigger);
        assert!(decoded.unison_chord.is_empty());
    }

    #[test]
    fn program_data_encoder_round_trips_address_and_patch() {
        let mut source = Patch::default();
        source.name.push_str("Stored Program").unwrap();
        source.filter.resonance = 0.75;
        let mut message = [0_u8; REV2_PROGRAM_DATA_SYSEX_LEN];

        let len = Rev2MidiEncoder::program_data(7, 127, &source, &mut message).unwrap();
        let decoded = Rev2MidiDecoder::program_data(&message).unwrap();

        assert_eq!(len, REV2_PROGRAM_DATA_SYSEX_LEN);
        assert_eq!((decoded.bank, decoded.program), (7, 127));
        assert_eq!(decoded.patch.name, source.name);
        assert!((decoded.patch.filter.resonance - source.filter.resonance).abs() < 0.01);
    }

    #[test]
    fn program_data_encoder_validates_address_and_capacity() {
        let patch = Patch::default();
        let mut message = [0_u8; REV2_PROGRAM_DATA_SYSEX_LEN];
        assert_eq!(
            Rev2MidiEncoder::program_data(8, 0, &patch, &mut message),
            Err(Rev2SysexError::InvalidBank)
        );
        assert_eq!(
            Rev2MidiEncoder::program_data(0, 128, &patch, &mut message),
            Err(Rev2SysexError::NonSevenBitData)
        );
        assert_eq!(
            Rev2MidiEncoder::program_data(0, 0, &patch, &mut message[..10]),
            Err(Rev2SysexError::OutputTooSmall)
        );
    }

    #[test]
    fn pan_mod_mode_round_trips_as_cc10() {
        let mut encoder = Rev2MidiEncoder::default();
        let mut decoder = Rev2MidiDecoder::default();
        let mut message = [0_u8; 3];
        assert!(encoder.param(3, ParamId::PanModMode, 1.0, |encoded| { message = encoded }));
        assert_eq!(message, [0xb3, 10, 127]);

        let mut decoded = None;
        assert!(decoder.control_change(3, message[1], message[2], |update| {
            decoded = Some(update)
        }));
        assert_eq!(
            decoded,
            Some(Rev2MidiUpdate::Param(ParamId::PanModMode, 1.0))
        );
    }

    #[test]
    fn oscillator_shape_combines_enabled_and_waveform() {
        let mut encoder = Rev2MidiEncoder::default();
        let mut messages = [[0; 3]; 8];
        let mut len = 0;
        encoder.param(0, ParamId::Osc1Waveform, 3.0, |message| {
            messages[len] = message;
            len += 1;
        });
        encoder.param(0, ParamId::Osc1Enabled, 0.0, |message| {
            messages[len] = message;
            len += 1;
        });
        assert_eq!(messages[3], [0xb0, 38, 4]);
        assert_eq!(messages[7], [0xb0, 38, 0]);
    }

    #[test]
    fn program_data_pack_round_trips_high_bits_and_partial_packet() {
        let mut raw = [0_u8; REV2_PROGRAM_DATA_LEN];
        raw[..9].copy_from_slice(&[0x80, 0x01, 0xfe, 0x7f, 0xaa, 0x55, 0xff, 0x81, 0x42]);
        raw[REV2_PROGRAM_DATA_LEN - 2..].copy_from_slice(&[0x80, 0xff]);
        let mut packed = [0_u8; REV2_PROGRAM_PACKED_LEN];
        pack_program_data(&raw, &mut packed);
        assert_eq!(packed[0], 0b0101_0101);
        assert!(packed.iter().all(|byte| *byte < 0x80));

        let mut decoded = [0_u8; REV2_PROGRAM_DATA_LEN];
        unpack_program_data(&packed, &mut decoded);
        assert_eq!(decoded, raw);
    }

    #[test]
    fn program_data_pack_uses_rev2_msb_bit_order() {
        let mut raw = [0_u8; REV2_PROGRAM_DATA_LEN];
        raw[0] = 0x80;
        let mut packed = [0_u8; REV2_PROGRAM_PACKED_LEN];
        pack_program_data(&raw, &mut packed);
        assert_eq!(packed[0], 0b0100_0000);

        raw.fill(0);
        raw[6] = 0x80;
        pack_program_data(&raw, &mut packed);
        assert_eq!(packed[0], 0b0000_0001);
    }

    #[test]
    fn factory_program_prefix_uses_program_offsets_not_nrpn_offsets() {
        // First two packed packets from Sequential's Rev2 factory bank v1.0.
        let mut packed = [0_u8; REV2_PROGRAM_PACKED_LEN];
        packed[..16].copy_from_slice(&[
            0x00, 0x18, 0x18, 0x30, 0x34, 0x01, 0x04, 0x32, 0x00, 0x2b, 0x29, 0x29, 0x01, 0x01,
            0x00, 0x00,
        ]);
        let mut raw = [0_u8; REV2_PROGRAM_DATA_LEN];
        unpack_program_data(&packed, &mut raw);

        assert_eq!(program_nrpn_value(&raw, 0, 0), Some(24));
        assert_eq!(program_nrpn_value(&raw, 1, 0), Some(48));
        assert_eq!(program_nrpn_value(&raw, 2, 0), Some(1));
        assert_eq!(program_nrpn_value(&raw, 5, 0), Some(24));
        assert_eq!(program_nrpn_value(&raw, 6, 0), Some(52));
        assert_eq!(program_nrpn_value(&raw, 7, 0), Some(4));
    }

    #[test]
    fn program_fields_round_trip_split_msb_values() {
        for number in [
            20, 58, 66, 69, 72, 75, 78, 81, 84, 87, 116, 118, 120, 122, 124,
        ] {
            for value in [0, 127, 254] {
                let mut raw = [0x55_u8; REV2_PROGRAM_DATA_LEN];
                store_program_nrpn(&mut raw, number, value, 0);
                assert_eq!(program_nrpn_value(&raw, number, 0), Some(value));
            }
        }
    }

    #[test]
    fn decode_patch_payload_reads_layer_a_name() {
        let mut raw = [0_u8; REV2_PROGRAM_DATA_LEN];
        raw[super::LAYER_A_NAME_RANGE].copy_from_slice(b"LosVangelis2041     ");
        let mut packed = [0_u8; REV2_PROGRAM_PACKED_LEN];
        pack_program_data(&raw, &mut packed);
        let patch = decode_patch_payload(&packed).unwrap();
        assert_eq!(patch.name.as_str(), "LosVangelis2041");
    }

    #[test]
    fn program_edit_buffer_round_trips_vca_initial_level() {
        let mut source = Patch::default();
        source.amplifier.initial_level = 103.0 / 127.0;

        let mut message = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        Rev2MidiEncoder::program_edit_buffer(&source, &mut message).unwrap();

        let decoded = Rev2MidiDecoder::program_edit_buffer(&message).unwrap();
        assert!(
            (decoded.amplifier.initial_level - source.amplifier.initial_level).abs() < 0.01,
            "decoded {} expected {}",
            decoded.amplifier.initial_level,
            source.amplifier.initial_level
        );
    }

    #[test]
    fn program_edit_buffer_round_trips_supported_patch_fields() {
        let mut source = Patch::default();
        source.osc1.waveform = 3;
        source.osc1.enabled = true;
        source.osc2.waveform = 2;
        source.osc2.enabled = true;
        source.osc1.shape_mod = 0.42;
        source.osc1.glide = 0.25;
        source.osc2.glide = 0.75;
        source.glide_mode = crate::GlideMode::FixedTimeAuto;
        source.glide_enabled = true;
        source.filter.cutoff = 1_234.0;
        source.filter.env_amount = -0.5;
        source.lfos[2].destination = ModDestination::FilterCutoff;
        source.effects.enabled = true;
        source.effects.effect_type = crate::EffectType::Reverb;
        source.effects.param1 = 0.75;
        source.mod_matrix.free_slots[0] = crate::ModMatrixSlot {
            enabled: true,
            source: ModSource::Lfo1,
            destination: ModDestination::Osc1ShapeMod,
            amount: -0.25,
        };

        let mut message = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        let len = Rev2MidiEncoder::program_edit_buffer(&source, &mut message).unwrap();
        assert_eq!(len, REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN);
        assert_eq!(&message[..4], &[0xf0, 0x01, 0x2f, 0x03]);
        assert_eq!(message[len - 1], 0xf7);

        let decoded = Rev2MidiDecoder::program_edit_buffer(&message).unwrap();
        assert_eq!(decoded.osc1.waveform, 3);
        assert!(decoded.osc1.enabled);
        assert_eq!(decoded.osc2.waveform, 2);
        assert!(decoded.osc2.enabled);
        assert!((decoded.osc1.glide - 0.25).abs() < 0.01);
        assert!((decoded.osc2.glide - 0.75).abs() < 0.01);
        assert_eq!(decoded.glide_mode, crate::GlideMode::FixedTimeAuto);
        assert!(decoded.glide_enabled);
        assert!((decoded.osc1.shape_mod - source.osc1.shape_mod).abs() < 0.02);
        assert!((decoded.filter.cutoff - source.filter.cutoff).abs() < 50.0);
        assert!((decoded.filter.env_amount - source.filter.env_amount).abs() < 0.01);
        assert_eq!(decoded.lfos[2].destination, ModDestination::FilterCutoff);
        assert!(decoded.effects.enabled);
        assert_eq!(decoded.effects.effect_type, crate::EffectType::Reverb);
        assert!((decoded.effects.param1 - source.effects.param1).abs() < 0.01);
        let slot = decoded.mod_matrix.free_slots[0];
        assert!(slot.enabled);
        assert_eq!(slot.source, ModSource::Lfo1);
        assert_eq!(slot.destination, ModDestination::Osc1ShapeMod);
        assert!((slot.amount - source.mod_matrix.free_slots[0].amount).abs() < 0.01);
    }

    #[test]
    fn program_edit_buffer_rejects_malformed_messages() {
        let mut message = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        Rev2MidiEncoder::program_edit_buffer(&Patch::default(), &mut message).unwrap();
        assert!(matches!(
            Rev2MidiDecoder::program_edit_buffer(&message[..message.len() - 1]),
            Err(Rev2SysexError::InvalidLength)
        ));
        message[1] = 2;
        assert!(matches!(
            Rev2MidiDecoder::program_edit_buffer(&message),
            Err(Rev2SysexError::InvalidManufacturer)
        ));
        message[1] = 1;
        message[4] = 0x80;
        assert!(matches!(
            Rev2MidiDecoder::program_edit_buffer(&message),
            Err(Rev2SysexError::NonSevenBitData)
        ));
    }

    #[test]
    fn program_edit_buffer_uses_default_layer_b_and_decodes_only_layer_a() {
        let mut source = Patch::default();
        source.filter.resonance = 0.25;
        source.osc1.waveform = 3;
        let mut message = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        Rev2MidiEncoder::program_edit_buffer(&source, &mut message).unwrap();

        let mut raw = [0_u8; REV2_PROGRAM_DATA_LEN];
        unpack_program_data(&message[4..4 + REV2_PROGRAM_PACKED_LEN], &mut raw);
        assert_eq!(raw[4] & 0x7f, 4);
        assert_eq!(raw[REV2_LAYER_DATA_LEN + 4] & 0x7f, 1);
        raw[REV2_LAYER_DATA_LEN + 23] = 127;
        pack_program_data(&raw, &mut message[4..4 + REV2_PROGRAM_PACKED_LEN]);

        let decoded = Rev2MidiDecoder::program_edit_buffer(&message).unwrap();
        assert!((decoded.filter.resonance - source.filter.resonance).abs() < 0.01);
    }

    #[test]
    fn program_edit_buffer_requires_complete_output_capacity() {
        let mut output = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN - 1];
        assert_eq!(
            Rev2MidiEncoder::program_edit_buffer(&Patch::default(), &mut output),
            Err(Rev2SysexError::OutputTooSmall)
        );
    }

    #[test]
    fn stored_program_data_decodes_metadata_and_patch() {
        let mut source = Patch::default();
        source.filter.resonance = 1.0;
        let message = program_data_message(7, 127, &source);
        let decoded = Rev2MidiDecoder::program_data(&message).unwrap();
        assert_eq!(decoded.bank, 7);
        assert_eq!(decoded.program, 127);
        assert_eq!(decoded.patch.filter.resonance, 1.0);
    }

    #[test]
    fn stored_program_data_rejects_invalid_metadata_and_payload() {
        let mut message = program_data_message(0, 0, &Patch::default());
        message[4] = 8;
        assert!(matches!(
            Rev2MidiDecoder::program_data(&message),
            Err(Rev2SysexError::InvalidBank)
        ));
        message[4] = 0;
        message[3] = 3;
        assert!(matches!(
            Rev2MidiDecoder::program_data(&message),
            Err(Rev2SysexError::UnsupportedCommand)
        ));
        message[3] = 2;
        message[6] = 0x80;
        assert!(matches!(
            Rev2MidiDecoder::program_data(&message),
            Err(Rev2SysexError::NonSevenBitData)
        ));
        assert!(matches!(
            Rev2MidiDecoder::program_data(&message[..message.len() - 1]),
            Err(Rev2SysexError::InvalidLength)
        ));
    }

    const FACTORY_SYSEX: &[u8] =
        include_bytes!("../../../Prophet-Rev2-Factory-Programs/Rev2_Programs_v1.0.syx");

    #[test]
    fn factory_program_decodes_mod_destination_indices() {
        let message = &FACTORY_SYSEX[..REV2_PROGRAM_DATA_SYSEX_LEN];
        let decoded = Rev2MidiDecoder::program_data(message).unwrap();
        assert_eq!(
            decoded.patch.lfos[2].destination,
            ModDestination::Osc1ShapeMod
        );

        let mut raw = [0_u8; REV2_PROGRAM_DATA_LEN];
        unpack_program_data(&message[6..6 + REV2_PROGRAM_PACKED_LEN], &mut raw);
        assert_eq!(raw[67] & 0x7f, 7);
        assert_eq!(raw[93] & 0x7f, 3);
    }

    #[test]
    fn rev2_oscillator_shape_uses_rev2_waveform_order() {
        for (raw, expected_waveform) in [(2, 1.0), (3, 2.0), (4, 3.0)] {
            let mut waveform = None;
            emit_osc_shape(
                &mut |update| {
                    if let Rev2MidiUpdate::Param(ParamId::Osc1Waveform, value) = update {
                        waveform = Some(value);
                    }
                },
                true,
                raw,
            );
            assert_eq!(waveform, Some(expected_waveform));
        }
    }

    #[test]
    fn shape_mod_nrpn_round_trips() {
        let mut encoder = Rev2MidiEncoder::default();
        let mut messages = [[0_u8; 3]; 4];
        let mut len = 0;
        encoder.param(0, ParamId::Osc1ShapeMod, 0.5, |message| {
            messages[len] = message;
            len += 1;
        });
        assert_eq!(len, 4);
        let number = u16::from(messages[0][2]) * 128 + u16::from(messages[1][2]);
        let value = u16::from(messages[2][2]) * 128 + u16::from(messages[3][2]);
        assert_eq!(number, 102);
        assert_eq!(value, 50);

        let mut decoded = None;
        map_nrpn(number, value, &mut |update| decoded = Some(update));
        assert_eq!(
            decoded,
            Some(Rev2MidiUpdate::Param(ParamId::Osc1ShapeMod, 50.0 / 99.0))
        );
    }

    #[test]
    fn shape_mod_cc_round_trips() {
        let mut decoded = None;
        map_cc(30, 64, &mut |update| decoded = Some(update));
        assert_eq!(
            decoded,
            Some(Rev2MidiUpdate::Param(ParamId::Osc1ShapeMod, 64.0 / 127.0))
        );
    }

    #[test]
    fn mod_destination_matches_cc_chart_indices() {
        assert_eq!(ModDestination::from_index(4), ModDestination::OscMix);
        assert_eq!(ModDestination::from_index(7), ModDestination::Osc1ShapeMod);
        assert_eq!(ModDestination::Osc1ShapeMod.index(), 7);
    }
}
