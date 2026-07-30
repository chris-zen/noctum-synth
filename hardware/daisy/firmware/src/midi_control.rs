//! Pure MIDI-to-engine control translation shared by firmware and host tests.

use synth_core::ControlMessage;
use synth_core::midi::clock::MidiRealtimeEvent;
use synth_core::midi::rev2;
use wmidi::MidiMessage;

pub fn realtime_to_control(
    message: &MidiMessage<'_>,
    timestamp_micros: u64,
) -> Option<ControlMessage> {
    let event = match message {
        MidiMessage::TimingClock => MidiRealtimeEvent::TimingClock { timestamp_micros },
        MidiMessage::Start => MidiRealtimeEvent::Start,
        MidiMessage::Stop => MidiRealtimeEvent::Stop,
        _ => return None,
    };
    Some(ControlMessage::MidiRealtime(event))
}

/// Convert a supported MIDI message into one real-time synth command.
///
/// Channel selection is intentionally omni for the initial firmware. Messages
/// without a corresponding performance control are ignored.
pub fn message_to_control(message: MidiMessage<'_>) -> Option<ControlMessage> {
    match message {
        MidiMessage::NoteOn(_, note, velocity) => Some(ControlMessage::NoteOn {
            note: u8::from(note),
            velocity: u8::from(velocity) as f32 / 127.0,
        }),
        MidiMessage::NoteOff(_, note, _) => Some(ControlMessage::NoteOff {
            note: u8::from(note),
        }),
        MidiMessage::PitchBendChange(_, bend) => Some(ControlMessage::PitchBend {
            value: u16::from(bend) as f32 / 16_383.0 * 2.0 - 1.0,
        }),
        MidiMessage::PolyphonicKeyPressure(_, _, pressure)
        | MidiMessage::ChannelPressure(_, pressure) => Some(ControlMessage::Pressure {
            value: u8::from(pressure) as f32 / 127.0,
        }),
        MidiMessage::ControlChange(_, controller, value) => {
            let controller = u8::from(controller);
            let value = u8::from(value);
            Some(match controller {
                1 => ControlMessage::ModWheel {
                    value: value as f32 / 127.0,
                },
                64 => ControlMessage::SustainPedal {
                    pressed: value >= 64,
                },
                120 | 123 => ControlMessage::AllNotesOff,
                _ => ControlMessage::ControlChange {
                    controller,
                    value: value as f32 / 127.0,
                },
            })
        }
        _ => None,
    }
}

/// Translate one MIDI message, including stateful Rev2 NRPN sequences.
pub fn message_to_controls(
    message: MidiMessage<'_>,
    decoder: &mut rev2::ControllerDecoder,
    mut emit: impl FnMut(ControlMessage),
) {
    if let MidiMessage::ControlChange(channel, controller, value) = message {
        let controller = u8::from(controller);
        let value = u8::from(value);
        if !matches!(controller, 1 | 64 | 120 | 123) {
            if decoder.control_change(channel.index(), controller, value, |update| match update {
                rev2::MidiUpdate::Param {
                    target,
                    param,
                    value,
                } => emit(ControlMessage::SetParam {
                    target,
                    param,
                    value,
                }),
                rev2::MidiUpdate::MidiClockMode(mode) => {
                    emit(ControlMessage::SetMidiClockMode(mode));
                }
                rev2::MidiUpdate::MasterVolume(volume) => {
                    emit(ControlMessage::SetMasterVolume(volume));
                }
                rev2::MidiUpdate::Modulation {
                    target,
                    route,
                    parameter,
                } => emit(ControlMessage::SetModulationParam {
                    target,
                    route,
                    parameter,
                }),
                rev2::MidiUpdate::EditLayer(layer) => emit(ControlMessage::SetEditLayer(layer)),
            }) {
                return;
            }
        }
    }
    if let Some(command) = message_to_control(message) {
        emit(command);
    }
}

#[cfg(test)]
mod tests {
    use synth_core::midi::rev2;
    use synth_core::{ControlMessage, LayerId, LayerTarget, ParamId};
    use wmidi::MidiMessage;

    use super::message_to_controls;

    fn decode_sequence(bytes: &[[u8; 3]]) -> Option<ControlMessage> {
        let mut decoder = rev2::ControllerDecoder::default();
        let mut command = None;
        for bytes in bytes {
            message_to_controls(
                MidiMessage::try_from(bytes.as_slice()).expect("valid MIDI message"),
                &mut decoder,
                |next| command = Some(next),
            );
        }
        command
    }

    #[test]
    fn layer_b_nrpn_reaches_the_engine_with_an_explicit_target() {
        let command = decode_sequence(&[
            [0xb0, 99, 16],
            [0xb0, 98, 16],
            [0xb0, 6, 0],
            [0xb0, 38, 127],
        ]);
        assert!(matches!(
            command,
            Some(ControlMessage::SetParam {
                target: LayerTarget::Explicit(LayerId::B),
                param: ParamId::FilterResonance,
                value: 1.0,
            })
        ));
    }

    #[test]
    fn edit_layer_nrpn_reaches_the_engine_as_topology_control() {
        let command =
            decode_sequence(&[[0xb0, 99, 32], [0xb0, 98, 94], [0xb0, 6, 0], [0xb0, 38, 1]]);
        assert!(matches!(
            command,
            Some(ControlMessage::SetEditLayer(LayerId::B))
        ));
    }
}
