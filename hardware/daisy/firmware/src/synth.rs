//! Translation from typed MIDI messages to synthesizer control commands.

use wmidi::MidiMessage;

use embassy_daisy::usb::midi::DecodeError;

use synth_core::{
    ControlMessage, Patch,
    midi::{p08, rev2},
};

use crate::{
    midi_control::{message_to_controls, realtime_to_control},
    program::{ProgramStorageQueue, ProgramStorageRequest, selection::ProgramSelection},
};

pub struct SynthMidiHandler<'a, const PATCH_CAPACITY: usize> {
    controls: &'a crate::audio::ControlQueue,
    performance: &'a crate::audio::PerformanceQueue,
    pending_releases: &'a crate::pending_releases::PendingReleases,
    patches: embassy_sync::channel::Sender<
        'a,
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        Patch,
        PATCH_CAPACITY,
    >,
    indicator: crate::indicator::Sender<'a>,
    storage: &'a ProgramStorageQueue,
    program_selection: ProgramSelection,
    decoder: rev2::ControllerDecoder,
    #[cfg(feature = "diagnostics")]
    nrpn_monitor: NrpnMonitor,
}

impl<'a, const PATCH_CAPACITY: usize> SynthMidiHandler<'a, PATCH_CAPACITY> {
    pub fn new(
        controls: &'a crate::audio::ControlQueue,
        performance: &'a crate::audio::PerformanceQueue,
        pending_releases: &'a crate::pending_releases::PendingReleases,
        patches: embassy_sync::channel::Sender<
            'a,
            embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
            Patch,
            PATCH_CAPACITY,
        >,
        indicator: crate::indicator::Sender<'a>,
        storage: &'a crate::program::ProgramStorageQueue,
        initial_bank: u8,
    ) -> Self {
        Self {
            controls,
            performance,
            pending_releases,
            patches,
            indicator,
            storage,
            program_selection: ProgramSelection::new(initial_bank),
            decoder: rev2::ControllerDecoder::default(),
            #[cfg(feature = "diagnostics")]
            nrpn_monitor: NrpnMonitor::default(),
        }
    }

    fn enqueue_storage(&self, request: ProgramStorageRequest) -> bool {
        self.storage.try_send(request).is_ok()
    }

    fn enqueue(&self, command: ControlMessage) {
        enqueue_command(
            self.controls,
            self.performance,
            self.pending_releases,
            command,
        );
    }
}

impl<const PATCH_CAPACITY: usize> embassy_daisy::usb::midi::MessageHandler
    for SynthMidiHandler<'_, PATCH_CAPACITY>
{
    fn handle_message(&mut self, cable: u8, bytes: &[u8]) {
        self.indicator.notify_midi();
        let message = match MidiMessage::try_from(bytes) {
            Ok(message) => message,
            Err(_) => {
                crate::diagnostics::emit(crate::diagnostics::Event::InvalidMidi {
                    cable,
                    reason: crate::diagnostics::InvalidMidiReason::InvalidMessage,
                    length: bytes.len() as u16,
                });
                return;
            }
        };

        if let Some(command) =
            realtime_to_control(&message, embassy_time::Instant::now().as_micros())
        {
            self.enqueue(command);
            return;
        }

        match message {
            MidiMessage::ControlChange(_, controller, value)
                if matches!(u8::from(controller), 0 | 32) =>
            {
                self.program_selection
                    .bank_select(u8::from(controller), u8::from(value));
                return;
            }
            MidiMessage::ProgramChange(_, program) => {
                let bank = self.program_selection.requested_bank();
                let program = u8::from(program);
                let request = ProgramStorageRequest::Load { bank, program };
                if self.enqueue_storage(request) {
                    self.program_selection.commit();
                    crate::diagnostics::emit(crate::diagnostics::Event::ProgramChangeReceived {
                        bank,
                        program,
                    });
                } else {
                    crate::diagnostics::emit(crate::diagnostics::Event::ProgramStorageQueueFull);
                }
                return;
            }
            _ => {}
        }

        #[cfg(feature = "diagnostics")]
        let completed_nrpn = if let MidiMessage::ControlChange(channel, controller, value) = message
        {
            self.nrpn_monitor
                .control_change(channel.index(), u8::from(controller), u8::from(value))
                .map(|nrpn| (channel.index() + 1, nrpn))
        } else {
            None
        };

        let controls = self.controls;
        let performance = self.performance;
        let pending_releases = self.pending_releases;
        message_to_controls(message, &mut self.decoder, |command| {
            enqueue_command(controls, performance, pending_releases, command);
        });

        #[cfg(feature = "diagnostics")]
        if let Some((channel, nrpn)) = completed_nrpn {
            crate::diagnostics::emit(crate::diagnostics::Event::NrpnRx {
                channel,
                number: nrpn.number,
                value: nrpn.value,
            });
        }
    }

    fn decode_error(&mut self, cable: u8, error: DecodeError) {
        use crate::diagnostics::InvalidMidiReason;

        let reason = match error {
            DecodeError::UnsupportedCable(_) => InvalidMidiReason::UnsupportedCable,
            DecodeError::UnsupportedCodeIndex(_) => InvalidMidiReason::UnsupportedCodeIndex,
            DecodeError::UnexpectedSysExContinuation => {
                InvalidMidiReason::UnexpectedSysExContinuation
            }
            DecodeError::NestedSysExStart => InvalidMidiReason::NestedSysExStart,
            DecodeError::SysExTooLong => InvalidMidiReason::SysExTooLong,
        };
        crate::diagnostics::emit(crate::diagnostics::Event::InvalidMidi {
            cable,
            reason,
            length: 0,
        });
    }

    fn handle_sysex(&mut self, cable: u8, message: &[u8]) {
        self.indicator.notify_midi();
        if message.len() < 4 || message[0] != 0xf0 || message[1] != 0x01 {
            return;
        }
        match (message[2], message[3]) {
            (0x2f, 0x02) => match rev2::decode::program_data(message) {
                Ok(program) => {
                    let (bank, program_number) = (program.bank, program.program);
                    if !self.enqueue_storage(ProgramStorageRequest::Save {
                        bank,
                        program: program_number,
                        patch: program.patch,
                    }) {
                        crate::diagnostics::emit(
                            crate::diagnostics::Event::ProgramStorageQueueFull,
                        );
                    }
                }
                Err(error) => emit_sysex_error(cable, message.len(), error),
            },
            (0x2f, 0x03) => match rev2::decode::program_edit_buffer(message) {
                Ok(patch) => {
                    crate::diagnostics::emit(crate::diagnostics::Event::ProgramEditBufferReceived);
                    if self.patches.try_send(patch).is_err() {
                        crate::diagnostics::emit(crate::diagnostics::Event::PatchQueueFull);
                    }
                }
                Err(error) => emit_sysex_error(cable, message.len(), error),
            },
            (0x23, 0x02) => match p08::decode::program_data(message) {
                Ok(program) => {
                    let bank = program.bank + 4;
                    let program_number = program.program;
                    if !self.enqueue_storage(ProgramStorageRequest::Save {
                        bank,
                        program: program_number,
                        patch: program.patch,
                    }) {
                        crate::diagnostics::emit(
                            crate::diagnostics::Event::ProgramStorageQueueFull,
                        );
                    }
                }
                Err(error) => emit_sysex_error(cable, message.len(), error),
            },
            (0x23, 0x03) => match p08::decode::program_edit_buffer(message) {
                Ok(patch) => {
                    crate::diagnostics::emit(crate::diagnostics::Event::ProgramEditBufferReceived);
                    if self.patches.try_send(patch).is_err() {
                        crate::diagnostics::emit(crate::diagnostics::Event::PatchQueueFull);
                    }
                }
                Err(error) => emit_sysex_error(cable, message.len(), error),
            },
            _ => {
                emit_sysex_error(cable, message.len(), rev2::SysexError::UnsupportedCommand);
            }
        }
    }
}

fn emit_sysex_error(cable: u8, length: usize, error: rev2::SysexError) {
    use crate::diagnostics::InvalidMidiReason;

    let reason = match error {
        rev2::SysexError::InvalidLength => InvalidMidiReason::InvalidSysExLength,
        rev2::SysexError::InvalidFraming => InvalidMidiReason::InvalidSysExFraming,
        rev2::SysexError::InvalidManufacturer => InvalidMidiReason::InvalidSysExManufacturer,
        rev2::SysexError::InvalidModel => InvalidMidiReason::InvalidSysExModel,
        rev2::SysexError::UnsupportedCommand => InvalidMidiReason::UnsupportedSysExCommand,
        rev2::SysexError::InvalidBank => InvalidMidiReason::InvalidSysExBank,
        rev2::SysexError::NonSevenBitData => InvalidMidiReason::NonSevenBitSysExData,
        rev2::SysexError::InvalidProgramData => InvalidMidiReason::InvalidSysExProgramData,
        rev2::SysexError::OutputTooSmall => InvalidMidiReason::SysExOutputTooSmall,
    };
    crate::diagnostics::emit(crate::diagnostics::Event::InvalidMidi {
        cable,
        reason,
        length: length as u16,
    });
}

fn enqueue_command(
    controls: &crate::audio::ControlQueue,
    performance: &crate::audio::PerformanceQueue,
    pending_releases: &crate::pending_releases::PendingReleases,
    command: ControlMessage,
) {
    let is_replaceable = matches!(
        &command,
        ControlMessage::SetParam { .. }
            | ControlMessage::SetModulationParam { .. }
            | ControlMessage::SetSequence { .. }
            | ControlMessage::SetLayerMode(_)
            | ControlMessage::SetSplitPoint(_)
            | ControlMessage::SetEditLayer(_)
    );
    let result = if is_replaceable {
        controls.try_send(command)
    } else {
        performance.try_send(command)
    };

    match result {
        Ok(()) => {}
        Err(embassy_sync::channel::TrySendError::Full(command)) => {
            match command {
                ControlMessage::NoteOn { note, velocity } if velocity <= 0.0 => {
                    pending_releases.note_off(note)
                }
                ControlMessage::NoteOff { note } => pending_releases.note_off(note),
                ControlMessage::AllNotesOff => pending_releases.all_notes_off(),
                _ => {}
            }
            crate::diagnostics::emit(crate::diagnostics::Event::ControlQueueFull);
        }
    }
}

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Default)]
struct NrpnMonitorChannel {
    number_msb: Option<u8>,
    number_lsb: Option<u8>,
    data_msb: Option<u8>,
}

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompletedNrpn {
    number: u16,
    value: u16,
}

/// Firmware-only observer used solely to collapse the four transport CCs into
/// one diagnostic event. Synth parameter decoding remains in `synth-core`.
#[cfg(feature = "diagnostics")]
struct NrpnMonitor {
    channels: [NrpnMonitorChannel; 16],
}

#[cfg(feature = "diagnostics")]
impl Default for NrpnMonitor {
    fn default() -> Self {
        Self {
            channels: [NrpnMonitorChannel::default(); 16],
        }
    }
}

#[cfg(feature = "diagnostics")]
impl NrpnMonitor {
    fn control_change(&mut self, channel: u8, controller: u8, value: u8) -> Option<CompletedNrpn> {
        let state = self.channels.get_mut(usize::from(channel))?;
        match controller {
            99 => {
                state.number_msb = Some(value);
                state.data_msb = None;
            }
            98 => {
                state.number_lsb = Some(value);
                state.data_msb = None;
            }
            6 => state.data_msb = Some(value),
            38 => {
                let number = u16::from(state.number_msb?) * 128 + u16::from(state.number_lsb?);
                let value = u16::from(state.data_msb?) * 128 + u16::from(value);
                return Some(CompletedNrpn { number, value });
            }
            // Selecting an RPN cancels the diagnostic NRPN assembly. The
            // authoritative decoder independently applies its full RPN rules.
            100 | 101 => *state = NrpnMonitorChannel::default(),
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use wmidi::MidiMessage;

    use synth_core::midi::clock::{MidiClockMode, MidiRealtimeEvent};
    use synth_core::midi::rev2;
    use synth_core::{ControlMessage, LayerId, LayerTarget, ParamId};

    use crate::midi_control::{message_to_control, message_to_controls, realtime_to_control};
    #[cfg(feature = "diagnostics")]
    use crate::synth::{CompletedNrpn, NrpnMonitor};

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

    #[test]
    fn maps_supported_system_realtime_messages() {
        assert!(matches!(
            realtime_to_control(&MidiMessage::try_from([0xf8].as_slice()).unwrap(), 42),
            Some(ControlMessage::MidiRealtime(
                MidiRealtimeEvent::TimingClock {
                    timestamp_micros: 42
                }
            ))
        ));
        assert!(matches!(
            realtime_to_control(&MidiMessage::try_from([0xfa].as_slice()).unwrap(), 0),
            Some(ControlMessage::MidiRealtime(MidiRealtimeEvent::Start))
        ));
        assert!(matches!(
            realtime_to_control(&MidiMessage::try_from([0xfb].as_slice()).unwrap(), 0),
            Some(ControlMessage::MidiRealtime(MidiRealtimeEvent::Continue))
        ));
        assert!(matches!(
            realtime_to_control(&MidiMessage::try_from([0xfc].as_slice()).unwrap(), 0),
            Some(ControlMessage::MidiRealtime(MidiRealtimeEvent::Stop))
        ));
    }

    #[test]
    fn decodes_rev2_nrpn_parameter_sequences() {
        let mut decoder = rev2::ControllerDecoder::default();
        let mut command = None;
        for bytes in [[0xb0, 99, 0], [0xb0, 98, 16], [0xb0, 6, 0], [0xb0, 38, 127]] {
            message_to_controls(
                MidiMessage::try_from(bytes.as_slice()).unwrap(),
                &mut decoder,
                |next| command = Some(next),
            );
        }
        assert!(matches!(
            command,
            Some(ControlMessage::SetParam {
                target: LayerTarget::Explicit(LayerId::A),
                param: ParamId::FilterResonance,
                value: 1.0,
            })
        ));
    }

    #[test]
    fn decodes_rev2_layer_b_nrpn_parameter_sequences() {
        let mut decoder = rev2::ControllerDecoder::default();
        let mut command = None;
        for bytes in [
            [0xb0, 99, 16],
            [0xb0, 98, 16],
            [0xb0, 6, 0],
            [0xb0, 38, 127],
        ] {
            message_to_controls(
                MidiMessage::try_from(bytes.as_slice()).unwrap(),
                &mut decoder,
                |next| command = Some(next),
            );
        }
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
    fn decodes_rev2_edit_layer_nrpn() {
        let mut decoder = rev2::ControllerDecoder::default();
        let mut command = None;
        for bytes in [[0xb0, 99, 32], [0xb0, 98, 94], [0xb0, 6, 0], [0xb0, 38, 1]] {
            message_to_controls(
                MidiMessage::try_from(bytes.as_slice()).unwrap(),
                &mut decoder,
                |next| command = Some(next),
            );
        }
        assert!(matches!(
            command,
            Some(ControlMessage::SetEditLayer(LayerId::B))
        ));
    }

    #[test]
    fn decodes_rev2_global_clock_mode_nrpn() {
        let mut decoder = rev2::ControllerDecoder::default();
        let mut command = None;
        for bytes in [[0xb0, 99, 32], [0xb0, 98, 3], [0xb0, 6, 0], [0xb0, 38, 2]] {
            message_to_controls(
                MidiMessage::try_from(bytes.as_slice()).unwrap(),
                &mut decoder,
                |next| command = Some(next),
            );
        }
        assert!(matches!(
            command,
            Some(ControlMessage::SetMidiClockMode(MidiClockMode::Slave))
        ));
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn firmware_monitor_emits_one_completed_nrpn() {
        let mut monitor = NrpnMonitor::default();
        assert_eq!(monitor.control_change(0, 99, 0), None);
        assert_eq!(monitor.control_change(0, 98, 33), None);
        assert_eq!(monitor.control_change(0, 6, 0), None);
        assert_eq!(
            monitor.control_change(0, 38, 13),
            Some(CompletedNrpn {
                number: 33,
                value: 13,
            })
        );
    }
}
