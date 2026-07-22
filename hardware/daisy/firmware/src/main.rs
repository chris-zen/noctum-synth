#![no_std]
#![no_main]

use embassy_daisy::{Board, PwmChannels, PwmFrequency};
use embassy_stm32::interrupt::{self, InterruptExt, Priority};
use embassy_sync::channel::Channel;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use synth_core::{FilterOversampling, FilterType, MidiClockMode};

use analog_synth_daisy_firmware::audio::{
    ControlQueue, HardwareSynth, PatchQueue, PerformanceQueue,
};
use analog_synth_daisy_firmware::pending_releases::PendingReleases;
use analog_synth_daisy_firmware::{audio, diagnostics, indicator, midi, usb_audio};

const SAMPLE_RATE_HZ: f32 = usb_audio::SAMPLE_RATE_HZ as f32;
// One second of float delay history per stereo channel. The buffer remains a
// shared pool because only one global effect is active at a time.
const EFFECTS_SAMPLES: usize = usb_audio::SAMPLE_RATE_HZ * 2;
const FIRMWARE_FILTER_TYPE: FilterType = FilterType::GainLimitedTpt;
const FIRMWARE_FILTER_OVERSAMPLING: FilterOversampling = FilterOversampling::Off;
const FIRMWARE_MIDI_CLOCK_MODE: MidiClockMode = MidiClockMode::Off;

static ENGINE: StaticCell<HardwareSynth> = StaticCell::new();
static CONTROLS: ControlQueue = ControlQueue::new();
static PERFORMANCE: PerformanceQueue = PerformanceQueue::new();
static PENDING_RELEASES: PendingReleases = PendingReleases::new();
static PATCHES: PatchQueue = Channel::new();
static INDICATOR: indicator::Indicator = indicator::Indicator::new();
static USB_AUDIO: usb_audio::UsbAudioBuffer = usb_audio::UsbAudioBuffer::new();

#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    defmt::info!("initializing Daisy Seed 1.1");

    diagnostics::init();

    let Some(mut core) = cortex_m::Peripherals::take() else {
        fatal("Cortex-M peripherals already initialized");
    };
    core.DCB.enable_trace();
    core.DWT.enable_cycle_counter();
    // Do not inherit cache state from whichever bootloader version launched
    // us. SDRAM initialization tests physical memory before the BSP installs
    // its own MPU regions and re-enables D-cache.
    core.SCB.disable_dcache(&mut core.CPUID);
    core.SCB.enable_icache();

    let parts = match Board::take() {
        Ok(parts) => parts,
        Err(_) => fatal("Daisy board already initialized"),
    };
    let mut sdram = match parts
        .sdram
        .init(&mut core.MPU, &mut core.SCB, &mut core.CPUID)
    {
        Ok(sdram) => sdram,
        Err(_) => fatal("SDRAM data/address-line test failed"),
    };
    let effects_memory = match sdram.allocate_f32(EFFECTS_SAMPLES) {
        Ok(memory) => memory,
        Err(_) => fatal("SDRAM effects allocation failed"),
    };
    defmt::info!(
        "initialized SDRAM; reserved {} effect samples",
        EFFECTS_SAMPLES
    );

    let Some(engine) = ENGINE.try_init_with(|| {
        let mut engine = HardwareSynth::new_with_effects_memory(SAMPLE_RATE_HZ, effects_memory);
        engine.set_filter_type(FIRMWARE_FILTER_TYPE);
        engine.set_filter_oversampling(FIRMWARE_FILTER_OVERSAMPLING);
        engine.set_midi_clock_mode(FIRMWARE_MIDI_CLOCK_MODE);
        engine
    }) else {
        fatal("synth engine already initialized");
    };

    let pwm_channels = PwmChannels::new(parts.tim3, PwmFrequency::khz(1));
    let status_led = parts.user_led_pin.into_pwm_led(pwm_channels.ch2);
    let (indicator_tx, indicator_rx) = INDICATOR.split();
    match indicator::run_task(status_led, indicator_rx) {
        Ok(task) => spawner.spawn(task),
        Err(_) => defmt::error!("status LED task unavailable"),
    }
    #[cfg(feature = "diagnostics")]
    match diagnostics::run_task() {
        Ok(task) => spawner.spawn(task),
        Err(_) => defmt::error!("diagnostics task unavailable"),
    }

    // Hardware DMA/USB handlers stay at P0. Audio rendering runs at P1, and
    // USB class/packet work runs at P2 so both preempt thread-mode diagnostics.
    interrupt::I2C4_EV.set_priority(Priority::P1);
    interrupt::I2C4_ER.set_priority(Priority::P2);
    if audio::spawn(
        parts.audio,
        engine,
        &CONTROLS,
        &PERFORMANCE,
        &PENDING_RELEASES,
        &PATCHES,
        indicator_tx,
        &USB_AUDIO,
    )
    .is_err()
    {
        fatal("failed to spawn audio task");
    }

    if midi::spawn(
        parts.usb,
        &CONTROLS,
        &PERFORMANCE,
        &PENDING_RELEASES,
        &PATCHES,
        indicator_tx,
        &USB_AUDIO,
    )
    .is_err()
    {
        defmt::error!("USB MIDI/audio task unavailable; DAC remains active");
    }

    core::future::pending().await
}

fn fatal(reason: &'static str) -> ! {
    defmt::error!("fatal firmware initialization failure: {=str}", reason);
    loop {
        cortex_m::asm::wfi();
    }
}
