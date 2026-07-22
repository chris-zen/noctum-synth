//! Allocation-free USB-MIDI 1.0 transport helpers.

pub use embassy_usb::class::midi::MidiClass;

/// Failure while converting a USB-MIDI event stream into complete MIDI bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// The decoder currently exposes only cable zero.
    UnsupportedCable(u8),
    /// USB-MIDI CIN values zero and one are reserved.
    UnsupportedCodeIndex(u8),
    /// A SysEx continuation arrived without a preceding `0xf0` start byte.
    UnexpectedSysExContinuation,
    /// A new SysEx start arrived before the current message ended.
    NestedSysExStart,
    /// The caller-selected fixed-capacity SysEx buffer was exhausted.
    SysExTooLong,
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

/// Synchronous application boundary for complete raw MIDI messages.
///
/// Implementations run in the USB receive task and must not block. Channel,
/// system-common, and realtime messages are delivered with their USB CIN
/// padding removed. SysEx messages include their `f0` and `f7` delimiters.
pub trait MessageHandler {
    fn handle_message(&mut self, cable: u8, message: &[u8]);

    fn handle_sysex(&mut self, cable: u8, message: &[u8]) {
        let _ = (cable, message);
    }

    fn decode_error(&mut self, cable: u8, error: DecodeError) {
        let _ = (cable, error);
    }
}

/// Synchronous boundary for raw USB-MIDI event packets.
pub trait MidiEventHandler {
    fn handle(&mut self, event: MidiEventPacket);
}

/// Stateful USB-MIDI event decoder.
///
/// It handles CIN message lengths and reassembles fragmented SysEx messages in
/// a caller-sized, fixed-capacity buffer without allocation.
pub struct Decoder<H, const SYSEX_CAPACITY: usize> {
    handler: H,
    sysex: [u8; SYSEX_CAPACITY],
    sysex_len: usize,
}

impl<H: MessageHandler, const SYSEX_CAPACITY: usize> Decoder<H, SYSEX_CAPACITY> {
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

    /// Borrow the application handler without disturbing decoder state.
    pub const fn handler(&self) -> &H {
        &self.handler
    }

    /// Discard a partially assembled SysEx message after disconnect or reset.
    pub fn reset(&mut self) {
        self.sysex_len = 0;
    }

    fn deliver(&mut self, cable: u8, bytes: &[u8]) {
        self.handler.handle_message(cable, bytes);
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

        // F7, rather than a transfer boundary or CIN alone, terminates SysEx.
        if ends_message && self.sysex.get(self.sysex_len.wrapping_sub(1)) == Some(&0xf7) {
            self.handler
                .handle_sysex(cable, &self.sysex[..self.sysex_len]);
            self.sysex_len = 0;
        }
    }
}

impl<H: MessageHandler, const SYSEX_CAPACITY: usize> MidiEventHandler
    for Decoder<H, SYSEX_CAPACITY>
{
    fn handle(&mut self, event: MidiEventPacket) {
        let cable = event.cable_number();
        if cable != 0 {
            self.handler
                .decode_error(cable, DecodeError::UnsupportedCable(cable));
            return;
        }

        let bytes = event.midi_bytes();
        match event.code_index_number() {
            0x2 => self.deliver(cable, &bytes[..2]),
            0x3 => self.deliver(cable, &bytes),
            0x4 => self.append_sysex(cable, &bytes, false),
            0x5 => {
                if self.sysex_len != 0 || bytes[0] == 0xf0 || bytes[0] == 0xf7 {
                    self.append_sysex(cable, &bytes[..1], true);
                } else {
                    self.deliver(cable, &bytes[..1]);
                }
            }
            0x6 => self.append_sysex(cable, &bytes[..2], true),
            0x7 => self.append_sysex(cable, &bytes, true),
            0x8..=0xb | 0xe => self.deliver(cable, &bytes),
            0xc..=0xd => self.deliver(cable, &bytes[..2]),
            0xf if bytes[0] == 0xf7 => {
                if self.sysex_len != 0 {
                    self.append_sysex(cable, &bytes[..1], true);
                }
            }
            // CoreMIDI can emit a single 7-bit SysEx data byte using CIN F.
            0xf if self.sysex_len != 0 && bytes[0] < 0x80 => {
                self.append_sysex(cable, &bytes[..1], false);
            }
            0xf => self.deliver(cable, &bytes[..1]),
            cin => self
                .handler
                .decode_error(cable, DecodeError::UnsupportedCodeIndex(cin)),
        }
    }
}

/// Deliver complete four-byte events and return the malformed trailing length.
pub fn dispatch_events(bytes: &[u8], handler: &mut impl MidiEventHandler) -> usize {
    let complete_len = bytes.len() - bytes.len() % 4;
    for chunk in bytes[..complete_len].chunks_exact(4) {
        handler.handle(MidiEventPacket::new([
            chunk[0], chunk[1], chunk[2], chunk[3],
        ]));
    }
    bytes.len() - complete_len
}
