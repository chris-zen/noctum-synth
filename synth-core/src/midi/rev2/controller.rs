//! Live Rev2 CC/NRPN controller decode.

use crate::{
    LayerId, LayerMode, LayerTarget, MAX_SPLIT_POINT, ModRoute, ModulationParam, ParamId,
    SequenceUpdate,
    midi::{
        clock::MidiClockMode,
        prophet::NRPN_RADIX,
        rev2::{
            ids::*,
            layer::{Layer, LayerA, LayerB},
            map::{LfoPairingState, MappedUpdate, map_cc, map_nrpn_with_lfo, nrpn_max},
            program::layer_mode_from_raw,
        },
    },
};

const MIDI_CHANNEL_COUNT: usize = 16;
const LAYER_B_NRPN_END: u16 = LayerB::NRPN_OFFSET * 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidiUpdate {
    Param {
        target: LayerTarget,
        param: ParamId,
        value: f32,
    },
    MasterVolume(f32),
    MidiClockMode(MidiClockMode),
    LayerMode(LayerMode),
    SplitPoint(u8),
    EditLayer(LayerId),
    Modulation {
        target: LayerTarget,
        route: ModRoute,
        parameter: ModulationParam,
    },
    Sequence {
        target: LayerTarget,
        update: SequenceUpdate,
    },
    SequencerRunning {
        target: LayerTarget,
        running: bool,
    },
    SequencerRecording {
        target: LayerTarget,
        recording: bool,
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
    lfo: [LfoPairingState; 2],
}

impl NrpnChannelState {
    fn number(self) -> Option<u16> {
        Some(u16::from(self.number_msb?) * NRPN_RADIX + u16::from(self.number_lsb?))
    }

    fn clear_nrpn(&mut self) {
        self.number_msb = None;
        self.number_lsb = None;
        self.data_msb = None;
        self.current_value = None;
    }
}

/// Stateful Rev2 controller decoder. NRPN selection is independent per channel.
pub struct ControllerDecoder {
    channels: [NrpnChannelState; MIDI_CHANNEL_COUNT],
}

impl Default for ControllerDecoder {
    fn default() -> Self {
        Self {
            channels: [NrpnChannelState::default(); MIDI_CHANNEL_COUNT],
        }
    }
}

impl ControllerDecoder {
    /// Decode one CC. Returns `true` when the controller belongs to the Rev2
    /// parameter protocol, even when the sequence is not complete yet.
    pub fn control_change(
        &mut self,
        channel: u8,
        controller: u8,
        value: u8,
        mut emit: impl FnMut(MidiUpdate),
    ) -> bool {
        let Some(state) = self.channels.get_mut(usize::from(channel)) else {
            return false;
        };
        match controller {
            CC_NRPN_MSB => {
                state.number_msb = Some(value);
                state.data_msb = None;
                state.current_value = None;
                state.rpn_msb = None;
                state.rpn_lsb = None;
                true
            }
            CC_NRPN_LSB => {
                state.number_lsb = Some(value);
                state.data_msb = None;
                state.current_value = None;
                state.rpn_msb = None;
                state.rpn_lsb = None;
                true
            }
            CC_DATA_ENTRY_MSB => {
                state.data_msb = Some(value);
                true
            }
            CC_DATA_ENTRY_LSB => {
                if let (Some(number), Some(msb)) = (state.number(), state.data_msb) {
                    let raw =
                        clamp_nrpn_value(number, u16::from(msb) * NRPN_RADIX + u16::from(value));
                    state.current_value = Some(raw);
                    emit_live_nrpn(number, raw, state, &mut emit);
                }
                true
            }
            CC_DATA_INCREMENT | CC_DATA_DECREMENT => {
                if let (Some(number), Some(current)) = (state.number(), state.current_value) {
                    let next = if controller == CC_DATA_INCREMENT {
                        current.saturating_add(1)
                    } else {
                        current.saturating_sub(1)
                    };
                    let next = clamp_nrpn_value(number, next);
                    state.current_value = Some(next);
                    emit_live_nrpn(number, next, state, &mut emit);
                }
                true
            }
            CC_RPN_MSB => {
                state.rpn_msb = Some(value);
                if state.rpn_msb == Some(RPN_NULL) && state.rpn_lsb == Some(RPN_NULL) {
                    state.clear_nrpn();
                }
                true
            }
            CC_RPN_LSB => {
                state.rpn_lsb = Some(value);
                if state.rpn_msb == Some(RPN_NULL) && state.rpn_lsb == Some(RPN_NULL) {
                    state.clear_nrpn();
                }
                true
            }
            _ => map_cc(controller, value, &mut |update| {
                emit_mapped(LayerTarget::Edit, update, &mut emit)
            }),
        }
    }
}

fn emit_live_nrpn(
    number: u16,
    raw: u16,
    state: &mut NrpnChannelState,
    emit: &mut impl FnMut(MidiUpdate),
) {
    if number == NRPN_EDIT_LAYER {
        emit(MidiUpdate::EditLayer(if raw == 0 {
            LayerA::ID
        } else {
            LayerB::ID
        }));
        return;
    }
    // A/B Mode and Split Point are program-global (no Layer B NRPN offset).
    if number == NRPN_LAYER_MODE || number == LayerB::NRPN_OFFSET + NRPN_LAYER_MODE {
        if let Some(mode) = layer_mode_from_raw(raw.min(2) as u8) {
            emit(MidiUpdate::LayerMode(mode));
        }
        return;
    }
    if number == NRPN_SPLIT_POINT || number == LayerB::NRPN_OFFSET + NRPN_SPLIT_POINT {
        emit(MidiUpdate::SplitPoint(
            raw.min(u16::from(MAX_SPLIT_POINT)) as u8
        ));
        return;
    }
    let (target, layer_index, number) = if (LayerB::NRPN_OFFSET..LAYER_B_NRPN_END).contains(&number)
    {
        (
            LayerTarget::Explicit(LayerB::ID),
            1,
            number - LayerB::NRPN_OFFSET,
        )
    } else {
        (LayerTarget::Explicit(LayerA::ID), 0, number)
    };
    map_nrpn_with_lfo(number, raw, &mut state.lfo[layer_index], &mut |update| {
        emit_mapped(target, update, emit);
    });
}

fn emit_mapped(target: LayerTarget, update: MappedUpdate, emit: &mut impl FnMut(MidiUpdate)) {
    emit(match update {
        MappedUpdate::Param(param, value) => MidiUpdate::Param {
            target,
            param,
            value,
        },
        MappedUpdate::MasterVolume(volume) => MidiUpdate::MasterVolume(volume),
        MappedUpdate::MidiClockMode(mode) => MidiUpdate::MidiClockMode(mode),
        MappedUpdate::LayerMode(mode) => MidiUpdate::LayerMode(mode),
        MappedUpdate::SplitPoint(split_point) => MidiUpdate::SplitPoint(split_point),
        MappedUpdate::Modulation { route, parameter } => MidiUpdate::Modulation {
            target,
            route,
            parameter,
        },
        MappedUpdate::Sequence(update) => MidiUpdate::Sequence { target, update },
        MappedUpdate::SequencerRunning(running) => MidiUpdate::SequencerRunning { target, running },
        MappedUpdate::SequencerRecording(recording) => {
            MidiUpdate::SequencerRecording { target, recording }
        }
    });
}

fn clamp_nrpn_value(number: u16, raw: u16) -> u16 {
    let base_number = if (LayerB::NRPN_OFFSET..LAYER_B_NRPN_END).contains(&number) {
        number - LayerB::NRPN_OFFSET
    } else {
        number
    };
    raw.min(nrpn_max(base_number).unwrap_or(u16::MAX))
}
