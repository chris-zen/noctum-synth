#![no_std]
#![no_main]

use analog_synth_daisy_firmware::profiling::{AudioProfiler, Snapshot};
use core::hint::black_box;
use cortex_m::peripheral::DWT;
use embassy_daisy::Board;
use embassy_daisy::audio::BLOCK_LENGTH;
use embassy_daisy::sdram::Sdram;
use synth_core::{
    ControlMessage, EffectType, FilterOversampling, ModDestination, ModRoute, ModSource, ParamId,
    SynthEngineWithMemory,
};
use {defmt_rtt as _, panic_probe as _};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const EFFECTS_SAMPLES: usize = 48_000;
const WARMUP_BLOCKS: usize = 128;
const MEASURED_BLOCKS: usize = 512;
const AUDIO_BLOCK_CYCLE_BUDGET: u32 =
    embassy_daisy::clocks::SYSCLK_HZ / embassy_daisy::audio::SAMPLE_RATE_HZ * BLOCK_LENGTH as u32;

type HardwareSynth = SynthEngineWithMemory<1, &'static mut [f32]>;

#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::info!("initializing Daisy DSP benchmark");

    let mut core = cortex_m::Peripherals::take().expect("Cortex-M peripherals already initialized");
    core.DCB.enable_trace();
    core.DWT.enable_cycle_counter();
    core.SCB.disable_dcache(&mut core.CPUID);
    core.SCB.enable_icache();

    let parts = Board::take().expect("Daisy board already initialized");
    let mut sdram = parts
        .sdram
        .init(&mut core.MPU, &mut core.SCB, &mut core.CPUID)
        .expect("SDRAM data/address-line test failed");

    run_scenario(&mut sdram, "idle", |_| {});
    run_scenario(&mut sdram, "one-note-default", |engine| {
        engine.note_on(60, 1.0);
    });
    run_scenario(&mut sdram, "four-note-default", configure_four_notes);
    run_scenario(&mut sdram, "four-note-active-filter", |engine| {
        configure_four_notes(engine);
        engine.set_param(ParamId::FilterCutoff, 1_200.0);
        engine.set_param(ParamId::FilterResonance, 0.65);
    });
    run_scenario(&mut sdram, "four-note-self-oscillation-off", |engine| {
        configure_self_oscillation(engine, FilterOversampling::Off)
    });
    run_scenario(&mut sdram, "four-note-self-oscillation-x2", |engine| {
        configure_self_oscillation(engine, FilterOversampling::X2)
    });
    run_scenario(&mut sdram, "four-note-modulation-heavy", |engine| {
        configure_four_notes(engine);
        configure_modulation_heavy(engine);
    });

    for effect in EffectType::ALL {
        run_scenario(&mut sdram, effect.name(), |engine| {
            configure_four_notes(engine);
            configure_effect(engine, effect);
        });
    }

    run_scenario(&mut sdram, "representative-worst-case-off", |engine| {
        configure_four_notes(engine);
        configure_worst_case(engine);
    });

    defmt::info!("Daisy DSP benchmark complete");
    loop {
        cortex_m::asm::wfi();
    }
}

fn run_scenario(sdram: &mut Sdram, name: &str, configure: impl FnOnce(&mut HardwareSynth)) {
    let effects_memory = sdram
        .allocate_f32(EFFECTS_SAMPLES)
        .expect("SDRAM effects allocation failed");
    let mut engine = HardwareSynth::new_with_effects_memory(SAMPLE_RATE_HZ, effects_memory);
    configure(&mut engine);
    benchmark_case(&mut engine, name);
}

fn benchmark_case(engine: &mut HardwareSynth, name: &str) {
    let mut output = [0.0f32; BLOCK_LENGTH * 2];
    for _ in 0..WARMUP_BLOCKS {
        engine.process_interleaved(&mut output, 2);
        black_box(output[0]);
    }

    let raw = measure_uninstrumented(engine, &mut output);

    let mut profiler = AudioProfiler::new(AUDIO_BLOCK_CYCLE_BUDGET);
    for _ in 0..MEASURED_BLOCKS {
        profiler.begin_block();
        engine.process_interleaved_profiled(&mut output, 2, &mut profiler);
        profiler.end_block();
        black_box(output[0]);
    }

    report_case(name, raw, profiler.take_snapshot());
}

fn measure_uninstrumented(
    engine: &mut HardwareSynth,
    output: &mut [f32; BLOCK_LENGTH * 2],
) -> RawTiming {
    let mut total = 0u32;
    let mut maximum = 0u32;
    let mut overruns = 0u32;
    for _ in 0..MEASURED_BLOCKS {
        let started = DWT::cycle_count();
        engine.process_interleaved(output, 2);
        let cycles = DWT::cycle_count().wrapping_sub(started);
        total = total.wrapping_add(cycles);
        maximum = maximum.max(cycles);
        overruns += u32::from(cycles > AUDIO_BLOCK_CYCLE_BUDGET);
        black_box(output[0]);
    }
    RawTiming {
        average: total / MEASURED_BLOCKS as u32,
        maximum,
        overruns,
    }
}

fn configure_four_notes(engine: &mut HardwareSynth) {
    for note in [60, 64, 67, 72] {
        engine.note_on(note, 1.0);
    }
}

fn configure_self_oscillation(engine: &mut HardwareSynth, oversampling: FilterOversampling) {
    configure_four_notes(engine);
    engine.set_param(ParamId::FilterCutoff, 440.0);
    engine.set_param(ParamId::FilterResonance, 1.0);
    engine.set_filter_oversampling(oversampling);
}

fn configure_modulation_heavy(engine: &mut HardwareSynth) {
    engine.set_param(ParamId::FilterCutoff, 1_200.0);
    engine.set_param(ParamId::FilterResonance, 0.65);
    engine.set_filter_oversampling(FilterOversampling::Off);
    engine.set_param(ParamId::Osc2Enabled, 1.0);
    engine.set_param(ParamId::OscMix, 0.5);
    engine.set_param(ParamId::SubOscLevel, 0.2);
    engine.set_param(ParamId::NoiseLevel, 0.15);
    engine.set_param(ParamId::Lfo1Rate, 5.0);
    engine.set_param(ParamId::Lfo1Depth, 0.8);
    engine.handle_control(ControlMessage::SetModulation {
        route: ModRoute::Free(0),
        enabled: true,
        source: ModSource::Lfo1,
        destination: ModDestination::FilterCutoff,
        amount: 0.75,
    });
}

fn configure_effect(engine: &mut HardwareSynth, effect: EffectType) {
    engine.set_filter_oversampling(FilterOversampling::Off);
    engine.set_param(ParamId::EffectEnabled, 1.0);
    engine.set_param(ParamId::EffectType, effect.index() as f32);
    engine.set_param(ParamId::EffectMix, 0.5);
    engine.set_param(ParamId::EffectClockSync, 0.0);
    engine.set_param(ParamId::EffectParam1, 0.5);
    engine.set_param(ParamId::EffectParam2, 0.5);
}

fn configure_worst_case(engine: &mut HardwareSynth) {
    configure_modulation_heavy(engine);
    engine.set_param(ParamId::FilterCutoff, 440.0);
    engine.set_param(ParamId::FilterResonance, 1.0);
    engine.set_filter_oversampling(FilterOversampling::Off);
    engine.set_param(ParamId::EffectEnabled, 1.0);
    engine.set_param(ParamId::EffectType, EffectType::Reverb.index() as f32);
    engine.set_param(ParamId::EffectMix, 0.5);
    engine.set_param(ParamId::EffectParam1, 0.8);
    engine.set_param(ParamId::EffectParam2, 0.6);
}

struct RawTiming {
    average: u32,
    maximum: u32,
    overruns: u32,
}

fn budget_permille(cycles: u32) -> u32 {
    (u64::from(cycles) * 1_000 / u64::from(AUDIO_BLOCK_CYCLE_BUDGET)) as u32
}

fn report_case(name: &str, raw: RawTiming, snapshot: Snapshot) {
    let average = snapshot.stage_average;
    let maximum = snapshot.stage_max;
    defmt::info!(
        "benchmark {} raw_avg={} raw_max={} raw_avg_budget_permille={} raw_max_budget_permille={} raw_overruns={}",
        name,
        raw.average,
        raw.maximum,
        budget_permille(raw.average),
        budget_permille(raw.maximum),
        raw.overruns
    );
    defmt::info!(
        "profiled avg={} max={} avg_budget_permille={} max_budget_permille={} overruns={}",
        snapshot.block_average,
        snapshot.block_max,
        budget_permille(snapshot.block_average),
        budget_permille(snapshot.block_max),
        snapshot.overruns
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
