//! Live Rev2 NRPN/CC parameter encoder.

use crate::dsp::{MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
use crate::math::F32;
use crate::midi::{
    clock::MidiClockMode,
    prophet::{
        FILTER_CUTOFF_RAW_MAX, attack_decay_raw, cutoff_hz_to_raw, key_track_to_raw, release_raw,
    },
    rev2::{
        map::{bool_raw, emit_nrpn, key_mode_raw, quantize, quantize_log},
        program::layer_mode_raw,
    },
};
use crate::{
    DedicatedModSource, LayerId, LayerMode, LfoSyncDivision, MAX_SPLIT_POINT, ModDestination,
    ModRoute, ModSource, ParamId, SequenceUpdate, SequencerType,
};

/// Stateful Rev2 NRPN encoder. Oscillator shape combines enabled/waveform state.
pub struct ControllerEncoder {
    oscillator_waveforms: [[u8; 2]; 2],
    oscillator_enabled: [[bool; 2]; 2],
}

impl Default for ControllerEncoder {
    fn default() -> Self {
        Self {
            oscillator_waveforms: [[0; 2]; 2],
            oscillator_enabled: [[true, false]; 2],
        }
    }
}

impl ControllerEncoder {
    pub fn edit_layer(&mut self, channel: u8, layer: LayerId, mut emit: impl FnMut([u8; 3])) {
        emit_nrpn(
            channel,
            4190,
            match layer {
                LayerId::A => 0,
                LayerId::B => 1,
            },
            &mut emit,
        );
    }

    pub fn layer_mode(&mut self, channel: u8, mode: LayerMode, mut emit: impl FnMut([u8; 3])) {
        emit_nrpn(channel, 163, u16::from(layer_mode_raw(mode)), &mut emit);
    }

    pub fn split_point(&mut self, channel: u8, split_point: u8, mut emit: impl FnMut([u8; 3])) {
        emit_nrpn(
            channel,
            171,
            u16::from(split_point.min(MAX_SPLIT_POINT)),
            &mut emit,
        );
    }

    pub fn param(
        &mut self,
        channel: u8,
        param: ParamId,
        value: f32,
        emit: impl FnMut([u8; 3]),
    ) -> bool {
        self.param_for_layer(channel, LayerId::A, param, value, emit)
    }

    pub fn param_for_layer(
        &mut self,
        channel: u8,
        layer: LayerId,
        param: ParamId,
        value: f32,
        mut emit: impl FnMut([u8; 3]),
    ) -> bool {
        let layer_index = match layer {
            LayerId::A => 0,
            LayerId::B => 1,
        };
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
                self.oscillator_waveforms[layer_index][0] = value as u8;
                (2, u16::from(self.oscillator_shape(layer_index, 0)))
            }
            ParamId::Osc1Enabled => {
                self.oscillator_enabled[layer_index][0] = value >= 0.5;
                (2, u16::from(self.oscillator_shape(layer_index, 0)))
            }
            ParamId::Osc1Frequency => (0, quantize(value, 0.0, 120.0, 120)),
            ParamId::Osc1FineTune => (1, quantize(value, -50.0, 50.0, 100)),
            ParamId::Osc1ShapeMod => (102, quantize(value, 0.0, 1.0, 99)),
            ParamId::Osc2Waveform => {
                self.oscillator_waveforms[layer_index][1] = value as u8;
                (7, u16::from(self.oscillator_shape(layer_index, 1)))
            }
            ParamId::Osc2Enabled => {
                self.oscillator_enabled[layer_index][1] = value >= 0.5;
                (7, u16::from(self.oscillator_shape(layer_index, 1)))
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
            ParamId::FilterEgAttack => (23, attack_decay_raw(value)),
            ParamId::FilterEgDecay => (24, attack_decay_raw(value)),
            ParamId::FilterEgSustain => (25, quantize(value, 0.0, 1.0, 127)),
            ParamId::FilterEgRelease => (26, release_raw(value)),
            ParamId::PanSpread => (28, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpEnvAmount => (30, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpVelocity => (31, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpEgDelay => (32, quantize(value, 0.0, 5.0, 127)),
            ParamId::AmpEgAttack => (33, attack_decay_raw(value)),
            ParamId::AmpEgDecay => (34, attack_decay_raw(value)),
            ParamId::AmpEgSustain => (35, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpEgRelease => (36, release_raw(value)),
            ParamId::AuxEgDestination => (57, quantize(value, 0.0, 52.0, 52)),
            ParamId::AuxEgAmount => (58, quantize(value, -1.0, 1.0, 254)),
            ParamId::AuxEgVelocity => (59, quantize(value, 0.0, 1.0, 127)),
            ParamId::AuxEgDelay => (60, quantize(value, 0.0, 5.0, 127)),
            ParamId::AuxEgAttack => (61, attack_decay_raw(value)),
            ParamId::AuxEgDecay => (62, attack_decay_raw(value)),
            ParamId::AuxEgSustain => (63, quantize(value, 0.0, 1.0, 127)),
            ParamId::AuxEgRelease => (64, release_raw(value)),
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
            ParamId::ProgramVolume => (29, quantize(value, 0.0, 1.0, 127)),
            ParamId::PitchBendRange => (100, quantize(value, 0.0, 12.0, 12)),
            ParamId::ArpEnabled => (172, bool_raw(value)),
            ParamId::ArpMode => (173, quantize(value, 0.0, 4.0, 4)),
            ParamId::ArpRange => (174, quantize(value, 0.0, 2.0, 2)),
            ParamId::ArpRepeats => (177, quantize(value, 0.0, 2.0, 2)),
            ParamId::ArpRelatch => (178, bool_raw(value)),
            ParamId::SequencerType => (
                183,
                u16::from(quantize(value, 0.0, 1.0, 1) == SequencerType::Gated.index() as u16),
            ),
            ParamId::GatedSequencerMode => (182, quantize(value, 0.0, 4.0, 4)),
            _ => return false,
        };
        let layer_offset = match layer {
            LayerId::A => 0,
            LayerId::B => 2048,
        };
        emit_nrpn(channel, mapped.0 + layer_offset, mapped.1, &mut emit);
        true
    }

    /// Encode one lossless sequencer edit as a Rev2 NRPN sequence.
    pub fn sequence(
        &mut self,
        channel: u8,
        layer: LayerId,
        update: SequenceUpdate,
        mut emit: impl FnMut([u8; 3]),
    ) {
        let offset = match layer {
            LayerId::A => 0,
            LayerId::B => 2048,
        };
        if let SequenceUpdate::PolyLaneStep { step, lane, value } = update {
            if step >= 64 || lane >= 6 {
                return;
            }
            let base = 276 + u16::from(lane) * 128 + u16::from(step) + offset;
            emit_nrpn(channel, base, value.note.rev2_raw(), &mut emit);
            emit_nrpn(channel, base + 64, value.velocity.rev2_raw(), &mut emit);
            return;
        }
        let (number, raw) = match update {
            SequenceUpdate::Type(value) => (183, u16::from(value == SequencerType::Gated)),
            SequenceUpdate::GatedMode(value) => (182, value.index() as u16),
            SequenceUpdate::GatedDestination { track, destination } if track < 4 => {
                (184 + u16::from(track), destination.rev2_raw())
            }
            SequenceUpdate::GatedStep { track, step, value } if track < 4 && step < 16 => (
                192 + u16::from(track) * 16 + u16::from(step),
                value.rev2_raw(),
            ),
            SequenceUpdate::PolyNote { step, lane, value } if step < 64 && lane < 6 => (
                276 + u16::from(lane) * 128 + u16::from(step),
                value.rev2_raw(),
            ),
            SequenceUpdate::PolyVelocity { step, lane, value } if step < 64 && lane < 6 => (
                340 + u16::from(lane) * 128 + u16::from(step),
                value.rev2_raw(),
            ),
            _ => return,
        };
        emit_nrpn(channel, number + offset, raw, &mut emit);
    }

    /// Encode the transient Rev2 polyphonic-sequencer play/stop switch (NRPN 180).
    pub fn sequencer_running(
        &mut self,
        channel: u8,
        layer: LayerId,
        running: bool,
        mut emit: impl FnMut([u8; 3]),
    ) {
        let offset = match layer {
            LayerId::A => 0,
            LayerId::B => 2048,
        };
        emit_nrpn(channel, 180 + offset, u16::from(running), &mut emit);
    }

    /// Encode the transient Rev2 polyphonic-sequencer record switch (NRPN 181).
    pub fn sequencer_recording(
        &mut self,
        channel: u8,
        layer: LayerId,
        recording: bool,
        mut emit: impl FnMut([u8; 3]),
    ) {
        let offset = match layer {
            LayerId::A => 0,
            LayerId::B => 2048,
        };
        emit_nrpn(channel, 181 + offset, u16::from(recording), &mut emit);
    }

    pub fn modulation(
        &mut self,
        channel: u8,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
        emit: impl FnMut([u8; 3]),
    ) {
        self.modulation_for_layer(
            channel,
            LayerId::A,
            route,
            enabled,
            source,
            destination,
            amount,
            emit,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn modulation_for_layer(
        &mut self,
        channel: u8,
        layer: LayerId,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
        mut emit: impl FnMut([u8; 3]),
    ) {
        let layer_offset = match layer {
            LayerId::A => 0,
            LayerId::B => 2048,
        };
        match route {
            ModRoute::Free(index) if index < 8 => {
                let base = layer_offset + 65 + index as u16 * 3;
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
                let base = layer_offset + 116 + index as u16 * 2;
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

    /// Encode the device-global Master Volume as CC 7.
    pub fn master_volume(&mut self, channel: u8, value: f32, mut emit: impl FnMut([u8; 3])) {
        emit([
            0xb0 | (channel & 0x0f),
            7,
            quantize(value, 0.0, 1.0, 127) as u8,
        ]);
    }

    fn oscillator_shape(&self, layer: usize, index: usize) -> u8 {
        if self.oscillator_enabled[layer][index] {
            self.oscillator_waveforms[layer][index].min(3) + 1
        } else {
            0
        }
    }
}
