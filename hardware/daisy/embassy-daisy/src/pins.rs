//! Daisy connector pin names and reserved onboard resource groups.

use embassy_stm32::{Peri, peripherals};

/// GPIO pins named according to the Daisy Seed connector labels.
///
/// Codec, SAI, QSPI, and SDRAM pins are deliberately absent. They remain owned
/// by their corresponding BSP capability so applications cannot accidentally
/// reconfigure an onboard connection.
#[non_exhaustive]
pub struct Pins {
    pub pin_0: Peri<'static, peripherals::PB12>,
    pub pin_1: Peri<'static, peripherals::PC11>,
    pub pin_2: Peri<'static, peripherals::PC10>,
    pub pin_3: Peri<'static, peripherals::PC9>,
    pub pin_4: Peri<'static, peripherals::PC8>,
    pub pin_5: Peri<'static, peripherals::PD2>,
    pub pin_6: Peri<'static, peripherals::PC12>,
    pub pin_7: Peri<'static, peripherals::PG10>,
    pub pin_8: Peri<'static, peripherals::PG11>,
    pub pin_9: Peri<'static, peripherals::PB4>,
    pub pin_10: Peri<'static, peripherals::PB5>,
    pub pin_11: Peri<'static, peripherals::PB8>,
    pub pin_12: Peri<'static, peripherals::PB9>,
    pub pin_13: Peri<'static, peripherals::PB6>,
    pub pin_14: Peri<'static, peripherals::PB7>,
    pub pin_15: Peri<'static, peripherals::PC0>,
    pub pin_16: Peri<'static, peripherals::PA3>,
    pub pin_17: Peri<'static, peripherals::PB1>,
    pub pin_18: Peri<'static, peripherals::PA7>,
    pub pin_19: Peri<'static, peripherals::PA6>,
    pub pin_20: Peri<'static, peripherals::PC1>,
    pub pin_21: Peri<'static, peripherals::PC4>,
    pub pin_22: Peri<'static, peripherals::PA5>,
    pub pin_23: Peri<'static, peripherals::PA4>,
    pub pin_24: Peri<'static, peripherals::PA1>,
    pub pin_25: Peri<'static, peripherals::PA0>,
    pub pin_26: Peri<'static, peripherals::PD11>,
    pub pin_27: Peri<'static, peripherals::PG9>,
    pub pin_28: Peri<'static, peripherals::PA2>,
    pub pin_29: Peri<'static, peripherals::PB14>,
    pub pin_30: Peri<'static, peripherals::PB15>,
}
