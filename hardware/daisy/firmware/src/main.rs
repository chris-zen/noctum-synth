#![no_std]
#![no_main]

use analog_synth_daisy_firmware::midi;
#[cfg(feature = "audio-profiling")]
use analog_synth_daisy_firmware::profiling;
use analog_synth_daisy_firmware::synth::SynthMidiHandler;
use embassy_daisy::Board;
use embassy_daisy::audio::{Audio, AudioResources, BLOCK_LENGTH, Block};
use embassy_futures::{join::join, yield_now};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;
use static_cell::StaticCell;
use synth_core::{ControlMessage, SynthEngineWithMemory};
use {defmt_rtt as _, panic_probe as _};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const CONTROL_QUEUE_CAPACITY: usize = 32;
const EFFECTS_SAMPLES: usize = 48_000;
#[cfg(feature = "audio-profiling")]
const AUDIO_BLOCK_CYCLE_BUDGET: u32 =
    embassy_daisy::clocks::SYSCLK_HZ / embassy_daisy::audio::SAMPLE_RATE_HZ * BLOCK_LENGTH as u32;
type HardwareSynth = SynthEngineWithMemory<1, &'static mut [f32]>;
type ControlQueue = Channel<ThreadModeRawMutex, ControlMessage, CONTROL_QUEUE_CAPACITY>;

static ENGINE: StaticCell<HardwareSynth> = StaticCell::new();
static CONTROLS: ControlQueue = Channel::new();

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    defmt::info!("initializing Daisy Seed 1.1");

    let mut core = cortex_m::Peripherals::take().expect("Cortex-M peripherals already initialized");
    #[cfg(feature = "audio-profiling")]
    {
        core.DCB.enable_trace();
        core.DWT.enable_cycle_counter();
    }
    // Do not inherit cache state from whichever bootloader version launched
    // us. SDRAM initialization tests physical memory before the BSP installs
    // its own MPU regions and re-enables D-cache.
    core.SCB.disable_dcache(&mut core.CPUID);
    core.SCB.enable_icache();

    let parts = Board::take().expect("Daisy board already initialized");
    let mut sdram = parts
        .sdram
        .init(&mut core.MPU, &mut core.SCB, &mut core.CPUID)
        .expect("SDRAM data/address-line test failed");
    let effects_memory = sdram
        .allocate_f32(EFFECTS_SAMPLES)
        .expect("SDRAM effects allocation failed");
    defmt::info!(
        "initialized SDRAM; reserved {} effect samples",
        EFFECTS_SAMPLES
    );
    let engine =
        ENGINE.init_with(|| HardwareSynth::new_with_effects_memory(SAMPLE_RATE_HZ, effects_memory));
    let midi_handler = SynthMidiHandler::new(CONTROLS.sender());
    join(
        run_audio(parts.audio, engine, &CONTROLS),
        midi::run(parts.usb, midi_handler),
    )
    .await;
}

async fn run_audio(
    resources: AudioResources,
    engine: &'static mut HardwareSynth,
    controls: &'static ControlQueue,
) -> ! {
    let mut audio = Audio::new(resources).expect("WM8731/SAI initialization failed");
    yield_now().await;

    let mut output: Block = [(0.0, 0.0); BLOCK_LENGTH];
    let mut input: Block = [(0.0, 0.0); BLOCK_LENGTH];
    let mut interleaved = [0.0f32; BLOCK_LENGTH * 2];
    #[cfg(feature = "audio-profiling")]
    let mut profiler = profiling::AudioProfiler::new(AUDIO_BLOCK_CYCLE_BUDGET);

    // Render before starting the receive clock. The SAI input ring cannot
    // overrun while the first, comparatively expensive DSP block is prepared.
    engine.process_interleaved(&mut interleaved, 2);
    copy_output(&interleaved, &mut output);
    yield_now().await;
    defmt::info!("running four-voice synth engine at 48 kHz");
    audio
        .start(&output)
        .await
        .expect("SAI stream failed to start");

    loop {
        while let Ok(command) = controls.try_receive() {
            engine.handle_control(command);
        }

        if let Err(error) = audio.transfer(&output, &mut input).await {
            defmt::error!("audio transfer failed: {}", error.category());
            #[cfg(feature = "audio-profiling")]
            report_profile(profiler.take_snapshot());
            panic!("audio transfer failed");
        }

        #[cfg(feature = "audio-profiling")]
        if profiler.report_due() {
            report_profile(profiler.take_snapshot());
        }

        while let Ok(command) = controls.try_receive() {
            engine.handle_control(command);
        }

        #[cfg(feature = "audio-profiling")]
        {
            profiler.begin_block();
            engine.process_interleaved_profiled(&mut interleaved, 2, &mut profiler);
            profiler.end_block();
        }
        #[cfg(not(feature = "audio-profiling"))]
        engine.process_interleaved(&mut interleaved, 2);
        copy_output(&interleaved, &mut output);
        yield_now().await;
    }
}

#[cfg(feature = "audio-profiling")]
fn report_profile(snapshot: profiling::Snapshot) {
    let average = snapshot.stage_average;
    let maximum = snapshot.stage_max;
    defmt::info!(
        "audio profile blocks={} overruns={} block_avg={} block_max={}",
        snapshot.blocks,
        snapshot.overruns,
        snapshot.block_average,
        snapshot.block_max
    );
    defmt::info!(
        "stage avg env_mod={} osc={} filter={} amp_pan={} effects={} output={}",
        average[0],
        average[1],
        average[2],
        average[3],
        average[4],
        average[5]
    );
    defmt::info!(
        "stage max env_mod={} osc={} filter={} amp_pan={} effects={} output={}",
        maximum[0],
        maximum[1],
        maximum[2],
        maximum[3],
        maximum[4],
        maximum[5]
    );
}

fn copy_output(interleaved: &[f32; BLOCK_LENGTH * 2], output: &mut Block) {
    for (frame, samples) in output.iter_mut().zip(interleaved.chunks_exact(2)) {
        *frame = (samples[0], samples[1]);
    }
}
