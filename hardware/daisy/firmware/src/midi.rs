//! USB-MIDI transport and its application-facing event boundary.

use embassy_daisy::usb::{
    Builder, Config, EndpointError,
    midi::{Decoder, MessageHandler, MidiClass, dispatch_events},
};
use embassy_executor::InterruptExecutor;
use embassy_stm32::interrupt;

use synth_core::midi::rev2::PROGRAM_DATA_SYSEX_LEN;

use crate::{
    audio::{ControlQueue, PatchQueue, PerformanceQueue},
    pending_releases::PendingReleases,
    program::ProgramStorageQueue,
    synth::SynthMidiHandler,
};

// Use the larger stored-program envelope even though the firmware currently
// applies only Program Edit Buffer dumps. This keeps transport assembly
// independent of the Rev2 command decoded at the application boundary.
const SYSEX_CAPACITY: usize = PROGRAM_DATA_SYSEX_LEN;

/// Temporary USB vendor ID used only for local development.
///
/// This identity is not globally assigned and must be replaced before any
/// firmware or hardware is distributed.
pub const DEVELOPMENT_VID: u16 = 0xc0de;

/// Temporary USB product ID used only for local development.
pub const DEVELOPMENT_PID: u16 = 0xcafe;

const MANUFACTURER: &str = "chris-zen";
const PRODUCT: &str = "Noctum (development)";
const CONTROL_BUFFER_SIZE: usize = 128;

static EXECUTOR: InterruptExecutor = InterruptExecutor::new();

pub fn spawn(
    resources: embassy_daisy::usb::UsbResources,
    controls: &'static ControlQueue,
    performance: &'static PerformanceQueue,
    pending_releases: &'static PendingReleases,
    patches: &'static PatchQueue,
    storage: &'static ProgramStorageQueue,
    initial_bank: u8,
    indicator: crate::indicator::Sender<'static>,
    audio_buffer: &'static crate::usb_audio::UsbAudioBuffer,
) -> Result<(), embassy_executor::SpawnError> {
    EXECUTOR.start(interrupt::I2C4_ER).spawn(run_task(
        resources,
        controls,
        performance,
        pending_releases,
        patches,
        storage,
        initial_bank,
        indicator,
        audio_buffer,
    )?);
    Ok(())
}

#[embassy_executor::task]
async fn run_task(
    resources: embassy_daisy::usb::UsbResources,
    controls: &'static ControlQueue,
    performance: &'static PerformanceQueue,
    pending_releases: &'static PendingReleases,
    patches: &'static PatchQueue,
    storage: &'static ProgramStorageQueue,
    initial_bank: u8,
    indicator: crate::indicator::Sender<'static>,
    audio_buffer: &'static crate::usb_audio::UsbAudioBuffer,
) -> ! {
    let handler = SynthMidiHandler::new(
        controls,
        performance,
        pending_releases,
        patches.sender(),
        indicator,
        storage,
        initial_bank,
    );
    run(resources, handler, audio_buffer).await
}

async fn run<H>(
    resources: embassy_daisy::usb::UsbResources,
    handler: H,
    audio_buffer: &'static crate::usb_audio::UsbAudioBuffer,
) -> !
where
    H: MessageHandler,
{
    use embassy_daisy::usb::audio::{
        AudioConfig, Channel, HostBinding, InputTerminalType, Microphone, SampleWidth,
        State as AudioState,
    };
    use embassy_futures::join::join3;

    let mut endpoint_out_buffer = [0u8; 256];
    let driver = resources.driver(&mut endpoint_out_buffer);

    let mut config = Config::new(DEVELOPMENT_VID, DEVELOPMENT_PID);
    // Embassy enables interface association descriptors by default. The USB-IF
    // IAD convention requires this class triplet on the device descriptor;
    // MidiClass supplies the Audio/MIDI class values on its own interfaces.
    // Keep IAD mode for the separate MIDI and audio functions. Together they
    // consume Embassy USB's four default interface slots; another function
    // must raise the max_interface_count compile-time setting.
    config.device_class = 0xef;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.manufacturer = Some(MANUFACTURER);
    config.product = Some(PRODUCT);
    config.serial_number = None;
    config.device_release = if cfg!(feature = "usb-audio-raw-test") {
        0x00f0
    } else {
        0x0025
    };
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    let mut config_descriptor = [0u8; 512];
    let mut bos_descriptor = [0u8; 256];
    let mut msos_descriptor = [];
    // String descriptors are UTF-16LE with a two-byte header. Keep enough
    // room for the complete product string, not only an EP0 packet.
    let mut control_buffer = [0u8; CONTROL_BUFFER_SIZE];

    // Class state must outlive the builder because the builder stores its
    // control handler until `build` transfers that handler into the device.
    let mut audio_state = AudioState::new();
    let audio_sample_rates = [crate::usb_audio::SAMPLE_RATE_HZ as u32];
    let audio_channels = [Channel::LeftFront, Channel::RightFront];
    let audio_config = AudioConfig::new(
        crate::usb_audio::MAX_PACKET_BYTES as u16,
        SampleWidth::Bits24,
        &audio_sample_rates,
        &audio_channels,
        InputTerminalType::LineConnector,
        if cfg!(feature = "usb-audio-raw-test") {
            HostBinding::VendorSpecific
        } else {
            HostBinding::AudioClass
        },
    );
    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut msos_descriptor,
        &mut control_buffer,
    );
    let mut class = MidiClass::new(&mut builder, 1, 1, 64);
    let mut audio_stream = match audio_config {
        Ok(config) => Some(Microphone::new(&mut builder, &mut audio_state, config)),
        Err(error) => {
            crate::diagnostics::emit(crate::diagnostics::Event::UsbAudioConfigurationInvalid {
                reason: error.category(),
            });
            None
        }
    };
    let mut device = builder.build();

    let device_fut = device.run();
    let receive_fut = async {
        let mut decoder = Decoder::<_, SYSEX_CAPACITY>::new(handler);
        let mut packet = [0u8; 64];
        loop {
            class.wait_connection().await;
            decoder.reset();
            crate::diagnostics::emit(crate::diagnostics::Event::UsbMidiConnected);

            loop {
                match class.read_packet(&mut packet).await {
                    Ok(length) => {
                        let trailing = dispatch_events(&packet[..length], &mut decoder);
                        if trailing != 0 {
                            crate::diagnostics::emit(
                                crate::diagnostics::Event::UsbMidiTrailingBytes {
                                    count: trailing as u8,
                                },
                            );
                        }
                    }
                    Err(EndpointError::Disabled) => {
                        decoder.reset();
                        crate::diagnostics::emit(crate::diagnostics::Event::UsbMidiDisconnected);
                        break;
                    }
                    Err(EndpointError::BufferOverflow) => {
                        decoder.reset();
                        crate::diagnostics::emit(crate::diagnostics::Event::UsbMidiBufferOverflow);
                        break;
                    }
                }
            }
        }
    };

    let audio_fut = async {
        match audio_stream.as_mut() {
            Some(stream) => crate::usb_audio::run(stream, audio_buffer).await,
            None => core::future::pending().await,
        }
    };
    join3(device_fut, receive_fut, audio_fut).await;
    unreachable!()
}

// I2C4 is not used by the Daisy BSP. Its error vector is reserved as a
// software-pended executor below deadline-critical audio but above thread-mode
// diagnostics and UI work.
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
unsafe extern "C" fn I2C4_ER() {
    unsafe { EXECUTOR.on_interrupt() }
}

#[cfg(test)]
mod tests {
    use super::{CONTROL_BUFFER_SIZE, PRODUCT, SYSEX_CAPACITY};
    use embassy_daisy::usb::midi::{
        DecodeError, Decoder, MessageHandler, MidiEventHandler, MidiEventPacket, dispatch_events,
    };
    use synth_core::midi::rev2::{PROGRAM_EDIT_BUFFER_SYSEX_LEN, encode};

    #[derive(Default)]
    struct Collector {
        events: std::vec::Vec<MidiEventPacket>,
    }

    impl MidiEventHandler for Collector {
        fn handle(&mut self, event: MidiEventPacket) {
            self.events.push(event);
        }
    }

    #[derive(Default)]
    struct DecodedCollector {
        messages: std::vec::Vec<(u8, std::vec::Vec<u8>)>,
        errors: std::vec::Vec<(u8, DecodeError)>,
    }

    impl MessageHandler for DecodedCollector {
        fn handle_message(&mut self, cable: u8, message: &[u8]) {
            self.messages.push((cable, message.to_vec()));
        }

        fn decode_error(&mut self, cable: u8, error: DecodeError) {
            self.errors.push((cable, error));
        }

        fn handle_sysex(&mut self, cable: u8, message: &[u8]) {
            self.messages.push((cable, message.to_vec()));
        }
    }

    #[test]
    fn event_exposes_header_and_midi_bytes() {
        let event = MidiEventPacket::new([0x19, 0x90, 60, 100]);
        assert_eq!(event.cable_number(), 1);
        assert_eq!(event.code_index_number(), 9);
        assert_eq!(event.midi_bytes(), [0x90, 60, 100]);
    }

    #[test]
    fn dispatches_complete_events_and_reports_trailing_bytes() {
        let mut collector = Collector::default();
        let trailing = dispatch_events(
            &[0x09, 0x90, 60, 100, 0x08, 0x80, 60, 0, 0xff],
            &mut collector,
        );
        assert_eq!(collector.events.len(), 2);
        assert_eq!(collector.events[0].as_bytes(), &[0x09, 0x90, 60, 100]);
        assert_eq!(collector.events[1].as_bytes(), &[0x08, 0x80, 60, 0]);
        assert_eq!(trailing, 1);
    }

    #[test]
    fn control_buffer_holds_longest_string_descriptor() {
        let descriptor_length = 2 + PRODUCT.encode_utf16().count() * 2;
        assert!(descriptor_length <= CONTROL_BUFFER_SIZE);
    }

    #[test]
    fn decoder_uses_cin_message_lengths() {
        let mut decoder = Decoder::<_, SYSEX_CAPACITY>::new(DecodedCollector::default());
        decoder.handle(MidiEventPacket::new([0x09, 0x90, 60, 100]));
        decoder.handle(MidiEventPacket::new([0x0c, 0xc0, 12, 0]));

        assert_eq!(
            decoder.handler().messages,
            [(0, std::vec![0x90, 60, 100]), (0, std::vec![0xc0, 12])]
        );
        assert!(decoder.handler().errors.is_empty());
    }

    #[test]
    fn decoder_reassembles_fragmented_sysex() {
        let mut decoder = Decoder::<_, SYSEX_CAPACITY>::new(DecodedCollector::default());
        decoder.handle(MidiEventPacket::new([0x04, 0xf0, 0x7e, 0x7f]));
        decoder.handle(MidiEventPacket::new([0x04, 0x06, 0x01, 0x02]));
        decoder.handle(MidiEventPacket::new([0x06, 0x03, 0xf7, 0]));

        assert_eq!(
            decoder.handler().messages,
            [(0, std::vec![0xf0, 0x7e, 0x7f, 0x06, 0x01, 0x02, 0x03, 0xf7])]
        );
        assert!(decoder.handler().errors.is_empty());
    }

    #[test]
    fn decoder_preserves_sysex_around_realtime_clock() {
        let mut decoder = Decoder::<_, SYSEX_CAPACITY>::new(DecodedCollector::default());
        decoder.handle(MidiEventPacket::new([0x04, 0xf0, 0x7e, 0x7f]));
        decoder.handle(MidiEventPacket::new([0x0f, 0xf8, 0, 0]));
        decoder.handle(MidiEventPacket::new([0x06, 0x01, 0xf7, 0]));

        assert_eq!(
            decoder.handler().messages,
            [
                (0, std::vec![0xf8]),
                (0, std::vec![0xf0, 0x7e, 0x7f, 0x01, 0xf7])
            ]
        );
        assert!(decoder.handler().errors.is_empty());
    }

    #[test]
    fn decoder_reassembles_full_rev2_edit_buffer() {
        let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        encode::program_edit_buffer(&synth_core::Patch::default(), &mut message).unwrap();
        let mut decoder = Decoder::<_, SYSEX_CAPACITY>::new(DecodedCollector::default());
        let complete = message.len() - 1;
        for chunk in message[..complete].chunks_exact(3) {
            decoder.handle(MidiEventPacket::new([0x04, chunk[0], chunk[1], chunk[2]]));
        }
        decoder.handle(MidiEventPacket::new([0x05, 0xf7, 0, 0]));

        assert_eq!(decoder.handler().messages, [(0, message.to_vec())]);
        assert!(decoder.handler().errors.is_empty());
    }

    #[test]
    fn decoder_waits_for_detached_f7_after_usb_end_marker() {
        let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        encode::program_edit_buffer(&synth_core::Patch::default(), &mut message).unwrap();
        let mut decoder = Decoder::<_, SYSEX_CAPACITY>::new(DecodedCollector::default());
        let chunks = message[..message.len() - 1].chunks_exact(3);
        let chunk_count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            let cin = if index + 1 == chunk_count { 0x07 } else { 0x04 };
            decoder.handle(MidiEventPacket::new([cin, chunk[0], chunk[1], chunk[2]]));
        }

        assert!(decoder.handler().messages.is_empty());
        assert!(decoder.handler().errors.is_empty());

        // The detached terminator completes the still-active assembly.
        decoder.handle(MidiEventPacket::new([0x0f, 0xf7, 0, 0]));

        assert_eq!(decoder.handler().messages, [(0, message.to_vec())]);
        assert!(decoder.handler().errors.is_empty());
    }

    #[test]
    fn decoder_keeps_single_cin_f_data_byte_in_active_sysex() {
        let mut message = [0_u8; PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        encode::program_edit_buffer(&synth_core::Patch::default(), &mut message).unwrap();
        let mut decoder = Decoder::<_, SYSEX_CAPACITY>::new(DecodedCollector::default());

        decoder.handle(MidiEventPacket::new([
            0x04, message[0], message[1], message[2],
        ]));
        decoder.handle(MidiEventPacket::new([0x0f, message[3], 0, 0]));
        let chunks = message[4..].chunks_exact(3);
        let chunk_count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            let cin = if index + 1 == chunk_count { 0x07 } else { 0x04 };
            decoder.handle(MidiEventPacket::new([cin, chunk[0], chunk[1], chunk[2]]));
        }

        assert_eq!(decoder.handler().messages, [(0, message.to_vec())]);
        assert!(decoder.handler().errors.is_empty());
    }
}
