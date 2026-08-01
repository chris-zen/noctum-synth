//! Live Rev2 NRPN/CC parameter encoder.

use crate::{
    DedicatedModSource, LayerId, LayerMode, LfoSyncDivision, MAX_SPLIT_POINT, ModDestination,
    ModRoute, ModSource, ParamId, SequenceUpdate, SequencerType,
    dsp::{MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ},
    math::F32,
    midi::{
        clock::MidiClockMode,
        prophet::{
            FILTER_CUTOFF_RAW_MAX, MAX_BPM, MIDI_CC_STATUS_BASE, MIDI_CHANNEL_MASK, MIN_BPM,
            attack_decay_raw, cutoff_hz_to_raw, key_track_to_raw, release_raw,
        },
        rev2::{
            ids::*,
            layer::{Layer, LayerB},
            map::{bool_raw, emit_nrpn, key_mode_raw, quantize, quantize_log},
            program::layer_mode_raw,
        },
    },
    sequencer::model::{GATED_STEP_COUNT, GATED_TRACK_COUNT, POLY_LANE_COUNT, POLY_STEP_COUNT},
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
            NRPN_EDIT_LAYER,
            match layer {
                LayerId::A => 0,
                LayerId::B => 1,
            },
            &mut emit,
        );
    }

    pub fn layer_mode(&mut self, channel: u8, mode: LayerMode, mut emit: impl FnMut([u8; 3])) {
        emit_nrpn(
            channel,
            NRPN_LAYER_MODE,
            u16::from(layer_mode_raw(mode)),
            &mut emit,
        );
    }

    pub fn split_point(&mut self, channel: u8, split_point: u8, mut emit: impl FnMut([u8; 3])) {
        emit_nrpn(
            channel,
            NRPN_SPLIT_POINT,
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
                MIDI_CC_STATUS_BASE | (channel & MIDI_CHANNEL_MASK),
                CC_PAN_MOD_MODE,
                if value >= 0.5 { 127 } else { 0 },
            ]);
            return true;
        }
        let mapped = match param {
            ParamId::Osc1Waveform => {
                self.oscillator_waveforms[layer_index][0] = value as u8;
                (
                    NRPN_OSC1_SHAPE,
                    u16::from(self.oscillator_shape(layer_index, 0)),
                )
            }
            ParamId::Osc1Enabled => {
                self.oscillator_enabled[layer_index][0] = value >= 0.5;
                (
                    NRPN_OSC1_SHAPE,
                    u16::from(self.oscillator_shape(layer_index, 0)),
                )
            }
            ParamId::Osc1Frequency => (NRPN_OSC1_FREQUENCY, quantize(value, 0.0, 120.0, 120)),
            ParamId::Osc1FineTune => (NRPN_OSC1_FINE_TUNE, quantize(value, -50.0, 50.0, 100)),
            ParamId::Osc1ShapeMod => (NRPN_OSC1_SHAPE_MOD, quantize(value, 0.0, 1.0, 99)),
            ParamId::Osc2Waveform => {
                self.oscillator_waveforms[layer_index][1] = value as u8;
                (
                    NRPN_OSC2_SHAPE,
                    u16::from(self.oscillator_shape(layer_index, 1)),
                )
            }
            ParamId::Osc2Enabled => {
                self.oscillator_enabled[layer_index][1] = value >= 0.5;
                (
                    NRPN_OSC2_SHAPE,
                    u16::from(self.oscillator_shape(layer_index, 1)),
                )
            }
            ParamId::Osc2Frequency => (NRPN_OSC2_FREQUENCY, quantize(value, 0.0, 120.0, 120)),
            ParamId::Osc2FineTune => (NRPN_OSC2_FINE_TUNE, quantize(value, -50.0, 50.0, 100)),
            ParamId::Osc2ShapeMod => (NRPN_OSC2_SHAPE_MOD, quantize(value, 0.0, 1.0, 99)),
            ParamId::OscMix => (NRPN_OSC_MIX, quantize(value, 0.0, 1.0, 127)),
            ParamId::SubOscLevel => (NRPN_SUB_OSC_LEVEL, quantize(value, 0.0, 1.0, 127)),
            ParamId::NoiseLevel => (NRPN_NOISE_LEVEL, quantize(value, 0.0, 1.0, 127)),
            ParamId::HardSync => (NRPN_HARD_SYNC, bool_raw(value)),
            ParamId::OscSlop | ParamId::AnalogDrift => {
                (NRPN_OSC_SLOP, quantize(value, 0.0, 1.0, 127))
            }
            ParamId::Osc1NoteReset => (NRPN_OSC1_NOTE_RESET, bool_raw(value)),
            ParamId::Osc2NoteReset => (NRPN_OSC2_NOTE_RESET, bool_raw(value)),
            ParamId::Osc1KeyboardOn => (NRPN_OSC1_KEYBOARD, bool_raw(value)),
            ParamId::Osc2KeyboardOn => (NRPN_OSC2_KEYBOARD, bool_raw(value)),
            ParamId::Osc1Glide => (NRPN_OSC1_GLIDE, quantize(value, 0.0, 1.0, 127)),
            ParamId::Osc2Glide => (NRPN_OSC2_GLIDE, quantize(value, 0.0, 1.0, 127)),
            ParamId::GlideMode => (NRPN_GLIDE_MODE, quantize(value, 0.0, 3.0, 3)),
            ParamId::GlideEnabled => (NRPN_GLIDE_ENABLED, bool_raw(value)),
            ParamId::KeyMode => (NRPN_KEY_MODE, key_mode_raw(value)),
            ParamId::UnisonEnabled => (NRPN_UNISON_ENABLED, bool_raw(value)),
            ParamId::UnisonMode => (NRPN_UNISON_MODE, quantize(value, 0.0, 16.0, 16)),
            ParamId::UnisonDetune => (NRPN_UNISON_DETUNE, quantize(value, 0.0, 16.0, 16)),
            ParamId::Bpm => (
                179,
                F32(value.clamp(f32::from(MIN_BPM), f32::from(MAX_BPM)))
                    .round()
                    .as_f32() as u16,
            ),
            ParamId::ClockDivide => (NRPN_CLOCK_DIVIDE, quantize(value, 0.0, 12.0, 12)),
            ParamId::FilterCutoff => (
                NRPN_FILTER_CUTOFF,
                cutoff_hz_to_raw(value, FILTER_CUTOFF_RAW_MAX),
            ),
            ParamId::FilterResonance => (NRPN_FILTER_RESONANCE, quantize(value, 0.0, 1.0, 127)),
            ParamId::FilterPoles => (NRPN_FILTER_POLES, bool_raw(value)),
            ParamId::FilterKeyTrack => (NRPN_FILTER_KEY_TRACK, key_track_to_raw(value)),
            ParamId::FilterEnvAmount => (NRPN_FILTER_ENV_AMOUNT, quantize(value, -1.0, 1.0, 254)),
            ParamId::FilterVelocity => (NRPN_FILTER_VELOCITY, quantize(value, 0.0, 1.0, 127)),
            ParamId::FilterAudioMod => (NRPN_FILTER_AUDIO_MOD, quantize(value, 0.0, 1.0, 127)),
            ParamId::FilterEgDelay => (NRPN_FILTER_EG_DELAY, quantize(value, 0.0, 5.0, 127)),
            ParamId::FilterEgAttack => (NRPN_FILTER_EG_ATTACK, attack_decay_raw(value)),
            ParamId::FilterEgDecay => (NRPN_FILTER_EG_DECAY, attack_decay_raw(value)),
            ParamId::FilterEgSustain => (NRPN_FILTER_EG_SUSTAIN, quantize(value, 0.0, 1.0, 127)),
            ParamId::FilterEgRelease => (NRPN_FILTER_EG_RELEASE, release_raw(value)),
            ParamId::PanSpread => (NRPN_PAN_SPREAD, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpEnvAmount => (NRPN_AMP_ENV_AMOUNT, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpVelocity => (NRPN_AMP_VELOCITY, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpEgDelay => (NRPN_AMP_EG_DELAY, quantize(value, 0.0, 5.0, 127)),
            ParamId::AmpEgAttack => (NRPN_AMP_EG_ATTACK, attack_decay_raw(value)),
            ParamId::AmpEgDecay => (NRPN_AMP_EG_DECAY, attack_decay_raw(value)),
            ParamId::AmpEgSustain => (NRPN_AMP_EG_SUSTAIN, quantize(value, 0.0, 1.0, 127)),
            ParamId::AmpEgRelease => (NRPN_AMP_EG_RELEASE, release_raw(value)),
            ParamId::AuxEgDestination => (NRPN_AUX_EG_DESTINATION, quantize(value, 0.0, 52.0, 52)),
            ParamId::AuxEgAmount => (NRPN_AUX_EG_AMOUNT, quantize(value, -1.0, 1.0, 254)),
            ParamId::AuxEgVelocity => (NRPN_AUX_EG_VELOCITY, quantize(value, 0.0, 1.0, 127)),
            ParamId::AuxEgDelay => (NRPN_AUX_EG_DELAY, quantize(value, 0.0, 5.0, 127)),
            ParamId::AuxEgAttack => (NRPN_AUX_EG_ATTACK, attack_decay_raw(value)),
            ParamId::AuxEgDecay => (NRPN_AUX_EG_DECAY, attack_decay_raw(value)),
            ParamId::AuxEgSustain => (NRPN_AUX_EG_SUSTAIN, quantize(value, 0.0, 1.0, 127)),
            ParamId::AuxEgRelease => (NRPN_AUX_EG_RELEASE, release_raw(value)),
            ParamId::AuxEgLoop => (NRPN_AUX_EG_LOOP, bool_raw(value)),
            ParamId::Lfo1Rate => (
                NRPN_LFO1_RATE,
                quantize_log(value, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ, 150),
            ),
            ParamId::Lfo1SyncDivision => (
                NRPN_LFO1_RATE,
                LfoSyncDivision::from_index(value as usize).rev2_raw(),
            ),
            ParamId::Lfo1Waveform => (NRPN_LFO1_WAVEFORM, quantize(value, 0.0, 4.0, 4)),
            ParamId::Lfo1Depth => (NRPN_LFO1_DEPTH, quantize(value, 0.0, 1.0, 127)),
            ParamId::Lfo1Destination => (NRPN_LFO1_DESTINATION, quantize(value, 0.0, 52.0, 52)),
            ParamId::Lfo1ClockSync => (NRPN_LFO1_CLOCK_SYNC, bool_raw(value)),
            ParamId::Lfo1KeySync => (NRPN_LFO1_KEY_SYNC, bool_raw(value)),
            ParamId::Lfo2Rate => (
                NRPN_LFO2_RATE,
                quantize_log(value, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ, 150),
            ),
            ParamId::Lfo2SyncDivision => (
                NRPN_LFO2_RATE,
                LfoSyncDivision::from_index(value as usize).rev2_raw(),
            ),
            ParamId::Lfo2Waveform => (NRPN_LFO2_WAVEFORM, quantize(value, 0.0, 4.0, 4)),
            ParamId::Lfo2Depth => (NRPN_LFO2_DEPTH, quantize(value, 0.0, 1.0, 127)),
            ParamId::Lfo2Destination => (NRPN_LFO2_DESTINATION, quantize(value, 0.0, 52.0, 52)),
            ParamId::Lfo2ClockSync => (NRPN_LFO2_CLOCK_SYNC, bool_raw(value)),
            ParamId::Lfo2KeySync => (NRPN_LFO2_KEY_SYNC, bool_raw(value)),
            ParamId::Lfo3Rate => (
                NRPN_LFO3_RATE,
                quantize_log(value, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ, 150),
            ),
            ParamId::Lfo3SyncDivision => (
                NRPN_LFO3_RATE,
                LfoSyncDivision::from_index(value as usize).rev2_raw(),
            ),
            ParamId::Lfo3Waveform => (NRPN_LFO3_WAVEFORM, quantize(value, 0.0, 4.0, 4)),
            ParamId::Lfo3Depth => (NRPN_LFO3_DEPTH, quantize(value, 0.0, 1.0, 127)),
            ParamId::Lfo3Destination => (NRPN_LFO3_DESTINATION, quantize(value, 0.0, 52.0, 52)),
            ParamId::Lfo3ClockSync => (NRPN_LFO3_CLOCK_SYNC, bool_raw(value)),
            ParamId::Lfo3KeySync => (NRPN_LFO3_KEY_SYNC, bool_raw(value)),
            ParamId::Lfo4Rate => (
                NRPN_LFO4_RATE,
                quantize_log(value, MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ, 150),
            ),
            ParamId::Lfo4SyncDivision => (
                NRPN_LFO4_RATE,
                LfoSyncDivision::from_index(value as usize).rev2_raw(),
            ),
            ParamId::Lfo4Waveform => (NRPN_LFO4_WAVEFORM, quantize(value, 0.0, 4.0, 4)),
            ParamId::Lfo4Depth => (NRPN_LFO4_DEPTH, quantize(value, 0.0, 1.0, 127)),
            ParamId::Lfo4Destination => (NRPN_LFO4_DESTINATION, quantize(value, 0.0, 52.0, 52)),
            ParamId::Lfo4ClockSync => (NRPN_LFO4_CLOCK_SYNC, bool_raw(value)),
            ParamId::Lfo4KeySync => (NRPN_LFO4_KEY_SYNC, bool_raw(value)),
            ParamId::EffectEnabled => (NRPN_EFFECT_ENABLED, bool_raw(value)),
            ParamId::EffectType => (NRPN_EFFECT_TYPE, quantize(value, 0.0, 12.0, 12)),
            ParamId::EffectMix => (NRPN_EFFECT_MIX, quantize(value, 0.0, 1.0, 127)),
            ParamId::EffectClockSync => (NRPN_EFFECT_CLOCK_SYNC, bool_raw(value)),
            ParamId::EffectParam1 => (NRPN_EFFECT_PARAM1, quantize(value, 0.0, 1.0, 255)),
            ParamId::EffectParam2 => (NRPN_EFFECT_PARAM2, quantize(value, 0.0, 1.0, 127)),
            ParamId::ProgramVolume => (NRPN_PROGRAM_VOLUME, quantize(value, 0.0, 1.0, 127)),
            ParamId::PitchBendRange => (NRPN_PITCH_BEND_RANGE, quantize(value, 0.0, 12.0, 12)),
            ParamId::ArpEnabled => (NRPN_ARP_ENABLED, bool_raw(value)),
            ParamId::ArpMode => (NRPN_ARP_MODE, quantize(value, 0.0, 4.0, 4)),
            ParamId::ArpRange => (NRPN_ARP_RANGE, quantize(value, 0.0, 2.0, 2)),
            ParamId::ArpRepeats => (NRPN_ARP_REPEATS, quantize(value, 0.0, 2.0, 2)),
            ParamId::ArpRelatch => (NRPN_ARP_RELATCH, bool_raw(value)),
            ParamId::SequencerType => (
                NRPN_SEQUENCER_TYPE,
                u16::from(quantize(value, 0.0, 1.0, 1) == SequencerType::Gated.index() as u16),
            ),
            ParamId::GatedSequencerMode => (NRPN_GATED_MODE, quantize(value, 0.0, 4.0, 4)),
            _ => return false,
        };
        let layer_offset = match layer {
            LayerId::A => 0,
            LayerId::B => LayerB::NRPN_OFFSET,
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
            LayerId::B => LayerB::NRPN_OFFSET,
        };
        if let SequenceUpdate::PolyLaneStep { step, lane, value } = update {
            if usize::from(step) >= POLY_STEP_COUNT || usize::from(lane) >= POLY_LANE_COUNT {
                return;
            }
            let base = NRPN_POLY_NOTE_START
                + u16::from(lane) * POLY_LANE_NRPN_STRIDE
                + u16::from(step)
                + offset;
            emit_nrpn(channel, base, value.note.rev2_raw(), &mut emit);
            emit_nrpn(
                channel,
                base + POLY_VELOCITY_NRPN_OFFSET,
                value.velocity.rev2_raw(),
                &mut emit,
            );
            return;
        }
        let (number, raw) = match update {
            SequenceUpdate::Type(value) => (
                NRPN_SEQUENCER_TYPE,
                u16::from(value == SequencerType::Gated),
            ),
            SequenceUpdate::GatedMode(value) => (NRPN_GATED_MODE, value.index() as u16),
            SequenceUpdate::GatedDestination { track, destination }
                if usize::from(track) < GATED_TRACK_COUNT =>
            {
                (
                    NRPN_GATED_DESTINATION_START + u16::from(track),
                    destination.rev2_raw(),
                )
            }
            SequenceUpdate::GatedStep { track, step, value }
                if usize::from(track) < GATED_TRACK_COUNT
                    && usize::from(step) < GATED_STEP_COUNT =>
            {
                (
                    NRPN_GATED_STEP_START
                        + u16::from(track) * GATED_STEP_COUNT as u16
                        + u16::from(step),
                    value.rev2_raw(),
                )
            }
            SequenceUpdate::PolyNote { step, lane, value }
                if usize::from(step) < POLY_STEP_COUNT && usize::from(lane) < POLY_LANE_COUNT =>
            {
                (
                    NRPN_POLY_NOTE_START
                        + u16::from(lane) * POLY_LANE_NRPN_STRIDE
                        + u16::from(step),
                    value.rev2_raw(),
                )
            }
            SequenceUpdate::PolyVelocity { step, lane, value }
                if usize::from(step) < POLY_STEP_COUNT && usize::from(lane) < POLY_LANE_COUNT =>
            {
                (
                    NRPN_POLY_NOTE_START
                        + POLY_VELOCITY_NRPN_OFFSET
                        + u16::from(lane) * POLY_LANE_NRPN_STRIDE
                        + u16::from(step),
                    value.rev2_raw(),
                )
            }
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
            LayerId::B => LayerB::NRPN_OFFSET,
        };
        emit_nrpn(
            channel,
            NRPN_SEQUENCER_RUNNING + offset,
            u16::from(running),
            &mut emit,
        );
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
            LayerId::B => LayerB::NRPN_OFFSET,
        };
        emit_nrpn(
            channel,
            NRPN_SEQUENCER_RECORDING + offset,
            u16::from(recording),
            &mut emit,
        );
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
            LayerId::B => LayerB::NRPN_OFFSET,
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
        emit_nrpn(channel, NRPN_MIDI_CLOCK, mode.index() as u16, &mut emit);
    }

    /// Encode the device-global Master Volume as CC 7.
    pub fn master_volume(&mut self, channel: u8, value: f32, mut emit: impl FnMut([u8; 3])) {
        emit([
            MIDI_CC_STATUS_BASE | (channel & MIDI_CHANNEL_MASK),
            CC_MASTER_VOLUME,
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
