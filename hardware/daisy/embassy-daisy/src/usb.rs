//! USB device resources reserved by the Daisy Seed board support package.

use embassy_stm32::bind_interrupts;
use embassy_stm32::peripherals;
use embassy_stm32::usb::{self, Driver};

use core::sync::atomic::{AtomicU32, Ordering};

bind_interrupts!(struct Irqs {
    OTG_FS => IsochronousInterruptObserver, usb::InterruptHandler<peripherals::USB_OTG_FS>, IsochronousInterruptAcknowledger;
});

struct IsochronousInterruptObserver;
struct IsochronousInterruptAcknowledger;
static ACTIVE_ISOCHRONOUS_IN_ENDPOINTS: AtomicU32 = AtomicU32::new(0);
const ENDPOINT_COUNT: usize = 9;

impl embassy_stm32::interrupt::typelevel::Handler<embassy_stm32::interrupt::typelevel::OTG_FS>
    for IsochronousInterruptObserver
{
    unsafe fn on_interrupt() {
        let registers = stm32_metapac::USB_OTG_FS;
        let status = registers.gintsts().read();

        if status.eopf() {
            resynchronize_missed_isochronous_in(registers);
        }
    }
}

/// Drop an isochronous packet whose scheduled frame has passed and make its
/// endpoint writable again.
///
/// The pinned Synopsys driver disables and re-enables the endpoint while
/// retaining the packet in its FIFO. On this OTG revision, endpoint disable
/// clears DIEPTSIZ, so that packet can no longer be transmitted and prevents
/// the next writer from obtaining FIFO space. Run before Embassy's interrupt
/// handler: the EPDISD interrupt remains pending for Embassy to acknowledge and
/// use to wake the endpoint writer.
fn resynchronize_missed_isochronous_in(registers: stm32_metapac::otg::Otg) {
    let frame_is_odd = registers.dsts().read().fnsof() & 1 != 0;
    let mut endpoints = ACTIVE_ISOCHRONOUS_IN_ENDPOINTS.load(Ordering::Relaxed);
    let mut endpoint = 0usize;

    while endpoints != 0 {
        if endpoints & 1 != 0 {
            let control = registers.diepctl(endpoint).read();
            if control.usbaep() && control.epena() && control.eonum_dpid() == frame_is_odd {
                let interrupt = registers.diepint(endpoint);
                registers
                    .diepctl(endpoint)
                    .modify(|register| register.set_snak(true));
                while !interrupt.read().inepne() {}

                registers.diepctl(endpoint).modify(|register| {
                    register.set_snak(true);
                    register.set_epdis(true);
                });
                while !interrupt.read().epdisd() {}

                registers.grstctl().modify(|register| {
                    register.set_txfnum(endpoint as u8);
                    register.set_txfflsh(true);
                });
                while registers.grstctl().read().txfflsh() {}
            }
        }
        endpoints >>= 1;
        endpoint += 1;
    }
}

impl embassy_stm32::interrupt::typelevel::Handler<embassy_stm32::interrupt::typelevel::OTG_FS>
    for IsochronousInterruptAcknowledger
{
    unsafe fn on_interrupt() {
        let status = stm32_metapac::USB_OTG_FS.gintsts().read();
        if status.eopf() {
            // GINTSTS is write-one-to-clear. A read-modify-write could also
            // acknowledge unrelated USB events that arrived concurrently.
            stm32_metapac::USB_OTG_FS
                .gintsts()
                .write(|register| register.set_eopf(true));
        }
    }
}

/// Flush data stranded in a disabled isochronous IN endpoint's dedicated TX
/// FIFO. Returns `false` without changing hardware if the endpoint is active.
///
/// Some Synopsys OTG revisions clear `EPENA` after an incomplete periodic
/// transfer without releasing the packet already written to the dedicated
/// FIFO. In that state the normal endpoint writer cannot submit another
/// packet because it waits for FIFO space that can no longer become available.
pub fn recover_disabled_isochronous_in(endpoint_index: usize) -> bool {
    cortex_m::interrupt::free(|_| {
        let registers = stm32_metapac::USB_OTG_FS;
        let control = registers.diepctl(endpoint_index).read();
        if control.epena() {
            return false;
        }

        registers.grstctl().modify(|register| {
            register.set_txfnum(endpoint_index as u8);
            register.set_txfflsh(true);
        });
        while registers.grstctl().read().txfflsh() {}

        let pending = registers.diepint(endpoint_index).read();
        registers.diepint(endpoint_index).write_value(pending);
        true
    })
}

/// Seed resources required by the internal full-speed USB peripheral.
pub struct UsbResources {
    pub(crate) peripheral: embassy_stm32::Peri<'static, peripherals::USB_OTG_FS>,
    pub(crate) dm: embassy_stm32::Peri<'static, peripherals::PA11>,
    pub(crate) dp: embassy_stm32::Peri<'static, peripherals::PA12>,
}

impl UsbResources {
    /// Construct the Embassy full-speed USB driver for the Seed's onboard USB
    /// connector.
    ///
    /// VBUS detection is disabled because the current development target is a
    /// bus-powered device. A self-powered product must revisit this setting for
    /// USB compliance.
    pub fn driver<'d>(
        self,
        endpoint_out_buffer: &'d mut [u8],
    ) -> Driver<'d, peripherals::USB_OTG_FS> {
        let mut config = usb::Config::default();
        config.vbus_detection = false;
        Driver::new_fs(
            self.peripheral,
            Irqs,
            self.dp,
            self.dm,
            endpoint_out_buffer,
            config,
        )
    }
}

/// Enable or disable recovery of an isochronous IN transfer that was not
/// consumed in its scheduled USB frame.
///
/// `embassy-usb-synopsys-otg` contains the recovery handler, but version 0.3.3
/// neither unmasks nor acknowledges its end-of-periodic-frame interrupt. Keep
/// the interrupt enabled only while an isochronous source is active: it fires
/// every USB frame and is unnecessary for MIDI and control transfers.
pub fn set_isochronous_in_recovery(endpoint_index: usize, enabled: bool) {
    assert!(endpoint_index < ENDPOINT_COUNT);
    cortex_m::interrupt::free(|_| {
        let endpoint = 1u32 << endpoint_index;
        let active = if enabled {
            ACTIVE_ISOCHRONOUS_IN_ENDPOINTS.fetch_or(endpoint, Ordering::AcqRel) | endpoint
        } else {
            ACTIVE_ISOCHRONOUS_IN_ENDPOINTS.fetch_and(!endpoint, Ordering::AcqRel) & !endpoint
        };
        stm32_metapac::USB_OTG_FS
            .gintmsk()
            .modify(|register| register.set_eopfm(active != 0));
    });
}
