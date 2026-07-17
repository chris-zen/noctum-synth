#![no_std]
#![no_main]

use core::hint::black_box;
use cortex_m::peripheral::DWT;
use embassy_daisy::Board;
use embassy_daisy::audio::BLOCK_LENGTH;
use embassy_daisy::sdram::Sdram;
use {defmt_rtt as _, panic_probe as _};

use analog_synth_daisy_firmware::profiling::{AudioProfiler, Snapshot};
use synth_core::{
    ControlMessage, EffectType, FilterOversampling, FilterType, ModDestination, ModRoute,
    ModSource, ParamId, SynthEngineWithMemory, Waveform, profiling::RenderStage,
};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const EFFECTS_SAMPLES: usize = 48_000;
const WARMUP_BLOCKS: usize = 128;
const MEASURED_BLOCKS: usize = 512;
// Keep focused hardware iterations short; set false for the complete stress suite.
const CUTOFF_MODULATION_BENCHMARK_ONLY: bool = true;
const DEFAULT_FILTER_TYPE: FilterType = FilterType::GainLimitedTpt;
const DEFAULT_FILTER_OVERSAMPLING: FilterOversampling = FilterOversampling::Off;
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
    run_scenario(&mut sdram, "four-note-osc1-saw", |engine| {
        configure_single_oscillator_waveform(engine, Waveform::Saw);
    });
    run_scenario(&mut sdram, "four-note-osc1-triangle", |engine| {
        configure_single_oscillator_waveform(engine, Waveform::Triangle);
    });
    run_scenario(&mut sdram, "four-note-osc1-pulse-square", |engine| {
        configure_single_oscillator_pulse(engine, 0.0);
    });
    run_scenario(&mut sdram, "four-note-osc1-pulse-wide", |engine| {
        configure_single_oscillator_pulse(engine, 0.5);
    });
    run_scenario(&mut sdram, "four-note-osc1-pulse-extreme", |engine| {
        configure_single_oscillator_pulse(engine, 1.0);
    });
    run_scenario(&mut sdram, "four-note-osc1-pulse-lfo-unrouted", |engine| {
        configure_single_oscillator_pulse(engine, 0.5);
        engine.set_param(ParamId::Lfo1Rate, 5.0);
        engine.set_param(ParamId::Lfo1Depth, 1.0);
    });
    run_scenario(&mut sdram, "four-note-osc1-pulse-direct-pwm", |engine| {
        configure_single_oscillator_pulse(engine, 0.5);
        engine.set_param(ParamId::Lfo1Rate, 5.0);
        engine.set_param(ParamId::Lfo1Depth, 1.0);
        engine.set_param(
            ParamId::Lfo1Destination,
            ModDestination::Osc1Shape.index() as f32,
        );
    });
    run_scenario(&mut sdram, "four-note-osc1-pulse-dc-shape", |engine| {
        configure_single_oscillator_pulse(engine, 0.5);
        engine.handle_control(ControlMessage::SetModulation {
            route: ModRoute::Free(0),
            enabled: true,
            source: ModSource::Dc,
            destination: ModDestination::Osc1Shape,
            amount: 0.49,
        });
    });
    run_scenario(&mut sdram, "four-note-osc1-pulse-pwm", |engine| {
        configure_single_oscillator_pulse(engine, 0.5);
        engine.set_param(ParamId::Lfo1Rate, 5.0);
        engine.set_param(ParamId::Lfo1Depth, 1.0);
        engine.handle_control(ControlMessage::SetModulation {
            route: ModRoute::Free(0),
            enabled: true,
            source: ModSource::Lfo1,
            destination: ModDestination::Osc1Shape,
            amount: 0.49,
        });
    });
    run_scenario(&mut sdram, "four-note-filter-static", |engine| {
        configure_cutoff_modulation_base(engine);
    });
    run_scenario(&mut sdram, "four-note-filter-lfo-unrouted", |engine| {
        configure_cutoff_modulation_base(engine);
        configure_cutoff_lfo(engine);
    });
    run_scenario(&mut sdram, "four-note-filter-cutoff-dc", |engine| {
        configure_cutoff_modulation_base(engine);
        engine.handle_control(ControlMessage::SetModulation {
            route: ModRoute::Free(0),
            enabled: true,
            source: ModSource::Dc,
            destination: ModDestination::FilterCutoff,
            amount: 0.75,
        });
    });
    run_scenario(&mut sdram, "four-note-filter-cutoff-direct-lfo", |engine| {
        configure_cutoff_modulation_base(engine);
        configure_cutoff_lfo(engine);
        engine.set_param(
            ParamId::Lfo1Destination,
            ModDestination::FilterCutoff.index() as f32,
        );
    });
    run_scenario(&mut sdram, "four-note-filter-cutoff-matrix-lfo", |engine| {
        configure_cutoff_modulation_base(engine);
        configure_cutoff_lfo(engine);
        engine.handle_control(ControlMessage::SetModulation {
            route: ModRoute::Free(0),
            enabled: true,
            source: ModSource::Lfo1,
            destination: ModDestination::FilterCutoff,
            amount: 0.75,
        });
    });
    if CUTOFF_MODULATION_BENCHMARK_ONLY {
        defmt::info!("cutoff-modulation benchmark complete");
        loop {
            cortex_m::asm::wfi();
        }
    }
    for filter_type in FilterType::ALL {
        run_filter_scenario(
            &mut sdram,
            filter_type,
            "four-note-active-filter-off",
            |engine| {
                configure_four_notes(engine);
                engine.set_param(ParamId::FilterCutoff, 1_200.0);
                engine.set_param(ParamId::FilterResonance, 0.65);
            },
        );
        run_filter_scenario(
            &mut sdram,
            filter_type,
            "four-note-self-oscillation-off",
            |engine| configure_self_oscillation(engine, FilterOversampling::Off),
        );
        run_filter_scenario(
            &mut sdram,
            filter_type,
            "four-note-self-oscillation-x2",
            |engine| configure_self_oscillation(engine, FilterOversampling::X2),
        );
        run_filter_scenario(
            &mut sdram,
            filter_type,
            "four-note-modulation-heavy",
            |engine| {
                configure_four_notes(engine);
                configure_modulation_heavy(engine);
            },
        );
    }

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
    configure_benchmark_defaults(&mut engine);
    configure(&mut engine);
    benchmark_case(&mut engine, name);
}

fn run_filter_scenario(
    sdram: &mut Sdram,
    filter_type: FilterType,
    case: &str,
    configure: impl FnOnce(&mut HardwareSynth),
) {
    let effects_memory = sdram
        .allocate_f32(EFFECTS_SAMPLES)
        .expect("SDRAM effects allocation failed");
    let mut engine = HardwareSynth::new_with_effects_memory(SAMPLE_RATE_HZ, effects_memory);
    configure_benchmark_defaults(&mut engine);
    engine.set_filter_type(filter_type);
    configure(&mut engine);
    benchmark_filter_case(&mut engine, filter_type, case);
}

fn configure_benchmark_defaults(engine: &mut HardwareSynth) {
    // Match production firmware unless a filter-comparison scenario
    // explicitly selects another model or oversampling mode.
    engine.set_filter_type(DEFAULT_FILTER_TYPE);
    engine.set_filter_oversampling(DEFAULT_FILTER_OVERSAMPLING);
}

fn benchmark_case(engine: &mut HardwareSynth, name: &str) {
    let (raw, snapshot) = measure_case(engine);
    report_case(name, raw, snapshot);
}

fn benchmark_filter_case(engine: &mut HardwareSynth, filter_type: FilterType, case: &str) {
    let (raw, snapshot) = measure_case(engine);
    defmt::info!("filter model={} case={}", filter_type.name(), case);
    report_case(case, raw, snapshot);
}

fn measure_case(engine: &mut HardwareSynth) -> (RawTiming, Snapshot) {
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
    (raw, profiler.take_snapshot())
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

fn configure_single_oscillator_waveform(engine: &mut HardwareSynth, waveform: Waveform) {
    configure_four_notes(engine);
    engine.set_param(ParamId::Osc1Enabled, 1.0);
    engine.set_param(ParamId::Osc1Waveform, waveform as u8 as f32);
    engine.set_param(ParamId::Osc1Shape, 0.0);
    engine.set_param(ParamId::Osc2Enabled, 0.0);
    engine.set_param(ParamId::OscMix, 0.0);
    engine.set_param(ParamId::SubOscLevel, 0.0);
    engine.set_param(ParamId::NoiseLevel, 0.0);
    engine.set_param(ParamId::HardSync, 0.0);
    engine.set_param(ParamId::OscSlop, 0.0);
}

fn configure_single_oscillator_pulse(engine: &mut HardwareSynth, shape: f32) {
    configure_single_oscillator_waveform(engine, Waveform::Pulse);
    engine.set_param(ParamId::Osc1Shape, shape);
}

fn configure_self_oscillation(engine: &mut HardwareSynth, oversampling: FilterOversampling) {
    configure_four_notes(engine);
    engine.set_param(ParamId::Osc1Enabled, 0.0);
    engine.set_param(ParamId::Osc2Enabled, 0.0);
    engine.set_param(ParamId::SubOscLevel, 0.0);
    engine.set_param(ParamId::NoiseLevel, 0.0);
    engine.set_param(ParamId::FilterCutoff, 440.0);
    engine.set_param(ParamId::FilterResonance, 1.0);
    engine.set_filter_oversampling(oversampling);
}

fn configure_cutoff_modulation_base(engine: &mut HardwareSynth) {
    configure_single_oscillator_waveform(engine, Waveform::Saw);
    engine.set_param(ParamId::FilterCutoff, 1_200.0);
    engine.set_param(ParamId::FilterResonance, 0.65);
    engine.set_filter_oversampling(FilterOversampling::Off);
}

fn configure_cutoff_lfo(engine: &mut HardwareSynth) {
    engine.set_param(ParamId::Lfo1Rate, 5.0);
    engine.set_param(ParamId::Lfo1Depth, 0.8);
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
        average[RenderStage::EnvelopesAndModulation.index()],
        average[RenderStage::Oscillators.index()],
        average[RenderStage::Filter.index()],
        average[RenderStage::AmplifierAndPan.index()],
        average[RenderStage::Effects.index()],
        average[RenderStage::MasterOutput.index()]
    );
    defmt::info!(
        "modulation avg envelopes={} lfo_control={} lfo_generation={} audio_routes={}",
        average[RenderStage::EnvelopeAdvance.index()],
        average[RenderStage::LfoControlRouting.index()],
        average[RenderStage::LfoGeneration.index()],
        average[RenderStage::AudioModulationRouting.index()]
    );
    defmt::info!(
        "oscillator avg control={} waveform={} mix={}",
        average[RenderStage::OscillatorControl.index()],
        average[RenderStage::OscillatorWaveform.index()],
        average[RenderStage::OscillatorMix.index()]
    );
    defmt::info!(
        "stage max env_mod={} osc={} filter={} amp_pan={} effects={} output={}",
        maximum[RenderStage::EnvelopesAndModulation.index()],
        maximum[RenderStage::Oscillators.index()],
        maximum[RenderStage::Filter.index()],
        maximum[RenderStage::AmplifierAndPan.index()],
        maximum[RenderStage::Effects.index()],
        maximum[RenderStage::MasterOutput.index()]
    );
    defmt::info!(
        "modulation max envelopes={} lfo_control={} lfo_generation={} audio_routes={}",
        maximum[RenderStage::EnvelopeAdvance.index()],
        maximum[RenderStage::LfoControlRouting.index()],
        maximum[RenderStage::LfoGeneration.index()],
        maximum[RenderStage::AudioModulationRouting.index()]
    );
    defmt::info!(
        "oscillator max control={} waveform={} mix={}",
        maximum[RenderStage::OscillatorControl.index()],
        maximum[RenderStage::OscillatorWaveform.index()],
        maximum[RenderStage::OscillatorMix.index()]
    );
}
