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
    ControlMessage, DedicatedModSource, EffectType, FilterOversampling, FilterType, ModDestination,
    ModRoute, ModSource, ParamId, Patch, SynthEngineWithMemory, Waveform, profiling::RenderStage,
};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const EFFECTS_SAMPLES: usize = 48_000;
const WARMUP_BLOCKS: usize = 128;
const MEASURED_BLOCKS: usize = 512;
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
    run_scenario(&mut sdram, "U1-001-one-note", |engine| {
        engine.apply_patch(&u1_001_patch());
        engine.note_on(60, 1.0);
    });
    run_scenario(&mut sdram, "U1-001-four-notes", |engine| {
        engine.apply_patch(&u1_001_patch());
        configure_four_notes(engine);
    });
    run_scenario(&mut sdram, "U1-001-four-notes-effects-off", |engine| {
        let mut patch = u1_001_patch();
        patch.effects.enabled = false;
        engine.apply_patch(&patch);
        configure_four_notes(engine);
    });
    run_scenario(&mut sdram, "U1-001-four-notes-flat-osc1-shape", |engine| {
        let mut patch = u1_001_patch();
        patch.osc1.shape_mod = 0.0;
        patch.lfos[3].depth = 0.0;
        patch.lfos[3].destination = ModDestination::Off;
        engine.apply_patch(&patch);
        configure_four_notes(engine);
    });
    run_scenario(&mut sdram, "U1-001-four-notes-static-filter", |engine| {
        let mut patch = u1_001_patch();
        patch.filter.key_track = 0.0;
        patch.filter.velocity = 0.0;
        engine.apply_patch(&patch);
        configure_four_notes(engine);
    });
    run_scenario(&mut sdram, "U1-001-four-notes-no-modulation", |engine| {
        let mut patch = u1_001_patch();
        for lfo in &mut patch.lfos {
            lfo.depth = 0.0;
            lfo.destination = ModDestination::Off;
        }
        patch.mod_matrix = Default::default();
        engine.apply_patch(&patch);
        configure_four_notes(engine);
    });
    run_scenario(&mut sdram, "four-note-osc1-saw", |engine| {
        configure_single_oscillator_waveform(engine, Waveform::Saw);
    });
    run_scenario(&mut sdram, "four-note-osc1-triangle", |engine| {
        configure_single_oscillator_waveform(engine, Waveform::Triangle);
    });
    run_scenario(&mut sdram, "four-note-osc1-sawtri", |engine| {
        configure_single_oscillator_waveform(engine, Waveform::SawTri);
        engine.set_param(ParamId::Osc1ShapeMod, 0.5);
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
            ModDestination::Osc1ShapeMod.index() as f32,
        );
    });
    run_scenario(&mut sdram, "four-note-osc1-pulse-dc-shape", |engine| {
        configure_single_oscillator_pulse(engine, 0.5);
        engine.handle_control(ControlMessage::SetModulation {
            route: ModRoute::Free(0),
            enabled: true,
            source: ModSource::Dc,
            destination: ModDestination::Osc1ShapeMod,
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
            destination: ModDestination::Osc1ShapeMod,
            amount: 0.49,
        });
    });
    run_scenario(&mut sdram, "four-note-dual-oscillator", |engine| {
        configure_dual_oscillator(engine);
    });
    run_scenario(&mut sdram, "four-note-hard-sync", |engine| {
        configure_dual_oscillator(engine);
        engine.set_param(ParamId::Osc2Frequency, 84.0);
        engine.set_param(ParamId::HardSync, 1.0);
    });
    run_scenario(&mut sdram, "four-lane-divergent-mips", |engine| {
        configure_single_oscillator_waveform(engine, Waveform::Saw);
        engine.all_notes_off();
        for note in [24, 48, 72, 96] {
            engine.note_on(note, 1.0);
        }
    });
    run_scenario(&mut sdram, "four-note-pitch-mip-crossing", |engine| {
        configure_single_oscillator_waveform(engine, Waveform::Saw);
        engine.set_param(ParamId::Lfo1Rate, 997.0);
        engine.set_param(ParamId::Lfo1Depth, 1.0);
        engine.set_param(
            ParamId::Lfo1Destination,
            ModDestination::Osc1Frequency.index() as f32,
        );
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
    run_scenario(&mut sdram, "four-note-saw-reverb", |engine| {
        configure_single_oscillator_waveform(engine, Waveform::Saw);
        configure_effect(engine, EffectType::Reverb);
    });
    run_scenario(&mut sdram, "four-note-triangle-reverb", |engine| {
        configure_single_oscillator_waveform(engine, Waveform::Triangle);
        configure_effect(engine, EffectType::Reverb);
    });
    run_scenario(&mut sdram, "four-note-pwm-reverb", |engine| {
        configure_pwm(engine);
        configure_effect(engine, EffectType::Reverb);
    });
    run_scenario(&mut sdram, "four-note-cutoff-lfo-reverb", |engine| {
        configure_cutoff_matrix_lfo(engine);
        configure_effect(engine, EffectType::Reverb);
    });
    run_scenario(&mut sdram, "four-note-dual-oscillator-reverb", |engine| {
        configure_dual_oscillator(engine);
        configure_effect(engine, EffectType::Reverb);
    });
    for destination in [
        ModDestination::FxMix,
        ModDestination::FxParam1,
        ModDestination::FxParam2,
    ] {
        run_scenario(&mut sdram, destination.name(), |engine| {
            configure_single_oscillator_waveform(engine, Waveform::Saw);
            configure_effect(engine, EffectType::Reverb);
            configure_lfo_route(engine, ModRoute::Free(0), destination, 0.75);
        });
    }
    run_scenario(&mut sdram, "four-note-eight-free-routes-reverb", |engine| {
        configure_eight_free_routes(engine);
        configure_effect(engine, EffectType::Reverb);
    });
    run_scenario(&mut sdram, "four-note-route-saturation-reverb", |engine| {
        configure_route_saturation(engine);
        configure_effect(engine, EffectType::Reverb);
    });
    run_effect_transition_scenario(&mut sdram);
    run_scenario(&mut sdram, "four-note-self-oscillation-off", |engine| {
        configure_self_oscillation(engine, FilterOversampling::Off);
    });
    run_scenario(&mut sdram, "four-note-self-oscillation-x2", |engine| {
        configure_self_oscillation(engine, FilterOversampling::X2);
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
    configure_benchmark_defaults(&mut engine);
    configure(&mut engine);
    benchmark_case(&mut engine, name);
}

fn configure_benchmark_defaults(engine: &mut HardwareSynth) {
    // Match production firmware unless a filter-comparison scenario
    // explicitly selects another model or oversampling mode.
    engine.set_filter_type(DEFAULT_FILTER_TYPE);
    engine.set_filter_oversampling(DEFAULT_FILTER_OVERSAMPLING);
}

/// Prophet Rev2 factory preset U1-001, "LosVangelis2041".
///
/// Values are decoded from bank 0/program 0 of Rev2_Programs_v1.0.syx so the
/// embedded benchmark does not need to carry the 1.2 MB factory bank.
fn u1_001_patch() -> Patch {
    let mut patch = Patch::default();

    patch.osc1.waveform = 0;
    patch.osc1.enabled = true;
    patch.osc1.frequency = 24.0;
    patch.osc1.fine_tune = -2.0;
    patch.osc1.shape_mod = 0.505_050_5;
    patch.osc1.level = 1.0;
    patch.osc1.note_reset = false;
    patch.osc1.keyboard_on = true;
    patch.osc1.glide = false;

    patch.osc2.waveform = 3;
    patch.osc2.enabled = true;
    patch.osc2.frequency = 24.0;
    patch.osc2.fine_tune = 2.0;
    patch.osc2.shape_mod = 0.434_343_43;
    patch.osc2.level = 1.0;
    patch.osc2.note_reset = false;
    patch.osc2.keyboard_on = true;
    patch.osc2.glide = false;

    patch.osc_mix = 0.503_937;
    patch.sub_osc_level = 0.0;
    patch.noise_level = 0.0;
    patch.hard_sync = false;
    patch.osc_slop = 0.078_740_16;
    patch.glide_time = 0.0;

    patch.filter.cutoff = 557.380_7;
    patch.filter.resonance = 0.0;
    patch.filter.poles = 4;
    patch.filter.key_track = 0.173_228_35;
    patch.filter.env_amount = 0.0;
    patch.filter.velocity = 0.480_314_97;
    patch.filter.audio_mod = 0.0;
    patch.filter.eg_delay = 0.0;
    patch.filter.eg_attack = 2.677_397_5;
    patch.filter.eg_decay = 4.606_338_5;
    patch.filter.eg_sustain = 0.346_456_7;
    patch.filter.eg_release = 7.795_386_3;

    patch.amplifier.pan_spread = 0.078_740_16;
    patch.amplifier.env_amount = 0.448_818_9;
    patch.amplifier.velocity = 1.0;
    patch.amplifier.eg_delay = 0.0;
    patch.amplifier.eg_attack = 1.220_850_3;
    patch.amplifier.eg_decay = 3.031_692_7;
    patch.amplifier.eg_sustain = 1.0;
    patch.amplifier.eg_release = 7.086_760_5;

    patch.aux_envelope.destination = ModDestination::Off;
    patch.aux_envelope.amount = 0.0;
    patch.aux_envelope.velocity = 0.0;
    patch.aux_envelope.delay = 0.0;
    patch.aux_envelope.attack = 0.000_5;
    patch.aux_envelope.decay = 0.000_5;
    patch.aux_envelope.sustain = 0.0;
    patch.aux_envelope.release = 0.000_5;
    patch.aux_envelope.repeat = false;

    for (index, (rate_hz, depth, destination)) in [
        (2.538_176, 0.0, ModDestination::Off),
        (0.023_521_572, 0.007_874_016, ModDestination::FxMix),
        (0.261_238_8, 0.409_448_83, ModDestination::SubOscLevel),
        (0.319_277_8, 0.299_212_6, ModDestination::Osc1ShapeMod),
    ]
    .into_iter()
    .enumerate()
    {
        let lfo = &mut patch.lfos[index];
        lfo.rate_hz = rate_hz;
        lfo.depth = depth;
        lfo.destination = destination;
        lfo.clock_sync = false;
        lfo.key_sync = false;
    }

    let route = &mut patch.mod_matrix.free_slots[0];
    route.enabled = true;
    route.source = ModSource::Lfo1;
    route.destination = ModDestination::OscAllFrequency;
    route.amount = 0.0;

    let route = &mut patch.mod_matrix.free_slots[1];
    route.enabled = true;
    route.source = ModSource::Lfo2;
    route.destination = ModDestination::Osc2ShapeMod;
    route.amount = 0.0;

    let route = &mut patch.mod_matrix.free_slots[7];
    route.enabled = true;
    route.source = ModSource::ModWheel;
    route.destination = ModDestination::Vca;
    route.amount = 0.015_748_024;

    let route = &mut patch.mod_matrix.dedicated[0];
    route.enabled = true;
    route.destination = ModDestination::OscSlop;
    route.amount = 0.338_582_63;

    let route = &mut patch.mod_matrix.dedicated[1];
    route.enabled = true;
    route.destination = ModDestination::EnvAllRelease;
    route.amount = 0.007_874_012;

    let route = &mut patch.mod_matrix.dedicated[3];
    route.enabled = true;
    route.destination = ModDestination::EnvAllRelease;
    route.amount = 0.007_874_012;

    patch.effects.enabled = true;
    patch.effects.effect_type = EffectType::BucketBrigadeDelay;
    patch.effects.mix = 0.0;
    patch.effects.clock_sync = false;
    patch.effects.param1 = 0.333_333_34;
    patch.effects.param2 = 0.377_952_75;
    patch.master_volume = 1.0;

    patch
}

fn benchmark_case(engine: &mut HardwareSynth, name: &str) {
    let (raw, snapshot) = measure_case(engine);
    report_case(name, raw, snapshot);
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
    let mut timing = RawTiming::new();
    for _ in 0..MEASURED_BLOCKS {
        let started = DWT::cycle_count();
        engine.process_interleaved(output, 2);
        let cycles = DWT::cycle_count().wrapping_sub(started);
        timing.observe(cycles);
        black_box(output[0]);
    }
    timing
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
    engine.set_param(ParamId::Osc1ShapeMod, 0.0);
    engine.set_param(ParamId::Osc2Enabled, 0.0);
    engine.set_param(ParamId::OscMix, 0.0);
    engine.set_param(ParamId::SubOscLevel, 0.0);
    engine.set_param(ParamId::NoiseLevel, 0.0);
    engine.set_param(ParamId::HardSync, 0.0);
    engine.set_param(ParamId::OscSlop, 0.0);
}

fn configure_single_oscillator_pulse(engine: &mut HardwareSynth, shape: f32) {
    configure_single_oscillator_waveform(engine, Waveform::Pulse);
    engine.set_param(ParamId::Osc1ShapeMod, shape);
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

fn configure_dual_oscillator(engine: &mut HardwareSynth) {
    configure_single_oscillator_waveform(engine, Waveform::Saw);
    engine.set_param(ParamId::Osc2Enabled, 1.0);
    engine.set_param(ParamId::Osc2Waveform, Waveform::Saw as u8 as f32);
    engine.set_param(ParamId::OscMix, 0.5);
}

fn configure_pwm(engine: &mut HardwareSynth) {
    configure_single_oscillator_pulse(engine, 0.5);
    configure_lfo_route(engine, ModRoute::Free(0), ModDestination::Osc1ShapeMod, 0.49);
}

fn configure_cutoff_matrix_lfo(engine: &mut HardwareSynth) {
    configure_cutoff_modulation_base(engine);
    configure_lfo_route(
        engine,
        ModRoute::Free(0),
        ModDestination::FilterCutoff,
        0.75,
    );
}

fn configure_lfo_route(
    engine: &mut HardwareSynth,
    route: ModRoute,
    destination: ModDestination,
    amount: f32,
) {
    configure_cutoff_lfo(engine);
    engine.handle_control(ControlMessage::SetModulation {
        route,
        enabled: true,
        source: ModSource::Lfo1,
        destination,
        amount,
    });
}

fn configure_eight_free_routes(engine: &mut HardwareSynth) {
    configure_dual_oscillator(engine);
    for (param, rate) in [
        (ParamId::Lfo1Rate, 2.0),
        (ParamId::Lfo2Rate, 3.0),
        (ParamId::Lfo3Rate, 5.0),
        (ParamId::Lfo4Rate, 7.0),
    ] {
        engine.set_param(param, rate);
    }
    for param in [
        ParamId::Lfo1Depth,
        ParamId::Lfo2Depth,
        ParamId::Lfo3Depth,
        ParamId::Lfo4Depth,
    ] {
        engine.set_param(param, 0.8);
    }
    let sources = [
        ModSource::Lfo1,
        ModSource::Lfo2,
        ModSource::Lfo3,
        ModSource::Lfo4,
        ModSource::Env3,
        ModSource::Velocity,
        ModSource::NoteNumber,
        ModSource::Dc,
    ];
    let destinations = [
        ModDestination::Osc1ShapeMod,
        ModDestination::Osc2ShapeMod,
        ModDestination::FilterCutoff,
        ModDestination::FilterResonance,
        ModDestination::OscMix,
        ModDestination::Pan,
        ModDestination::FxParam1,
        ModDestination::FxParam2,
    ];
    for index in 0..8 {
        engine.handle_control(ControlMessage::SetModulation {
            route: ModRoute::Free(index),
            enabled: true,
            source: sources[index],
            destination: destinations[index],
            amount: 0.3,
        });
    }
}

fn configure_route_saturation(engine: &mut HardwareSynth) {
    configure_eight_free_routes(engine);
    for (destination_param, destination) in [
        (ParamId::Lfo1Destination, ModDestination::Osc1Frequency),
        (ParamId::Lfo2Destination, ModDestination::Osc2Frequency),
        (ParamId::Lfo3Destination, ModDestination::NoiseLevel),
        (ParamId::Lfo4Destination, ModDestination::SubOscLevel),
    ] {
        engine.set_param(destination_param, destination.index() as f32);
    }
    engine.set_param(
        ParamId::AuxEgDestination,
        ModDestination::FilterAudioMod.index() as f32,
    );
    engine.set_param(ParamId::AuxEgAmount, 0.5);

    for (source, destination) in [
        (DedicatedModSource::ModWheel, ModDestination::Vca),
        (DedicatedModSource::Pressure, ModDestination::OscMix),
        (DedicatedModSource::Breath, ModDestination::NoiseLevel),
        (DedicatedModSource::Velocity, ModDestination::Pan),
        (DedicatedModSource::Footswitch, ModDestination::FxMix),
    ] {
        engine.handle_control(ControlMessage::SetModulation {
            route: ModRoute::Dedicated(source),
            enabled: true,
            source: source.source(),
            destination,
            amount: 0.25,
        });
    }
}

fn run_effect_transition_scenario(sdram: &mut Sdram) {
    let effects_memory = sdram
        .allocate_f32(EFFECTS_SAMPLES)
        .expect("SDRAM effects allocation failed");
    let mut engine = HardwareSynth::new_with_effects_memory(SAMPLE_RATE_HZ, effects_memory);
    configure_benchmark_defaults(&mut engine);
    configure_single_oscillator_waveform(&mut engine, Waveform::Saw);
    configure_effect(&mut engine, EffectType::Reverb);
    let mut output = [0.0f32; BLOCK_LENGTH * 2];
    for _ in 0..WARMUP_BLOCKS {
        engine.process_interleaved(&mut output, 2);
    }

    let mut timing = RawTiming::new();
    for block in 0..MEASURED_BLOCKS {
        let started = DWT::cycle_count();
        let effect = if block & 1 == 0 {
            EffectType::DelayMono
        } else {
            EffectType::Reverb
        };
        engine.set_param(ParamId::EffectType, effect.index() as f32);
        engine.set_param(ParamId::EffectParam1, (block & 31) as f32 / 31.0);
        engine.set_param(ParamId::EffectParam2, ((block + 11) & 31) as f32 / 31.0);
        engine.process_interleaved(&mut output, 2);
        timing.observe(DWT::cycle_count().wrapping_sub(started));
        black_box(output[0]);
    }
    report_raw_case("effect-transition-control-stress", timing);
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

const RAW_HISTOGRAM_BINS: usize = 128;
const RAW_HISTOGRAM_RANGE: u32 = AUDIO_BLOCK_CYCLE_BUDGET * 5 / 4;

struct RawTiming {
    total: u64,
    blocks: u32,
    maximum: u32,
    overruns: u32,
    histogram: [u16; RAW_HISTOGRAM_BINS],
}

impl RawTiming {
    const fn new() -> Self {
        Self {
            total: 0,
            blocks: 0,
            maximum: 0,
            overruns: 0,
            histogram: [0; RAW_HISTOGRAM_BINS],
        }
    }

    fn observe(&mut self, cycles: u32) {
        self.total += u64::from(cycles);
        self.blocks += 1;
        self.maximum = self.maximum.max(cycles);
        self.overruns += u32::from(cycles > AUDIO_BLOCK_CYCLE_BUDGET);
        let bin = ((u64::from(cycles.min(RAW_HISTOGRAM_RANGE)) * RAW_HISTOGRAM_BINS as u64)
            / u64::from(RAW_HISTOGRAM_RANGE))
        .min((RAW_HISTOGRAM_BINS - 1) as u64) as usize;
        self.histogram[bin] = self.histogram[bin].saturating_add(1);
    }

    fn average(&self) -> u32 {
        (self.total / u64::from(self.blocks.max(1))) as u32
    }

    fn quantile(&self, percentile: u32) -> u32 {
        let target = (u64::from(self.blocks) * u64::from(percentile) + 99) / 100;
        let mut cumulative = 0u64;
        for (index, count) in self.histogram.iter().enumerate() {
            cumulative += u64::from(*count);
            if cumulative >= target {
                return ((index as u64 + 1) * u64::from(RAW_HISTOGRAM_RANGE)
                    / RAW_HISTOGRAM_BINS as u64) as u32;
            }
        }
        RAW_HISTOGRAM_RANGE
    }
}

fn budget_permille(cycles: u32) -> u32 {
    (u64::from(cycles) * 1_000 / u64::from(AUDIO_BLOCK_CYCLE_BUDGET)) as u32
}

fn report_case(name: &str, raw: RawTiming, snapshot: Snapshot) {
    let average = snapshot.stage_average;
    let worst = snapshot.stage_worst_block;
    defmt::info!(
        "benchmark {} raw_avg={} raw_p95={} raw_p99={} raw_max={} raw_max_permille={} headroom={} raw_overruns={}",
        name,
        raw.average(),
        raw.quantile(95),
        raw.quantile(99),
        raw.maximum,
        budget_permille(raw.maximum),
        AUDIO_BLOCK_CYCLE_BUDGET.saturating_sub(raw.maximum),
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
        "stage worst env_mod={} osc={} filter={} amp_pan={} effects={} output={}",
        worst[RenderStage::EnvelopesAndModulation.index()],
        worst[RenderStage::Oscillators.index()],
        worst[RenderStage::Filter.index()],
        worst[RenderStage::AmplifierAndPan.index()],
        worst[RenderStage::Effects.index()],
        worst[RenderStage::MasterOutput.index()]
    );
    defmt::info!(
        "modulation worst envelopes={} lfo_control={} lfo_generation={} audio_routes={}",
        worst[RenderStage::EnvelopeAdvance.index()],
        worst[RenderStage::LfoControlRouting.index()],
        worst[RenderStage::LfoGeneration.index()],
        worst[RenderStage::AudioModulationRouting.index()]
    );
    defmt::info!(
        "oscillator worst control={} waveform={} mix={}",
        worst[RenderStage::OscillatorControl.index()],
        worst[RenderStage::OscillatorWaveform.index()],
        worst[RenderStage::OscillatorMix.index()]
    );
    defmt::info!(
        "effects avg prepare={} combs={} allpasses={} mix={}",
        average[RenderStage::EffectsPreparation.index()],
        average[RenderStage::ReverbCombs.index()],
        average[RenderStage::ReverbAllpasses.index()],
        average[RenderStage::EffectsMix.index()]
    );
    defmt::info!(
        "effects worst prepare={} combs={} allpasses={} mix={}",
        worst[RenderStage::EffectsPreparation.index()],
        worst[RenderStage::ReverbCombs.index()],
        worst[RenderStage::ReverbAllpasses.index()],
        worst[RenderStage::EffectsMix.index()]
    );
}

fn report_raw_case(name: &str, raw: RawTiming) {
    defmt::info!(
        "benchmark {} raw_avg={} raw_p95={} raw_p99={} raw_max={} raw_max_permille={} headroom={} raw_overruns={}",
        name,
        raw.average(),
        raw.quantile(95),
        raw.quantile(99),
        raw.maximum,
        budget_permille(raw.maximum),
        AUDIO_BLOCK_CYCLE_BUDGET.saturating_sub(raw.maximum),
        raw.overruns
    );
}
