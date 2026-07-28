#![no_std]
#![no_main]

use core::hint::black_box;
use cortex_m::peripheral::DWT;
use {defmt_rtt as _, panic_probe as _};

use embassy_daisy::audio::BLOCK_LENGTH;
use embassy_daisy::qspi::QspiFlash;
use embassy_daisy::Board;
use noctum_micro::audio::{AdaptiveControlBudget, ControlQueue, PatchQueue, BLOCK_CYCLE_BUDGET};
use noctum_micro::patch_transition::PatchTransition;
use noctum_micro::profiling::{AudioProfiler, Snapshot};
use noctum_micro::model::{FILTER_OVERSAMPLING, FILTER_TYPE};
use synth_core::midi::rev2::{MidiDecoder, PROGRAM_DATA_SYSEX_LEN};
use synth_core::{
    profiling::RenderStage, ControlMessage, ModDestination, ParamId, SynthEngineWithMemory,
};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const EFFECTS_SAMPLES: usize = 48_000 * 2;
const FACTORY_PRESET_COUNT: usize = 512;
const PRESETS_PER_BANK: usize = 128;

/// The combined bootloader image stores the bank immediately after the maximum
/// 512 KiB application storage reservation. QSPI commands use offsets relative
/// to the 0x9000_0000 memory-mapped base.
const FACTORY_BANK_QSPI_OFFSET: u32 = embassy_daisy::qspi::APPLICATION_RESERVED_END;
const FACTORY_BANK_SIZE: usize = FACTORY_PRESET_COUNT * PROGRAM_DATA_SYSEX_LEN;
const FACTORY_BANK_CRC32: u32 = 0x3df3_3c23;

const WARMUP_BLOCKS: usize = 128;
const ATTACK_BLOCKS: usize = 128;
const RAW_BLOCKS: usize = 512;
const CONTROL_STRESS_BLOCKS: usize = 128;
const PROFILED_BLOCKS: usize = 256;
const PROFILE_THRESHOLD_CYCLES: u32 = 272_000;
const PROFILE_TRIGGER_CYCLES: u32 = PROFILE_THRESHOLD_CYCLES;

type HardwareSynth<'a> = SynthEngineWithMemory<1, &'a mut [f32]>;

#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::info!(
        "initializing factory-preset benchmark presets={} bytes={} qspi_offset={:#x}",
        FACTORY_PRESET_COUNT,
        FACTORY_BANK_SIZE,
        FACTORY_BANK_QSPI_OFFSET
    );

    let mut core = cortex_m::Peripherals::take().expect("Cortex-M peripherals already initialized");
    core.DCB.enable_trace();
    core.DWT.enable_cycle_counter();
    core.SCB.disable_dcache(&mut core.CPUID);
    core.SCB.enable_icache();

    let parts = Board::take().expect("Daisy board already initialized");
    let mut qspi = QspiFlash::new(parts.qspi);
    let mut sdram = parts
        .sdram
        .init(&mut core.MPU, &mut core.SCB, &mut core.CPUID)
        .expect("SDRAM data/address-line test failed");
    let effects_memory = sdram
        .allocate_f32(EFFECTS_SAMPLES)
        .expect("SDRAM effects allocation failed");

    let mut message = [0_u8; PROGRAM_DATA_SYSEX_LEN];
    validate_factory_bank(&mut qspi, &mut message);

    let mut summary = Summary::new();
    let mut adaptive_summary = AdaptiveBudgetSummary::new();
    for index in 0..FACTORY_PRESET_COUNT {
        read_message(&mut qspi, index, &mut message);
        let program = match MidiDecoder::program_data(&message) {
            Ok(program) => program,
            Err(_) => bank_failure(index, "decode failed after validation"),
        };

        // Reconstructing the engine gives every program identical voice,
        // envelope, limiter, and delay-history state. The SDRAM slice is
        // reborrowed and reused; initialization is outside every timed region.
        let mut engine =
            HardwareSynth::new_with_effects_memory(SAMPLE_RATE_HZ, &mut *effects_memory);
        engine.set_filter_type(FILTER_TYPE);
        engine.set_filter_oversampling(FILTER_OVERSAMPLING);
        for note in [60, 64, 67, 72] {
            engine.note_on(note, 1.0);
        }

        let mut output = [0.0_f32; BLOCK_LENGTH * 2];
        let mut dma_output = [(0.0_f32, 0.0_f32); BLOCK_LENGTH];
        let controls = ControlQueue::new();
        let patches = PatchQueue::new();
        let mut adaptive_budget = AdaptiveControlBudget::new();
        let transition = measure_transition(
            &mut engine,
            &program.patch,
            &mut output,
            &mut dma_output,
            &controls,
            &patches,
            &mut adaptive_budget,
        );
        report_raw(
            program.bank,
            program.program,
            Scenario::PatchTransition,
            transition,
        );
        summary.observe(
            program.bank,
            program.program,
            Scenario::PatchTransition,
            transition,
        );

        engine.all_notes_off();
        for note in [60, 64, 67, 72] {
            assert!(controls
                .try_send(ControlMessage::NoteOn {
                    note,
                    velocity: 1.0,
                })
                .is_ok());
        }
        let attack = measure_raw(
            &mut engine,
            &mut output,
            &mut dma_output,
            ATTACK_BLOCKS,
            &controls,
            &patches,
            &mut adaptive_budget,
        );
        report_raw(program.bank, program.program, Scenario::Attack, attack);
        summary.observe(program.bank, program.program, Scenario::Attack, attack);

        warm_up(&mut engine, &mut output);
        let steady = measure_raw(
            &mut engine,
            &mut output,
            &mut dma_output,
            RAW_BLOCKS,
            &controls,
            &patches,
            &mut adaptive_budget,
        );
        report_raw(program.bank, program.program, Scenario::Steady, steady);
        summary.observe(program.bank, program.program, Scenario::Steady, steady);
        summary.observe_features(&program.patch, steady);

        let control_stress = measure_control_stress(
            &mut engine,
            &program.patch,
            &mut output,
            &mut dma_output,
            &controls,
            &patches,
            &mut adaptive_budget,
        );
        report_raw(
            program.bank,
            program.program,
            Scenario::ControlStress,
            control_stress,
        );
        summary.observe(
            program.bank,
            program.program,
            Scenario::ControlStress,
            control_stress,
        );

        if [transition, attack, steady, control_stress]
            .iter()
            .any(|timing| timing.maximum >= PROFILE_TRIGGER_CYCLES)
        {
            engine.apply_patch(&program.patch);
            let snapshot = measure_profiled(&mut engine, &mut output);
            report_profile(program.bank, program.program, snapshot);
        }

        drop(engine);
        adaptive_summary.measure(
            &program.patch,
            &mut *effects_memory,
            program.bank,
            program.program,
        );
    }

    summary.report();
    adaptive_summary.report();
    defmt::info!("factory-preset benchmark complete");
    loop {
        cortex_m::asm::wfi();
    }
}

fn validate_factory_bank(qspi: &mut QspiFlash, message: &mut [u8; PROGRAM_DATA_SYSEX_LEN]) {
    let mut crc = Crc32::new();
    for index in 0..FACTORY_PRESET_COUNT {
        read_message(qspi, index, message);
        crc.update(message);
        let program = match MidiDecoder::program_data(message) {
            Ok(program) => program,
            Err(_) => bank_failure(index, "invalid SysEx message"),
        };
        let expected_bank = (index / PRESETS_PER_BANK) as u8;
        let expected_program = (index % PRESETS_PER_BANK) as u8;
        if program.bank != expected_bank || program.program != expected_program {
            bank_failure(index, "unexpected bank/program metadata");
        }
    }
    let actual_crc = crc.finish();
    if actual_crc != FACTORY_BANK_CRC32 {
        defmt::error!(
            "factory bank CRC mismatch expected={:#x} actual={:#x}",
            FACTORY_BANK_CRC32,
            actual_crc
        );
        loop {
            cortex_m::asm::wfi();
        }
    }
    defmt::info!("validated factory bank CRC32={:#x}", actual_crc);
}

fn read_message(qspi: &mut QspiFlash, index: usize, message: &mut [u8; PROGRAM_DATA_SYSEX_LEN]) {
    let message_offset: u32 = (index * PROGRAM_DATA_SYSEX_LEN)
        .try_into()
        .expect("factory-bank address fits u32");
    let address = FACTORY_BANK_QSPI_OFFSET + message_offset;
    if qspi.read(address, message).is_err() {
        bank_failure(index, "QSPI read failed");
    }
}

fn bank_failure(index: usize, reason: &str) -> ! {
    defmt::error!(
        "factory bank failure index={} bank={} program={} reason={=str}",
        index,
        index / PRESETS_PER_BANK + 1,
        index % PRESETS_PER_BANK + 1,
        reason
    );
    loop {
        cortex_m::asm::wfi();
    }
}

fn warm_up(engine: &mut HardwareSynth<'_>, output: &mut [f32; BLOCK_LENGTH * 2]) {
    for _ in 0..WARMUP_BLOCKS {
        engine.process_interleaved(output, 2);
        black_box(output[0]);
    }
}

fn measure_raw(
    engine: &mut HardwareSynth<'_>,
    output: &mut [f32; BLOCK_LENGTH * 2],
    dma_output: &mut [(f32, f32); BLOCK_LENGTH],
    blocks: usize,
    controls: &ControlQueue,
    patches: &PatchQueue,
    adaptive_budget: &mut AdaptiveControlBudget,
) -> RawTiming {
    let mut timing = RawTiming::new();
    let mut transition = PatchTransition::default();
    for _ in 0..blocks {
        let started = DWT::cycle_count();
        run_callback(
            engine,
            output,
            dma_output,
            controls,
            patches,
            &mut transition,
            adaptive_budget,
        );
        timing.observe(DWT::cycle_count().wrapping_sub(started));
        black_box(dma_output[0]);
    }
    timing
}

fn measure_transition(
    engine: &mut HardwareSynth<'_>,
    patch: &synth_core::Patch,
    output: &mut [f32; BLOCK_LENGTH * 2],
    dma_output: &mut [(f32, f32); BLOCK_LENGTH],
    controls: &ControlQueue,
    patches: &PatchQueue,
    adaptive_budget: &mut AdaptiveControlBudget,
) -> RawTiming {
    let mut timing = RawTiming::new();
    let mut transition = PatchTransition::default();
    assert!(patches.try_send(patch.clone()).is_ok());
    let mut queued = true;
    while queued || !transition.is_idle() {
        let started = DWT::cycle_count();
        run_callback(
            engine,
            output,
            dma_output,
            controls,
            patches,
            &mut transition,
            adaptive_budget,
        );
        timing.observe(DWT::cycle_count().wrapping_sub(started));
        black_box(dma_output[0]);
        queued = false;
    }
    timing
}

fn measure_control_stress(
    engine: &mut HardwareSynth<'_>,
    patch: &synth_core::Patch,
    output: &mut [f32; BLOCK_LENGTH * 2],
    dma_output: &mut [(f32, f32); BLOCK_LENGTH],
    controls: &ControlQueue,
    patches: &PatchQueue,
    adaptive_budget: &mut AdaptiveControlBudget,
) -> RawTiming {
    let mut timing = RawTiming::new();
    let mut transition = PatchTransition::default();
    for index in 0..CONTROL_STRESS_BLOCKS {
        let direction = if index & 1 == 0 { 1.0 } else { -1.0 };
        for (param, value) in [
            (
                ParamId::FilterCutoff,
                patch.filter.cutoff * (1.0 + direction * 0.001),
            ),
            (
                ParamId::FilterResonance,
                patch.filter.resonance + direction * 0.001,
            ),
            (
                ParamId::Osc1ShapeMod,
                patch.osc1.shape_mod + direction * 0.001,
            ),
            (
                ParamId::EffectParam1,
                patch.effects.param1 + direction * 0.001,
            ),
        ] {
            assert!(controls
                .try_send(ControlMessage::SetParam(param, value))
                .is_ok());
        }
        let started = DWT::cycle_count();
        run_callback(
            engine,
            output,
            dma_output,
            controls,
            patches,
            &mut transition,
            adaptive_budget,
        );
        timing.observe(DWT::cycle_count().wrapping_sub(started));
        black_box(dma_output[0]);
    }
    timing
}

#[inline(always)]
fn run_callback(
    engine: &mut HardwareSynth<'_>,
    interleaved: &mut [f32; BLOCK_LENGTH * 2],
    dma_output: &mut [(f32, f32); BLOCK_LENGTH],
    controls: &ControlQueue,
    patches: &PatchQueue,
    transition: &mut PatchTransition,
    adaptive_budget: &mut AdaptiveControlBudget,
) {
    let work_started = DWT::cycle_count();
    if let Ok(patch) = patches.try_receive() {
        transition.enqueue(patch);
    }
    let action = transition.begin_block();
    if action.patch.is_some() {
        adaptive_budget.reset();
    }
    if let Some(patch) = action.patch {
        engine.apply_patch(&patch);
    }
    if let Ok(command) = controls.try_receive() {
        engine.handle_control(command);
    }
    let extras_started = DWT::cycle_count();
    let effective_budget = adaptive_budget.effective_budget();
    while DWT::cycle_count().wrapping_sub(extras_started) < effective_budget {
        let Ok(command) = controls.try_receive() else {
            break;
        };
        engine.handle_control(command);
    }
    let adaptive_spent = DWT::cycle_count().wrapping_sub(extras_started);
    if action.render {
        engine.process_interleaved(interleaved, 2);
    }
    transition.finish_block(interleaved, action.render);
    copy_output(interleaved, dma_output);
    let work_cycles = DWT::cycle_count().wrapping_sub(work_started);
    if action.render {
        adaptive_budget.observe_rendered_block(work_cycles, adaptive_spent, BLOCK_CYCLE_BUDGET);
    }
}

struct AdaptiveBudgetSummary {
    maximum: u32,
    bank: u8,
    program: u8,
    overruns: u32,
}

impl AdaptiveBudgetSummary {
    const fn new() -> Self {
        Self {
            maximum: 0,
            bank: 0,
            program: 0,
            overruns: 0,
        }
    }

    fn measure(
        &mut self,
        patch: &synth_core::Patch,
        effects_memory: &mut [f32],
        bank: u8,
        program: u8,
    ) {
        let mut engine =
            HardwareSynth::new_with_effects_memory(SAMPLE_RATE_HZ, &mut *effects_memory);
        engine.set_filter_type(FILTER_TYPE);
        engine.set_filter_oversampling(FILTER_OVERSAMPLING);
        engine.apply_patch(patch);

        let controls = ControlQueue::new();
        for note in [60, 64, 67, 72] {
            assert!(controls
                .try_send(ControlMessage::NoteOn {
                    note,
                    velocity: 1.0,
                })
                .is_ok());
        }
        for (param, value) in [
            (ParamId::FilterCutoff, patch.filter.cutoff),
            (ParamId::FilterResonance, patch.filter.resonance),
            (ParamId::Osc1ShapeMod, patch.osc1.shape_mod),
            (ParamId::EffectParam1, patch.effects.param1),
        ] {
            assert!(controls
                .try_send(ControlMessage::SetParam(param, value))
                .is_ok());
        }
        let patches = PatchQueue::new();
        let mut transition = PatchTransition::default();
        let mut adaptive_budget = AdaptiveControlBudget::new();
        let mut output = [0.0_f32; BLOCK_LENGTH * 2];
        let mut dma_output = [(0.0_f32, 0.0_f32); BLOCK_LENGTH];
        let mut maximum = 0_u32;
        for _ in 0..16 {
            let started = DWT::cycle_count();
            run_callback(
                &mut engine,
                &mut output,
                &mut dma_output,
                &controls,
                &patches,
                &mut transition,
                &mut adaptive_budget,
            );
            maximum = maximum.max(DWT::cycle_count().wrapping_sub(started));
            black_box(dma_output[0]);
        }

        self.overruns += u32::from(maximum > BLOCK_CYCLE_BUDGET);
        if maximum > self.maximum {
            self.maximum = maximum;
            self.bank = bank;
            self.program = program;
        }
    }

    fn report(&self) {
        defmt::info!(
            "FACTORY adaptive-budget max={} max_permille={} deadline_headroom={} overrun_presets={} worst_bank={} worst_program={}",
            self.maximum,
            budget_permille(self.maximum),
            BLOCK_CYCLE_BUDGET.saturating_sub(self.maximum),
            self.overruns,
            self.bank + 1,
            self.program + 1
        );
    }
}

#[inline(always)]
fn copy_output(interleaved: &[f32; BLOCK_LENGTH * 2], output: &mut [(f32, f32); BLOCK_LENGTH]) {
    for (frame, samples) in output.iter_mut().zip(interleaved.chunks_exact(2)) {
        *frame = (samples[0], samples[1]);
    }
}

fn measure_profiled(
    engine: &mut HardwareSynth<'_>,
    output: &mut [f32; BLOCK_LENGTH * 2],
) -> Snapshot {
    let mut profiler = AudioProfiler::new(BLOCK_CYCLE_BUDGET);
    for _ in 0..PROFILED_BLOCKS {
        profiler.begin_block();
        engine.process_interleaved_profiled(output, 2, &mut profiler);
        profiler.end_block();
        black_box(output[0]);
    }
    profiler.take_snapshot()
}

const RAW_HISTOGRAM_BINS: usize = 256;
const RAW_HISTOGRAM_RANGE: u32 = BLOCK_CYCLE_BUDGET * 2;

#[derive(Clone, Copy)]
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
        self.overruns += u32::from(cycles > BLOCK_CYCLE_BUDGET);
        let bin = ((u64::from(cycles.min(RAW_HISTOGRAM_RANGE)) * RAW_HISTOGRAM_BINS as u64)
            / u64::from(RAW_HISTOGRAM_RANGE))
        .min((RAW_HISTOGRAM_BINS - 1) as u64) as usize;
        self.histogram[bin] = self.histogram[bin].saturating_add(1);
    }

    fn average(self) -> u32 {
        (self.total / u64::from(self.blocks.max(1))) as u32
    }

    fn quantile(self, percentile: u32) -> u32 {
        let target = (u64::from(self.blocks) * u64::from(percentile) + 99) / 100;
        let mut cumulative = 0_u64;
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

const SCENARIO_COUNT: usize = 4;

#[derive(Clone, Copy)]
enum Scenario {
    PatchTransition,
    Attack,
    Steady,
    ControlStress,
}

impl Scenario {
    const ALL: [Self; SCENARIO_COUNT] = [
        Self::PatchTransition,
        Self::Attack,
        Self::Steady,
        Self::ControlStress,
    ];

    const fn index(self) -> usize {
        self as usize
    }

    const fn name(self) -> &'static str {
        match self {
            Self::PatchTransition => "transition",
            Self::Attack => "attack",
            Self::Steady => "steady",
            Self::ControlStress => "controls",
        }
    }
}

fn report_raw(bank: u8, program: u8, scenario: Scenario, raw: RawTiming) {
    defmt::info!(
        "FACTORY raw bank={} program={} scenario={=str} avg={} p95={} p99={} max={} max_permille={} target_headroom={} deadline_headroom={} overruns={}",
        bank + 1,
        program + 1,
        scenario.name(),
        raw.average(),
        raw.quantile(95),
        raw.quantile(99),
        raw.maximum,
        budget_permille(raw.maximum),
        PROFILE_THRESHOLD_CYCLES.saturating_sub(raw.maximum),
        BLOCK_CYCLE_BUDGET.saturating_sub(raw.maximum),
        raw.overruns
    );
}

fn report_profile(bank: u8, program: u8, snapshot: Snapshot) {
    let average = snapshot.stage_average;
    let worst = snapshot.stage_worst_block;
    defmt::info!(
        "FACTORY profile bank={} program={} avg={} max={} overruns={} env_mod={} osc={} filter={} amp_pan={} effects={} output={}",
        bank + 1,
        program + 1,
        snapshot.block_average,
        snapshot.block_max,
        snapshot.overruns,
        average[RenderStage::EnvelopesAndModulation.index()],
        average[RenderStage::Oscillators.index()],
        average[RenderStage::Filter.index()],
        average[RenderStage::AmplifierAndPan.index()],
        average[RenderStage::Effects.index()],
        average[RenderStage::MasterOutput.index()]
    );
    defmt::info!(
        "FACTORY profile-detail bank={} program={} envelopes={} lfo_control={} lfo_generation={} audio_routes={} osc_control={} waveform={} osc_mix={} fx_prepare={} combs={} allpasses={} fx_mix={}",
        bank + 1,
        program + 1,
        average[RenderStage::EnvelopeAdvance.index()],
        average[RenderStage::LfoControlRouting.index()],
        average[RenderStage::LfoGeneration.index()],
        average[RenderStage::AudioModulationRouting.index()],
        average[RenderStage::OscillatorControl.index()],
        average[RenderStage::OscillatorWaveform.index()],
        average[RenderStage::OscillatorMix.index()],
        average[RenderStage::EffectsPreparation.index()],
        average[RenderStage::ReverbCombs.index()],
        average[RenderStage::ReverbAllpasses.index()],
        average[RenderStage::EffectsMix.index()]
    );
    defmt::info!(
        "FACTORY profile-worst bank={} program={} env_mod={} osc={} filter={} amp_pan={} effects={} output={}",
        bank + 1,
        program + 1,
        worst[RenderStage::EnvelopesAndModulation.index()],
        worst[RenderStage::Oscillators.index()],
        worst[RenderStage::Filter.index()],
        worst[RenderStage::AmplifierAndPan.index()],
        worst[RenderStage::Effects.index()],
        worst[RenderStage::MasterOutput.index()]
    );
}

const fn budget_permille(cycles: u32) -> u32 {
    (cycles as u64 * 1_000 / BLOCK_CYCLE_BUDGET as u64) as u32
}

struct Summary {
    at_or_above_target: [u16; SCENARIO_COUNT],
    over_deadline: [u16; SCENARIO_COUNT],
    worst_bank: u8,
    worst_program: u8,
    worst_scenario: Scenario,
    worst_cycles: u32,
    slowest: [PresetPeak; 16],
    waveform_groups: [FeatureGroup; 4],
    dual_oscillator_groups: [FeatureGroup; 2],
    pole_groups: [FeatureGroup; 2],
    effect_groups: [FeatureGroup; 14],
    route_groups: [FeatureGroup; 19],
}

#[derive(Clone, Copy)]
struct FeatureGroup {
    presets: u16,
    at_or_above_target: u16,
    over_deadline: u16,
    maximum: u32,
}

impl FeatureGroup {
    const EMPTY: Self = Self {
        presets: 0,
        at_or_above_target: 0,
        over_deadline: 0,
        maximum: 0,
    };

    fn observe(&mut self, raw: RawTiming) {
        self.presets += 1;
        self.at_or_above_target += u16::from(raw.maximum >= PROFILE_THRESHOLD_CYCLES);
        self.over_deadline += u16::from(raw.maximum > BLOCK_CYCLE_BUDGET);
        self.maximum = self.maximum.max(raw.maximum);
    }
}

#[derive(Clone, Copy)]
struct PresetPeak {
    bank: u8,
    program: u8,
    scenario: Scenario,
    cycles: u32,
}

impl PresetPeak {
    const EMPTY: Self = Self {
        bank: 0,
        program: 0,
        scenario: Scenario::Steady,
        cycles: 0,
    };
}

impl Summary {
    const fn new() -> Self {
        Self {
            at_or_above_target: [0; SCENARIO_COUNT],
            over_deadline: [0; SCENARIO_COUNT],
            worst_bank: 0,
            worst_program: 0,
            worst_scenario: Scenario::Steady,
            worst_cycles: 0,
            slowest: [PresetPeak::EMPTY; 16],
            waveform_groups: [FeatureGroup::EMPTY; 4],
            dual_oscillator_groups: [FeatureGroup::EMPTY; 2],
            pole_groups: [FeatureGroup::EMPTY; 2],
            effect_groups: [FeatureGroup::EMPTY; 14],
            route_groups: [FeatureGroup::EMPTY; 19],
        }
    }

    fn observe(&mut self, bank: u8, program: u8, scenario: Scenario, raw: RawTiming) {
        let scenario_index = scenario.index();
        self.at_or_above_target[scenario_index] +=
            u16::from(raw.maximum >= PROFILE_THRESHOLD_CYCLES);
        self.over_deadline[scenario_index] += u16::from(raw.maximum > BLOCK_CYCLE_BUDGET);
        if raw.maximum > self.worst_cycles {
            self.worst_bank = bank;
            self.worst_program = program;
            self.worst_scenario = scenario;
            self.worst_cycles = raw.maximum;
        }
        let Some(position) = self
            .slowest
            .iter()
            .position(|entry| raw.maximum > entry.cycles)
        else {
            return;
        };
        for index in (position + 1..self.slowest.len()).rev() {
            self.slowest[index] = self.slowest[index - 1];
        }
        self.slowest[position] = PresetPeak {
            bank,
            program,
            scenario,
            cycles: raw.maximum,
        };
    }

    fn observe_features(&mut self, patch: &synth_core::Patch, raw: RawTiming) {
        self.waveform_groups[usize::from(patch.osc1.waveform.min(3))].observe(raw);
        self.dual_oscillator_groups[usize::from(patch.osc2.enabled)].observe(raw);
        self.pole_groups[usize::from(patch.filter.poles > 2)].observe(raw);
        let effect_index = if patch.effects.enabled {
            patch.effects.effect_type.index()
        } else {
            13
        };
        self.effect_groups[effect_index].observe(raw);
        let direct_routes = patch
            .lfos
            .iter()
            .filter(|lfo| lfo.destination != ModDestination::Off)
            .count();
        let aux_routes = usize::from(patch.aux_envelope.destination != ModDestination::Off);
        let free_routes = patch
            .mod_matrix
            .free_slots
            .iter()
            .filter(|slot| slot.enabled)
            .count();
        let dedicated_routes = patch
            .mod_matrix
            .dedicated
            .iter()
            .filter(|slot| slot.enabled)
            .count();
        let route_count = (direct_routes + aux_routes + free_routes + dedicated_routes).min(18);
        self.route_groups[route_count].observe(raw);
    }

    fn report(&self) {
        for scenario in Scenario::ALL {
            defmt::info!(
                "FACTORY scenario-summary scenario={=str} presets={} at_or_above_272k={} over_deadline={}",
                scenario.name(),
                FACTORY_PRESET_COUNT,
                self.at_or_above_target[scenario.index()],
                self.over_deadline[scenario.index()]
            );
        }
        report_feature_groups("osc1_waveform", &self.waveform_groups);
        report_feature_groups("dual_oscillator", &self.dual_oscillator_groups);
        report_feature_groups("filter_poles_4", &self.pole_groups);
        report_feature_groups("effect", &self.effect_groups);
        report_feature_groups("routes", &self.route_groups);
        defmt::info!(
            "FACTORY summary presets={} cases={} worst_bank={} worst_program={} worst_scenario={=str} worst_cycles={} worst_permille={}",
            FACTORY_PRESET_COUNT,
            FACTORY_PRESET_COUNT * SCENARIO_COUNT,
            self.worst_bank + 1,
            self.worst_program + 1,
            self.worst_scenario.name(),
            self.worst_cycles,
            budget_permille(self.worst_cycles)
        );
        for (index, preset) in self.slowest.iter().enumerate() {
            defmt::info!(
                "FACTORY slowest rank={} bank={} program={} scenario={=str} max={} max_permille={}",
                index + 1,
                preset.bank + 1,
                preset.program + 1,
                preset.scenario.name(),
                preset.cycles,
                budget_permille(preset.cycles)
            );
        }
    }
}

fn report_feature_groups(kind: &str, groups: &[FeatureGroup]) {
    for (value, group) in groups.iter().enumerate() {
        if group.presets == 0 {
            continue;
        }
        defmt::info!(
            "FACTORY feature kind={=str} value={} presets={} at_or_above_272k={} over_deadline={} max={}",
            kind,
            value,
            group.presets,
            group.at_or_above_target,
            group.over_deadline,
            group.maximum
        );
    }
}

struct Crc32(u32);

impl Crc32 {
    const fn new() -> Self {
        Self(u32::MAX)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u32::from(*byte);
            for _ in 0..8 {
                self.0 = (self.0 >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(self.0 & 1));
            }
        }
    }

    const fn finish(self) -> u32 {
        !self.0
    }
}
