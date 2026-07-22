#![no_std]
#![cfg_attr(test, no_main)]
#![doc = include_str!("../README.md")]

#[cfg(not(any(
    feature = "seed",
    feature = "seed_1_1",
    feature = "seed_1_2",
    feature = "patch_sm"
)))]
compile_error!("select exactly one Daisy board feature; currently supported: `seed_1_1`");

#[cfg(any(
    all(feature = "seed", feature = "seed_1_1"),
    all(feature = "seed", feature = "seed_1_2"),
    all(feature = "seed", feature = "patch_sm"),
    all(feature = "seed_1_1", feature = "seed_1_2"),
    all(feature = "seed_1_1", feature = "patch_sm"),
    all(feature = "seed_1_2", feature = "patch_sm"),
))]
compile_error!("Daisy board features are mutually exclusive; select exactly one");

#[cfg(any(
    all(
        feature = "seed",
        not(any(feature = "seed_1_1", feature = "seed_1_2", feature = "patch_sm"))
    ),
    all(
        feature = "seed_1_2",
        not(any(feature = "seed", feature = "seed_1_1", feature = "patch_sm"))
    ),
    all(
        feature = "patch_sm",
        not(any(feature = "seed", feature = "seed_1_1", feature = "seed_1_2"))
    ),
))]
compile_error!("this Daisy board is reserved but not implemented; currently supported: `seed_1_1`");

#[cfg(feature = "sampling_rate_96khz")]
compile_error!("96 kHz audio is reserved but has not been validated yet");

#[cfg(feature = "block_length_64")]
compile_error!("64-frame audio blocks are reserved but have not been validated yet");

#[cfg(not(target_arch = "arm"))]
compile_error!("embassy-daisy targets Cortex-M7 only; build with thumbv7em-none-eabihf");

mod wm8731;

pub mod audio;
pub mod board;
pub mod clocks;
pub mod led;
mod memory;
pub mod pins;
pub mod pwm;
pub mod qspi;
pub mod sdram;
pub mod usb;

pub use board::{Board, BoardParts, TakeError};
pub use led::{PwmUserLed, UserLed, UserLedPin};
pub use pwm::{PwmChannel, PwmChannels, PwmFrequency, PwmOutput};

#[cfg(test)]
#[embedded_test::setup]
fn setup() {
    use defmt_rtt as _;
}

#[cfg(test)]
#[embedded_test::tests]
mod link_hack {
    use defmt_rtt as _;
    use embassy_stm32 as _;
    use panic_probe as _;

    use crate::usb::audio::{
        AudioConfig, Channel, ConfigError, HostBinding, InputTerminalType, SampleWidth,
    };
    use crate::usb::midi::{
        DecodeError, Decoder, MessageHandler, MidiEventHandler, MidiEventPacket, dispatch_events,
    };

    #[derive(Clone, Copy)]
    struct Record {
        bytes: [u8; 16],
        len: usize,
        sysex: bool,
    }

    impl Record {
        const EMPTY: Self = Self {
            bytes: [0; 16],
            len: 0,
            sysex: false,
        };
    }

    struct Collector {
        records: [Record; 4],
        records_len: usize,
        error: Option<DecodeError>,
    }

    impl Collector {
        const fn new() -> Self {
            Self {
                records: [Record::EMPTY; 4],
                records_len: 0,
                error: None,
            }
        }

        fn push(&mut self, bytes: &[u8], sysex: bool) {
            let record = &mut self.records[self.records_len];
            record.bytes[..bytes.len()].copy_from_slice(bytes);
            record.len = bytes.len();
            record.sysex = sysex;
            self.records_len += 1;
        }
    }

    impl MessageHandler for Collector {
        fn handle_message(&mut self, _cable: u8, message: &[u8]) {
            self.push(message, false);
        }

        fn handle_sysex(&mut self, _cable: u8, message: &[u8]) {
            self.push(message, true);
        }

        fn decode_error(&mut self, _cable: u8, error: DecodeError) {
            self.error = Some(error);
        }
    }

    #[test]
    fn embassy_stm32_linked() {}

    #[test]
    fn usb_audio_configuration_returns_errors_instead_of_panicking() {
        let rates = [48_000];
        let channels = [Channel::LeftFront, Channel::RightFront];
        assert!(
            AudioConfig::new(
                294,
                SampleWidth::Bits24,
                &rates,
                &channels,
                InputTerminalType::LineConnector,
                HostBinding::AudioClass,
            )
            .is_ok()
        );
        assert_eq!(
            AudioConfig::new(
                0,
                SampleWidth::Bits24,
                &rates,
                &channels,
                InputTerminalType::LineConnector,
                HostBinding::AudioClass,
            )
            .err(),
            Some(ConfigError::InvalidMaxPacketSize(0))
        );
        assert_eq!(
            AudioConfig::new(
                294,
                SampleWidth::Bits24,
                &[],
                &channels,
                InputTerminalType::LineConnector,
                HostBinding::AudioClass,
            )
            .err(),
            Some(ConfigError::MissingSampleRates)
        );
        assert_eq!(
            AudioConfig::new(
                288,
                SampleWidth::Bits24,
                &rates,
                &channels,
                InputTerminalType::LineConnector,
                HostBinding::AudioClass,
            )
            .err(),
            Some(ConfigError::PacketTooSmall {
                provided: 288,
                required: 294,
            })
        );
    }

    #[test]
    fn usb_midi_dispatches_complete_events() {
        struct Events {
            packets: [MidiEventPacket; 2],
            len: usize,
        }

        impl MidiEventHandler for Events {
            fn handle(&mut self, event: MidiEventPacket) {
                self.packets[self.len] = event;
                self.len += 1;
            }
        }

        let mut events = Events {
            packets: [MidiEventPacket::new([0; 4]); 2],
            len: 0,
        };
        let trailing =
            dispatch_events(&[0x09, 0x90, 60, 100, 0x08, 0x80, 60, 0, 0xff], &mut events);

        assert_eq!(events.len, 2);
        assert_eq!(events.packets[0].as_bytes(), &[0x09, 0x90, 60, 100]);
        assert_eq!(events.packets[1].as_bytes(), &[0x08, 0x80, 60, 0]);
        assert_eq!(trailing, 1);
    }

    #[test]
    fn usb_midi_delivers_cin_lengths_without_semantic_parser() {
        let mut decoder = Decoder::<_, 16>::new(Collector::new());
        decoder.handle(MidiEventPacket::new([0x09, 0x90, 60, 100]));
        decoder.handle(MidiEventPacket::new([0x0c, 0xc0, 12, 0]));

        let collector = decoder.handler();
        assert_eq!(collector.records_len, 2);
        assert_eq!(&collector.records[0].bytes[..3], &[0x90, 60, 100]);
        assert_eq!(&collector.records[1].bytes[..2], &[0xc0, 12]);
    }

    #[test]
    fn usb_midi_preserves_realtime_during_fragmented_sysex() {
        let mut decoder = Decoder::<_, 16>::new(Collector::new());
        decoder.handle(MidiEventPacket::new([0x04, 0xf0, 0x7e, 0x7f]));
        decoder.handle(MidiEventPacket::new([0x0f, 0xf8, 0, 0]));
        decoder.handle(MidiEventPacket::new([0x06, 0x01, 0xf7, 0]));

        let collector = decoder.handler();
        assert_eq!(collector.records_len, 2);
        assert_eq!(&collector.records[0].bytes[..1], &[0xf8]);
        assert!(!collector.records[0].sysex);
        assert_eq!(
            &collector.records[1].bytes[..5],
            &[0xf0, 0x7e, 0x7f, 0x01, 0xf7]
        );
        assert!(collector.records[1].sysex);
    }

    #[test]
    fn usb_midi_waits_for_detached_sysex_terminator() {
        let mut decoder = Decoder::<_, 16>::new(Collector::new());
        decoder.handle(MidiEventPacket::new([0x07, 0xf0, 0x01, 0x02]));
        assert_eq!(decoder.handler().records_len, 0);
        decoder.handle(MidiEventPacket::new([0x0f, 0xf7, 0, 0]));

        let collector = decoder.handler();
        assert_eq!(collector.records_len, 1);
        assert_eq!(&collector.records[0].bytes[..4], &[0xf0, 0x01, 0x02, 0xf7]);
        assert!(collector.records[0].sysex);
    }

    #[test]
    fn usb_midi_reports_sysex_capacity_and_reset_errors() {
        let mut decoder = Decoder::<_, 4>::new(Collector::new());
        decoder.handle(MidiEventPacket::new([0x04, 0xf0, 0x01, 0x02]));
        decoder.handle(MidiEventPacket::new([0x06, 0x03, 0xf7, 0]));
        assert_eq!(decoder.handler().error, Some(DecodeError::SysExTooLong));

        decoder.reset();
        decoder.handle(MidiEventPacket::new([0x06, 0x03, 0xf7, 0]));
        assert_eq!(
            decoder.handler().error,
            Some(DecodeError::UnexpectedSysExContinuation)
        );
    }
}
