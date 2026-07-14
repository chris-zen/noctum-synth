//! Translation from typed MIDI messages to synthesizer control commands.

use synth_core::ControlMessage;
use wmidi::MidiMessage;

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

#[cfg(target_arch = "arm")]
pub struct SynthMidiHandler<'a, const N: usize> {
    sender: embassy_sync::channel::Sender<
        'a,
        embassy_sync::blocking_mutex::raw::ThreadModeRawMutex,
        ControlMessage,
        N,
    >,
}

#[cfg(target_arch = "arm")]
impl<'a, const N: usize> SynthMidiHandler<'a, N> {
    pub const fn new(
        sender: embassy_sync::channel::Sender<
            'a,
            embassy_sync::blocking_mutex::raw::ThreadModeRawMutex,
            ControlMessage,
            N,
        >,
    ) -> Self {
        Self { sender }
    }
}

#[cfg(target_arch = "arm")]
impl<const N: usize> crate::midi::MidiMessageHandler for SynthMidiHandler<'_, N> {
    fn handle(&mut self, _cable: u8, message: MidiMessage<'_>) {
        if let Some(command) = message_to_control(message)
            && self.sender.try_send(command).is_err()
        {
            defmt::warn!("synth control queue full; dropping newest MIDI command");
        }
    }

    fn decode_error(&mut self, cable: u8, _error: crate::midi::DecodeError) {
        defmt::warn!("invalid MIDI message on cable {}", cable);
    }
}

#[cfg(test)]
mod tests {
    use super::message_to_control;
    use synth_core::ControlMessage;
    use wmidi::MidiMessage;

    fn command(bytes: &[u8]) -> Option<ControlMessage> {
        message_to_control(MidiMessage::try_from(bytes).unwrap())
    }

    #[test]
    fn maps_notes_and_zero_velocity_note_on() {
        match command(&[0x92, 60, 100]).unwrap() {
            ControlMessage::NoteOn { note, velocity } => {
                assert_eq!(note, 60);
                assert!((velocity - 100.0 / 127.0).abs() < f32::EPSILON);
            }
            _ => panic!("expected note on"),
        }

        assert!(matches!(
            command(&[0x90, 60, 0]),
            Some(ControlMessage::NoteOff { note: 60 })
        ));
        assert!(matches!(
            command(&[0x82, 61, 55]),
            Some(ControlMessage::NoteOff { note: 61 })
        ));
    }

    #[test]
    fn maps_pitch_bend_and_pressure() {
        match command(&[0xe0, 0, 0]).unwrap() {
            ControlMessage::PitchBend { value } => assert_eq!(value, -1.0),
            _ => panic!("expected minimum pitch bend"),
        }
        match command(&[0xe0, 0, 64]).unwrap() {
            ControlMessage::PitchBend { value } => assert!(value.abs() < 0.000_1),
            _ => panic!("expected centered pitch bend"),
        }
        match command(&[0xe0, 127, 127]).unwrap() {
            ControlMessage::PitchBend { value } => assert_eq!(value, 1.0),
            _ => panic!("expected maximum pitch bend"),
        }
        match command(&[0xd0, 127]).unwrap() {
            ControlMessage::Pressure { value } => assert_eq!(value, 1.0),
            _ => panic!("expected channel pressure"),
        }
        match command(&[0xa0, 60, 64]).unwrap() {
            ControlMessage::Pressure { value } => {
                assert!((value - 64.0 / 127.0).abs() < f32::EPSILON)
            }
            _ => panic!("expected polyphonic key pressure"),
        }
    }

    #[test]
    fn maps_channel_controllers() {
        assert!(matches!(
            command(&[0xb0, 1, 127]),
            Some(ControlMessage::ModWheel { value: 1.0 })
        ));
        assert!(matches!(
            command(&[0xb0, 64, 64]),
            Some(ControlMessage::SustainPedal { pressed: true })
        ));
        assert!(matches!(
            command(&[0xb0, 123, 0]),
            Some(ControlMessage::AllNotesOff)
        ));
        match command(&[0xb0, 11, 32]).unwrap() {
            ControlMessage::ControlChange { controller, value } => {
                assert_eq!(controller, 11);
                assert!((value - 32.0 / 127.0).abs() < f32::EPSILON);
            }
            _ => panic!("expected generic control change"),
        }
    }

    #[test]
    fn ignores_messages_without_synth_commands() {
        assert!(command(&[0xc0, 5]).is_none());
        assert!(command(&[0xf8]).is_none());
    }
}
