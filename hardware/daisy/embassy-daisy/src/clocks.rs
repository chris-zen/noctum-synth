//! Daisy-specific STM32 clock configuration.

use embassy_stm32::Config;
use embassy_stm32::rcc::mux::{Saisel, Usbsel};
use embassy_stm32::rcc::{
    AHBPrescaler, APBPrescaler, Hse, HseMode, Hsi48Config, Pll, PllDiv, PllMul, PllPreDiv,
    PllSource, Sysclk,
};
use embassy_stm32::time::Hertz;

/// External high-speed crystal fitted to Daisy Seed boards.
pub const HSE_HZ: u32 = 16_000_000;

/// Core clock used for audio DSP workloads.
pub const SYSCLK_HZ: u32 = 480_000_000;

/// Master audio clock for 48 kHz operation (256 × sample rate).
pub const AUDIO_MCLK_HZ: u32 = 12_288_000;

/// Build the clock configuration used by Daisy Seed 1.1.
///
/// PLL1 runs the Cortex-M7 at 480 MHz. PLL3 produces the 12.288 MHz codec clock
/// exactly using a 3.2 MHz input, a ×192 VCO multiplier, and a ÷50 P output.
pub fn config() -> Config {
    let mut config = Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz::mhz(16),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll1 = Some(Pll {
        source: PllSource::HSE,
        prediv: PllPreDiv::DIV4,
        mul: PllMul::MUL240,
        divp: Some(PllDiv::DIV2),
        divq: Some(PllDiv::DIV20),
        divr: None,
    });
    config.rcc.pll3 = Some(Pll {
        source: PllSource::HSE,
        prediv: PllPreDiv::DIV5,
        mul: PllMul::MUL192,
        divp: Some(PllDiv::DIV50),
        divq: None,
        divr: None,
    });
    config.rcc.sys = Sysclk::PLL1_P;
    config.rcc.d1c_pre = AHBPrescaler::DIV1;
    config.rcc.ahb_pre = AHBPrescaler::DIV2;
    config.rcc.apb1_pre = APBPrescaler::DIV2;
    config.rcc.apb2_pre = APBPrescaler::DIV2;
    config.rcc.apb3_pre = APBPrescaler::DIV2;
    config.rcc.apb4_pre = APBPrescaler::DIV2;
    config.rcc.mux.sai1sel = Saisel::PLL3_P;
    config.rcc.hsi48 = Some(Hsi48Config {
        sync_from_usb: true,
    });
    config.rcc.mux.usbsel = Usbsel::HSI48;
    config
}
