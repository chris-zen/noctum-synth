//! Top-level Daisy board ownership and initialization.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_stm32::{Peri, peripherals};

use crate::{
    audio::AudioResources, led::UserLedPin, memory::zero_sram1_bss, pins::Pins,
    qspi::QspiFlashResources, sdram::SdramResources, usb::UsbResources,
};

static TAKEN: AtomicBool = AtomicBool::new(false);

// Make linking two versions of this BSP fail instead of allowing each version
// to believe it owns a separate board singleton.
#[unsafe(no_mangle)]
#[used]
static EMBASSY_DAISY_BOARD_SINGLETON: () = ();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TakeError {
    AlreadyTaken,
}

pub struct Board;

#[non_exhaustive]
pub struct BoardParts {
    pub pins: Pins,
    pub user_led_pin: UserLedPin,
    pub tim3: Peri<'static, peripherals::TIM3>,
    pub audio: AudioResources,
    pub usb: UsbResources,
    pub sdram: SdramResources,
    pub qspi: QspiFlashResources,
}

impl Board {
    /// Initialize and take exclusive ownership of the Daisy board.
    pub fn take() -> Result<BoardParts, TakeError> {
        if TAKEN
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TakeError::AlreadyTaken);
        }

        let p = embassy_stm32::init(crate::clocks::config());

        zero_sram1_bss();

        Ok(BoardParts {
            pins: Pins {
                pin_0: p.PB12,
                pin_1: p.PC11,
                pin_2: p.PC10,
                pin_3: p.PC9,
                pin_4: p.PC8,
                pin_5: p.PD2,
                pin_6: p.PC12,
                pin_7: p.PG10,
                pin_8: p.PG11,
                pin_9: p.PB4,
                pin_10: p.PB5,
                pin_11: p.PB8,
                pin_12: p.PB9,
                pin_13: p.PB6,
                pin_14: p.PB7,
                pin_15: p.PC0,
                pin_16: p.PA3,
                pin_17: p.PB1,
                pin_18: p.PA7,
                pin_19: p.PA6,
                pin_20: p.PC1,
                pin_21: p.PC4,
                pin_22: p.PA5,
                pin_23: p.PA4,
                pin_24: p.PA1,
                pin_25: p.PA0,
                pin_26: p.PD11,
                pin_27: p.PG9,
                pin_28: p.PA2,
                pin_29: p.PB14,
                pin_30: p.PB15,
            },
            user_led_pin: UserLedPin::new(p.PC7),
            tim3: p.TIM3,
            audio: AudioResources {
                sai: p.SAI1,
                mclk: p.PE2,
                sd_b: p.PE3,
                fs: p.PE4,
                sck: p.PE5,
                sd_a: p.PE6,
                codec_i2c: p.I2C2,
                codec_scl: p.PH4,
                codec_sda: p.PB11,
                dma_a: p.DMA1_CH0,
                dma_b: p.DMA1_CH1,
            },
            usb: UsbResources {
                peripheral: p.USB_OTG_FS,
                dm: p.PA11,
                dp: p.PA12,
            },
            sdram: SdramResources {
                fmc: p.FMC,
                a0: p.PF0,
                a1: p.PF1,
                a2: p.PF2,
                a3: p.PF3,
                a4: p.PF4,
                a5: p.PF5,
                a6: p.PF12,
                a7: p.PF13,
                a8: p.PF14,
                a9: p.PF15,
                a10: p.PG0,
                a11: p.PG1,
                a12: p.PG2,
                ba0: p.PG4,
                ba1: p.PG5,
                d0: p.PD14,
                d1: p.PD15,
                d2: p.PD0,
                d3: p.PD1,
                d4: p.PE7,
                d5: p.PE8,
                d6: p.PE9,
                d7: p.PE10,
                d8: p.PE11,
                d9: p.PE12,
                d10: p.PE13,
                d11: p.PE14,
                d12: p.PE15,
                d13: p.PD8,
                d14: p.PD9,
                d15: p.PD10,
                d16: p.PH8,
                d17: p.PH9,
                d18: p.PH10,
                d19: p.PH11,
                d20: p.PH12,
                d21: p.PH13,
                d22: p.PH14,
                d23: p.PH15,
                d24: p.PI0,
                d25: p.PI1,
                d26: p.PI2,
                d27: p.PI3,
                d28: p.PI6,
                d29: p.PI7,
                d30: p.PI9,
                d31: p.PI10,
                nbl0: p.PE0,
                nbl1: p.PE1,
                nbl2: p.PI4,
                nbl3: p.PI5,
                sdcke: p.PH2,
                sdclk: p.PG8,
                sdncas: p.PG15,
                sdne: p.PH3,
                sdnras: p.PF11,
                sdnwe: p.PH5,
            },
            qspi: QspiFlashResources {
                qspi: p.QUADSPI,
                io0: p.PF8,
                io1: p.PF9,
                io2: p.PF7,
                io3: p.PF6,
                sck: p.PF10,
                cs: p.PG6,
            },
        })
    }
}
