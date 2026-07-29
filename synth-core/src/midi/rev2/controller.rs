//! Live Rev2 CC/NRPN controller decode.

use crate::midi::clock::MidiClockMode;
use crate::midi::rev2::layer::{Layer, LayerA, LayerB};
use crate::midi::rev2::map::{LfoPairingState, MappedUpdate, map_cc, map_nrpn_with_lfo, nrpn_max};
use crate::{LayerId, LayerTarget, ModRoute, ModulationParam, ParamId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidiUpdate {
    Param {
        target: LayerTarget,
        param: ParamId,
        value: f32,
    },
    MasterVolume(f32),
    MidiClockMode(MidiClockMode),
    EditLayer(LayerId),
    Modulation {
        target: LayerTarget,
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
    lfo: [LfoPairingState; 2],
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
pub struct ControllerDecoder {
    channels: [NrpnChannelState; 16],
}

impl Default for ControllerDecoder {
    fn default() -> Self {
        Self {
            channels: [NrpnChannelState::default(); 16],
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
                    emit_live_nrpn(number, raw, state, &mut emit);
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
                    emit_live_nrpn(number, next, state, &mut emit);
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
    if number == 4190 {
        emit(MidiUpdate::EditLayer(if raw == 0 {
            LayerA::ID
        } else {
            LayerB::ID
        }));
        return;
    }
    let (target, layer_index, number) = if (LayerB::NRPN_OFFSET..4096).contains(&number) {
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
        MappedUpdate::Modulation { route, parameter } => MidiUpdate::Modulation {
            target,
            route,
            parameter,
        },
    });
}

fn clamp_nrpn_value(number: u16, raw: u16) -> u16 {
    let base_number = if (LayerB::NRPN_OFFSET..4096).contains(&number) {
        number - LayerB::NRPN_OFFSET
    } else {
        number
    };
    raw.min(nrpn_max(base_number).unwrap_or(u16::MAX))
}
