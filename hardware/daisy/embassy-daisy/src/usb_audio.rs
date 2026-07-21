//! Generic USB Audio Class transports for Embassy USB devices.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_usb::control::{InResponse, OutResponse, Recipient, Request, RequestType};
use embassy_usb::descriptor::{SynchronizationType, UsageType};
use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointType};
use embassy_usb::{Builder, Handler};

const USB_AUDIO_CLASS: u8 = 0x01;
const VENDOR_SPECIFIC_CLASS: u8 = 0xff;
const AUDIOCONTROL_SUBCLASS: u8 = 0x01;
const AUDIOSTREAMING_SUBCLASS: u8 = 0x02;
const PROTOCOL_NONE: u8 = 0x00;
const CS_INTERFACE: u8 = 0x24;
const CS_ENDPOINT: u8 = 0x25;
const HEADER: u8 = 0x01;
const INPUT_TERMINAL: u8 = 0x02;
const OUTPUT_TERMINAL: u8 = 0x03;
const AS_GENERAL: u8 = 0x01;
const FORMAT_TYPE: u8 = 0x02;
const FORMAT_TYPE_I: u8 = 0x01;
const PCM: u16 = 0x0001;
const SET_CUR: u8 = 0x01;
const GET_CUR: u8 = 0x81;
const SAMPLING_FREQ_CONTROL: u8 = 0x01;
const LOCK_DELAY_UNIT_PCM_SAMPLES: u8 = 0x02;
const INPUT_TERMINAL_ID: u8 = 0x01;
const OUTPUT_TERMINAL_ID: u8 = 0x02;
const USB_STREAMING_TERMINAL: u16 = 0x0101;
const MAX_SAMPLE_RATES: usize = 10;

/// Audio channel positions supported by the UAC1 channel bitmap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    LeftFront,
    RightFront,
    CenterFront,
    Lfe,
    LeftSurround,
    RightSurround,
}

impl Channel {
    const fn bit(self) -> u16 {
        match self {
            Self::LeftFront => 0x0001,
            Self::RightFront => 0x0002,
            Self::CenterFront => 0x0004,
            Self::Lfe => 0x0008,
            Self::LeftSurround => 0x0010,
            Self::RightSurround => 0x0020,
        }
    }
}

/// PCM sample width advertised by the streaming interface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SampleWidth {
    Bits16 = 2,
    Bits24 = 3,
    Bits32 = 4,
}

impl SampleWidth {
    const fn bytes(self) -> u8 {
        self as u8
    }

    const fn bits(self) -> u8 {
        self.bytes() * 8
    }
}

/// Physical source represented by the UAC1 input terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum InputTerminalType {
    Microphone = 0x0201,
    LineConnector = 0x0603,
}

/// Host-driver binding used by a USB audio function.
///
/// `VendorSpecific` keeps the audio class driver from claiming the stream so
/// a raw USB diagnostic can inspect the isochronous packets. The descriptors
/// and stream transport remain otherwise identical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostBinding {
    AudioClass,
    VendorSpecific,
}

struct Shared {
    sample_rate_hz: AtomicU32,
    suspended: AtomicBool,
    suspension_epoch: AtomicU32,
    state_changed: Signal<CriticalSectionRawMutex, ()>,
}

impl Shared {
    const fn new() -> Self {
        Self {
            sample_rate_hz: AtomicU32::new(0),
            suspended: AtomicBool::new(false),
            suspension_epoch: AtomicU32::new(0),
            state_changed: Signal::new(),
        }
    }

    fn set_suspended(&self, suspended: bool) {
        if suspended {
            self.suspension_epoch.fetch_add(1, Ordering::AcqRel);
        }
        self.suspended.store(suspended, Ordering::Release);
        self.state_changed.signal(());
    }
}

/// Storage for the USB control handler. It must live as long as the USB device.
pub struct State<'d> {
    control: Option<Control<'d>>,
    shared: Shared,
}

impl Default for State<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl State<'_> {
    pub const fn new() -> Self {
        Self {
            control: None,
            shared: Shared::new(),
        }
    }
}

/// UAC1 device-to-host audio source.
pub struct Microphone;

impl Microphone {
    /// Add a single asynchronous full-speed PCM capture stream to an Embassy
    /// USB builder.
    #[allow(clippy::too_many_arguments)]
    pub fn new<'d, D: Driver<'d>>(
        builder: &mut Builder<'d, D>,
        state: &'d mut State<'d>,
        max_packet_size: u16,
        sample_width: SampleWidth,
        sample_rates_hz: &'d [u32],
        channels: &'d [Channel],
        terminal_type: InputTerminalType,
        host_binding: HostBinding,
    ) -> Stream<'d, D> {
        assert!(max_packet_size != 0 && max_packet_size <= 1_023);
        assert!(!sample_rates_hz.is_empty());
        assert!(sample_rates_hz.len() <= MAX_SAMPLE_RATES);
        assert!(!channels.is_empty() && channels.len() <= u8::MAX as usize);

        let mut channel_bitmap = 0u16;
        let mut previous_channel_bit = 0u16;
        for channel in channels {
            let bit = channel.bit();
            assert_eq!(channel_bitmap & bit, 0, "duplicate USB audio channel");
            assert!(
                bit > previous_channel_bit,
                "USB audio channels must be in canonical bitmap order"
            );
            channel_bitmap |= bit;
            previous_channel_bit = bit;
        }

        let mut highest_sample_rate_hz = 0u32;
        for (index, rate) in sample_rates_hz.iter().copied().enumerate() {
            assert!(rate != 0 && rate <= 0x00ff_ffff);
            assert!(
                !sample_rates_hz[..index].contains(&rate),
                "duplicate USB audio sample rate"
            );
            highest_sample_rate_hz = highest_sample_rate_hz.max(rate);
        }
        let bytes_per_frame = channels.len() as u32 * u32::from(sample_width.bytes());
        // An asynchronous source must be able to send one frame above the
        // nominal full-speed interval to correct positive source-clock drift.
        let required_packet_bytes = (highest_sample_rate_hz.div_ceil(1_000) + 1) * bytes_per_frame;
        assert!(
            u32::from(max_packet_size) >= required_packet_bytes,
            "USB audio packet lacks format or asynchronous drift capacity"
        );
        assert_eq!(
            u32::from(max_packet_size) % bytes_per_frame,
            0,
            "USB audio packet must contain a whole number of frames"
        );

        let (function_class, control_subclass, streaming_subclass) = match host_binding {
            HostBinding::AudioClass => (
                USB_AUDIO_CLASS,
                AUDIOCONTROL_SUBCLASS,
                AUDIOSTREAMING_SUBCLASS,
            ),
            HostBinding::VendorSpecific => (VENDOR_SPECIFIC_CLASS, 0x00, 0x00),
        };
        let mut function = builder.function(function_class, control_subclass, PROTOCOL_NONE);

        let mut control_interface = function.interface();
        let control_interface_number = control_interface.interface_number();
        let streaming_interface_number = u8::from(control_interface_number) + 1;
        let mut control_alt =
            control_interface.alt_setting(function_class, control_subclass, PROTOCOL_NONE, None);
        control_alt.descriptor(
            CS_INTERFACE,
            &[
                HEADER,
                0x00,
                0x01,
                30,
                0x00,
                0x01,
                streaming_interface_number,
            ],
        );

        let terminal = terminal_type as u16;
        control_alt.descriptor(
            CS_INTERFACE,
            &[
                INPUT_TERMINAL,
                INPUT_TERMINAL_ID,
                terminal as u8,
                (terminal >> 8) as u8,
                0x00,
                channels.len() as u8,
                channel_bitmap as u8,
                (channel_bitmap >> 8) as u8,
                0x00,
                0x00,
            ],
        );
        control_alt.descriptor(
            CS_INTERFACE,
            &[
                OUTPUT_TERMINAL,
                OUTPUT_TERMINAL_ID,
                USB_STREAMING_TERMINAL as u8,
                (USB_STREAMING_TERMINAL >> 8) as u8,
                0x00,
                INPUT_TERMINAL_ID,
                0x00,
            ],
        );

        let mut streaming_interface = function.interface();
        let idle_alt = streaming_interface.alt_setting(
            function_class,
            streaming_subclass,
            PROTOCOL_NONE,
            None,
        );
        drop(idle_alt);

        let mut active_alt = streaming_interface.alt_setting(
            function_class,
            streaming_subclass,
            PROTOCOL_NONE,
            None,
        );
        active_alt.descriptor(
            CS_INTERFACE,
            &[
                AS_GENERAL,
                OUTPUT_TERMINAL_ID,
                0x00,
                PCM as u8,
                (PCM >> 8) as u8,
            ],
        );

        let mut format = [0u8; 6 + MAX_SAMPLE_RATES * 3];
        format[0] = FORMAT_TYPE;
        format[1] = FORMAT_TYPE_I;
        format[2] = channels.len() as u8;
        format[3] = sample_width as u8;
        format[4] = sample_width.bits();
        format[5] = sample_rates_hz.len() as u8;
        for (index, rate) in sample_rates_hz.iter().copied().enumerate() {
            let offset = 6 + index * 3;
            format[offset] = rate as u8;
            format[offset + 1] = (rate >> 8) as u8;
            format[offset + 2] = (rate >> 16) as u8;
        }
        active_alt.descriptor(CS_INTERFACE, &format[..6 + sample_rates_hz.len() * 3]);

        let endpoint =
            active_alt.alloc_endpoint_in(EndpointType::Isochronous, None, max_packet_size, 1);
        active_alt.endpoint_descriptor(
            endpoint.info(),
            SynchronizationType::Asynchronous,
            UsageType::DataEndpoint,
            &[0x00, 0x00],
        );
        active_alt.descriptor(
            CS_ENDPOINT,
            &[
                AS_GENERAL,
                SAMPLING_FREQ_CONTROL,
                LOCK_DELAY_UNIT_PCM_SAMPLES,
                0x00,
                0x00,
            ],
        );
        drop(function);

        state
            .shared
            .sample_rate_hz
            .store(sample_rates_hz[0], Ordering::Relaxed);
        state.control = Some(Control {
            endpoint_address: endpoint.info().addr.into(),
            sample_rates_hz,
            shared: &state.shared,
        });
        builder.handler(state.control.as_mut().unwrap());

        Stream {
            endpoint,
            shared: &state.shared,
        }
    }
}

/// Isochronous device-to-host audio stream.
pub struct Stream<'d, D: Driver<'d>> {
    endpoint: D::EndpointIn,
    shared: &'d Shared,
}

impl<'d, D: Driver<'d>> Stream<'d, D> {
    /// Hardware endpoint index allocated for this stream.
    pub fn endpoint_index(&self) -> usize {
        self.endpoint.info().addr.index()
    }

    pub async fn wait_connection(&mut self) {
        self.endpoint.wait_enabled().await;
    }

    pub async fn write_packet(&mut self, packet: &[u8]) -> Result<(), EndpointError> {
        self.endpoint.write(packet).await
    }

    /// Return the sample rate most recently selected by the host.
    pub fn sample_rate_hz(&self) -> u32 {
        self.shared.sample_rate_hz.load(Ordering::Relaxed)
    }

    /// Return whether the USB bus is currently suspended.
    pub fn is_suspended(&self) -> bool {
        self.shared.suspended.load(Ordering::Acquire)
    }

    /// Monotonic counter incremented whenever the bus enters suspend.
    pub fn suspension_epoch(&self) -> u32 {
        self.shared.suspension_epoch.load(Ordering::Acquire)
    }

    /// Wait until the USB bus has resumed.
    pub async fn wait_resumed(&self) {
        while self.is_suspended() {
            self.shared.state_changed.wait().await;
        }
    }
}

struct Control<'d> {
    endpoint_address: u8,
    sample_rates_hz: &'d [u32],
    shared: &'d Shared,
}

impl Control<'_> {
    fn handles_endpoint(&self, request: &Request) -> bool {
        request.recipient == Recipient::Endpoint
            && request.index as u8 == self.endpoint_address
            && (request.value >> 8) as u8 == SAMPLING_FREQ_CONTROL
    }
}

impl Handler for Control<'_> {
    fn reset(&mut self) {
        self.shared
            .sample_rate_hz
            .store(self.sample_rates_hz[0], Ordering::Relaxed);
        self.shared.set_suspended(false);
    }

    fn suspended(&mut self, suspended: bool) {
        self.shared.set_suspended(suspended);
    }

    fn control_out(&mut self, request: Request, data: &[u8]) -> Option<OutResponse> {
        if request.request_type != RequestType::Class || !self.handles_endpoint(&request) {
            return None;
        }
        if request.request != SET_CUR || data.len() != 3 {
            return Some(OutResponse::Rejected);
        }
        let rate = u32::from(data[0]) | u32::from(data[1]) << 8 | u32::from(data[2]) << 16;
        if !self.sample_rates_hz.contains(&rate) {
            return Some(OutResponse::Rejected);
        }
        self.shared.sample_rate_hz.store(rate, Ordering::Relaxed);
        Some(OutResponse::Accepted)
    }

    fn control_in<'a>(
        &'a mut self,
        request: Request,
        buffer: &'a mut [u8],
    ) -> Option<InResponse<'a>> {
        if request.request_type != RequestType::Class || !self.handles_endpoint(&request) {
            return None;
        }
        if request.request != GET_CUR || buffer.len() < 3 {
            return Some(InResponse::Rejected);
        }
        let rate = self.shared.sample_rate_hz.load(Ordering::Relaxed);
        buffer[0] = rate as u8;
        buffer[1] = (rate >> 8) as u8;
        buffer[2] = (rate >> 16) as u8;
        Some(InResponse::Accepted(&buffer[..3]))
    }
}
