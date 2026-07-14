//! USB device resources reserved by the Daisy Seed board support package.

use embassy_stm32::bind_interrupts;
use embassy_stm32::peripherals;
use embassy_stm32::usb::{self, Driver};

bind_interrupts!(struct Irqs {
    OTG_FS => usb::InterruptHandler<peripherals::USB_OTG_FS>;
});

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
