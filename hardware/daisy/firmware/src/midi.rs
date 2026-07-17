//! USB-MIDI transport and its application-facing event boundary.

use wmidi::{FromBytesError, MidiMessage};

use synth_core::REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN;

const SYSEX_CAPACITY: usize = REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN;

/// Temporary USB vendor ID used only for local development.
///
/// This identity is not globally assigned and must be replaced before any
/// firmware or hardware is distributed.
pub const DEVELOPMENT_VID: u16 = 0xc0de;

/// Temporary USB product ID used only for local development.
pub const DEVELOPMENT_PID: u16 = 0xcafe;

const MANUFACTURER: &str = "chris-zen";
const PRODUCT: &str = "Analog Synth USB MIDI (development)";
const CONTROL_BUFFER_SIZE: usize = 128;

/// Failure while converting a USB-MIDI event stream into MIDI messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// The endpoint currently exposes only cable zero.
    UnsupportedCable(u8),
    /// USB-MIDI CIN values zero and one are reserved.
    UnsupportedCodeIndex(u8),
    /// A SysEx continuation arrived without a preceding `0xf0` start byte.
    UnexpectedSysExContinuation,
    /// A new SysEx start arrived before the current message ended.
    NestedSysExStart,
    /// The fixed-capacity SysEx assembly buffer was exhausted.
    SysExTooLong,
    /// `wmidi` rejected the assembled MIDI bytes.
    InvalidMessage(FromBytesError),
}

/// One USB-MIDI 1.0 event packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MidiEventPacket([u8; 4]);

impl MidiEventPacket {
    pub const fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }

    pub const fn cable_number(&self) -> u8 {
        self.0[0] >> 4
    }

    pub const fn code_index_number(&self) -> u8 {
        self.0[0] & 0x0f
    }

    pub const fn midi_bytes(&self) -> [u8; 3] {
        [self.0[1], self.0[2], self.0[3]]
    }
}

/// Application-facing boundary for typed MIDI messages.
///
/// Implementations run synchronously in the USB receive task and must not
/// block. A future synth handler should translate relevant messages into
/// `synth_core::ControlMessage` values and enqueue them for the audio task.
pub trait MidiMessageHandler {
    fn handle(&mut self, cable: u8, message: MidiMessage<'_>);

    fn handle_sysex(&mut self, cable: u8, message: &[u8]) {
        let _ = (cable, message);
    }

    fn decode_error(&mut self, cable: u8, error: DecodeError) {
        let _ = (cable, error);
    }
}

/// Synchronous, allocation-free boundary between USB transport and MIDI
/// behavior.
///
/// A future synth integration can replace the logging implementation with a
/// decoder that pushes `synth_core::ControlMessage` values into a bounded
/// queue. Handlers must remain non-blocking.
pub trait MidiEventHandler {
    fn handle(&mut self, event: MidiEventPacket);
}

/// Stateful USB-MIDI event decoder backed by `wmidi`.
///
/// It handles the USB code-index-number lengths and reassembles fragmented
/// SysEx messages without allocation.
pub struct WmidiDecoder<H> {
    handler: H,
    sysex: [u8; SYSEX_CAPACITY],
    sysex_len: usize,
}

impl<H: MidiMessageHandler> WmidiDecoder<H> {
    pub const fn new(handler: H) -> Self {
        Self {
            handler,
            sysex: [0; SYSEX_CAPACITY],
            sysex_len: 0,
        }
    }

    pub fn into_inner(self) -> H {
        self.handler
    }

    fn decode(&mut self, cable: u8, bytes: &[u8]) {
        match MidiMessage::try_from(bytes) {
            Ok(message) => self.handler.handle(cable, message),
            Err(error) => self
                .handler
                .decode_error(cable, DecodeError::InvalidMessage(error)),
        }
    }

    fn append_sysex(&mut self, cable: u8, bytes: &[u8], ends_message: bool) {
        if self.sysex_len == 0 {
            if bytes.first() != Some(&0xf0) {
                self.handler
                    .decode_error(cable, DecodeError::UnexpectedSysExContinuation);
                return;
            }
        } else if bytes.first() == Some(&0xf0) {
            self.sysex_len = 0;
            self.handler
                .decode_error(cable, DecodeError::NestedSysExStart);
        }

        let Some(end) = self.sysex_len.checked_add(bytes.len()) else {
            self.sysex_len = 0;
            self.handler.decode_error(cable, DecodeError::SysExTooLong);
            return;
        };
        if end > self.sysex.len() {
            self.sysex_len = 0;
            self.handler.decode_error(cable, DecodeError::SysExTooLong);
            return;
        }

        self.sysex[self.sysex_len..end].copy_from_slice(bytes);
        self.sysex_len = end;

        if ends_message {
            self.handler
                .handle_sysex(cable, &self.sysex[..self.sysex_len]);
            self.sysex_len = 0;
        }
    }
}

impl<H: MidiMessageHandler> MidiEventHandler for WmidiDecoder<H> {
    fn handle(&mut self, event: MidiEventPacket) {
        let cable = event.cable_number();
        if cable != 0 {
            self.handler
                .decode_error(cable, DecodeError::UnsupportedCable(cable));
            return;
        }

        let bytes = event.midi_bytes();
        match event.code_index_number() {
            0x2 => self.decode(cable, &bytes[..2]),
            0x3 => self.decode(cable, &bytes),
            0x4 => self.append_sysex(cable, &bytes, false),
            0x5 => {
                if self.sysex_len != 0 || bytes[0] == 0xf0 || bytes[0] == 0xf7 {
                    self.append_sysex(cable, &bytes[..1], true);
                } else {
                    self.decode(cable, &bytes[..1]);
                }
            }
            0x6 => self.append_sysex(cable, &bytes[..2], true),
            0x7 => self.append_sysex(cable, &bytes, true),
            0x8..=0xb | 0xe => self.decode(cable, &bytes),
            0xc..=0xd => self.decode(cable, &bytes[..2]),
            0xf => self.decode(cable, &bytes[..1]),
            cin => self
                .handler
                .decode_error(cable, DecodeError::UnsupportedCodeIndex(cin)),
        }
    }
}

/// Deliver all complete four-byte events and return the number of malformed
/// trailing bytes.
fn dispatch_events(bytes: &[u8], handler: &mut impl MidiEventHandler) -> usize {
    let complete_len = bytes.len() - bytes.len() % 4;
    for chunk in bytes[..complete_len].chunks_exact(4) {
        handler.handle(MidiEventPacket::new([
            chunk[0], chunk[1], chunk[2], chunk[3],
        ]));
    }
    bytes.len() - complete_len
}

pub async fn run(
    resources: embassy_daisy::usb::UsbResources,
    handler: impl MidiMessageHandler,
) -> ! {
    use embassy_futures::join::join;
    use embassy_usb::class::midi::MidiClass;
    use embassy_usb::driver::EndpointError;
    use embassy_usb::{Builder, Config};

    let mut endpoint_out_buffer = [0u8; 256];
    let driver = resources.driver(&mut endpoint_out_buffer);

    let mut config = Config::new(DEVELOPMENT_VID, DEVELOPMENT_PID);
    // Embassy enables interface association descriptors by default. The USB-IF
    // IAD convention requires this class triplet on the device descriptor;
    // MidiClass supplies the Audio/MIDI class values on its own interfaces.
    // Keeping IAD mode also leaves room for additional USB functions later.
    config.device_class = 0xef;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.manufacturer = Some(MANUFACTURER);
    config.product = Some(PRODUCT);
    config.serial_number = None;
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    let mut config_descriptor = [0u8; 256];
    let mut bos_descriptor = [0u8; 256];
    let mut msos_descriptor = [];
    // String descriptors are UTF-16LE with a two-byte header. Keep enough
    // room for the complete product string, not only an EP0 packet.
    let mut control_buffer = [0u8; CONTROL_BUFFER_SIZE];

    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut msos_descriptor,
        &mut control_buffer,
    );
    let mut class = MidiClass::new(&mut builder, 1, 1, 64);
    let mut device = builder.build();

    let device_fut = device.run();
    let receive_fut = async {
        let mut decoder = WmidiDecoder::new(handler);
        let mut packet = [0u8; 64];
        loop {
            class.wait_connection().await;
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
                        crate::diagnostics::emit(crate::diagnostics::Event::UsbMidiDisconnected);
                        break;
                    }
                    Err(EndpointError::BufferOverflow) => {
                        crate::diagnostics::emit(crate::diagnostics::Event::UsbMidiBufferOverflow);
                        break;
                    }
                }
            }
        }
    };

    join(device_fut, receive_fut).await;
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::{
        CONTROL_BUFFER_SIZE, DecodeError, MidiEventHandler, MidiEventPacket, MidiMessageHandler,
        PRODUCT, SYSEX_CAPACITY, WmidiDecoder, dispatch_events,
    };
    use wmidi::MidiMessage;

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

    impl MidiMessageHandler for DecodedCollector {
        fn handle(&mut self, cable: u8, message: MidiMessage<'_>) {
            let mut bytes = [0; SYSEX_CAPACITY];
            let length = message.copy_to_slice(&mut bytes).unwrap();
            self.messages.push((cable, bytes[..length].to_vec()));
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
    fn wmidi_decoder_uses_cin_message_lengths() {
        let mut decoder = WmidiDecoder::new(DecodedCollector::default());
        decoder.handle(MidiEventPacket::new([0x09, 0x90, 60, 100]));
        decoder.handle(MidiEventPacket::new([0x0c, 0xc0, 12, 0]));

        assert_eq!(
            decoder.handler.messages,
            [(0, std::vec![0x90, 60, 100]), (0, std::vec![0xc0, 12])]
        );
        assert!(decoder.handler.errors.is_empty());
    }

    #[test]
    fn wmidi_decoder_reassembles_fragmented_sysex() {
        let mut decoder = WmidiDecoder::new(DecodedCollector::default());
        decoder.handle(MidiEventPacket::new([0x04, 0xf0, 0x7e, 0x7f]));
        decoder.handle(MidiEventPacket::new([0x04, 0x06, 0x01, 0x02]));
        decoder.handle(MidiEventPacket::new([0x06, 0x03, 0xf7, 0]));

        assert_eq!(
            decoder.handler.messages,
            [(0, std::vec![0xf0, 0x7e, 0x7f, 0x06, 0x01, 0x02, 0x03, 0xf7])]
        );
        assert!(decoder.handler.errors.is_empty());
    }

    #[test]
    fn wmidi_decoder_reassembles_full_rev2_edit_buffer() {
        let mut message = [0_u8; synth_core::REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        synth_core::Rev2MidiEncoder::program_edit_buffer(
            &synth_core::Patch::default(),
            &mut message,
        )
        .unwrap();
        let mut decoder = WmidiDecoder::new(DecodedCollector::default());
        let complete = message.len() - 1;
        for chunk in message[..complete].chunks_exact(3) {
            decoder.handle(MidiEventPacket::new([0x04, chunk[0], chunk[1], chunk[2]]));
        }
        decoder.handle(MidiEventPacket::new([0x05, 0xf7, 0, 0]));

        assert_eq!(decoder.handler.messages, [(0, message.to_vec())]);
        assert!(decoder.handler.errors.is_empty());
    }
}
