//! Compact dynamic characterization for the measured-wavetable candidate.

use std::fs::{self, File};
use std::hint::black_box;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use synth_core::dsp::{FilterType, Waveform, WavetableBank};
use synth_core::{
    BankId, GlideMode, MONOLOGUE_WAVETABLE_BANK_PROFILE, OscillatorEngineType,
    OscillatorResearchModel, ParamId, ResearchEvent, ResearchModelId, ResearchRegistry,
    ResearchRenderCase, SynthEngineWithMemory,
};

const SOURCE_SECONDS: f32 = 2.0;
const SOURCE_REPEATS: usize = 3;
const ENGINE_BLOCK_FRAMES: usize = 64;
const ENGINE_BLOCKS: usize = 1_000;

#[derive(Clone, Copy)]
enum Scenario {
    PitchShape,
    PwmAudio,
    HardSync,
    Combined,
}

impl Scenario {
    const ALL: [Self; 4] = [
        Self::PitchShape,
        Self::PwmAudio,
        Self::HardSync,
        Self::Combined,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::PitchShape => "pitch-shape-sweep",
            Self::PwmAudio => "audio-rate-pwm",
            Self::HardSync => "hard-sync-ratios",
            Self::Combined => "combined-pitch-pwm-sync",
        }
    }

    fn waveform(self) -> Waveform {
        match self {
            Self::PitchShape => Waveform::Triangle,
            Self::PwmAudio | Self::Combined => Waveform::Pulse,
            Self::HardSync => Waveform::Saw,
        }
    }

    fn initial_frequency(self) -> f32 {
        match self {
            Self::PitchShape => 55.0,
            Self::PwmAudio => 220.0,
            Self::HardSync => 880.0,
            Self::Combined => 110.0,
        }
    }
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    source_duration_seconds: f32,
    source_repeats: usize,
    source_cases: Vec<SourceResult>,
    engine_block_frames: usize,
    engine_blocks: usize,
    engine_cases: Vec<EngineResult>,
}

#[derive(Serialize)]
struct SourceResult {
    model_id: &'static str,
    scenario: &'static str,
    sample_rate_hz: f32,
    samples: usize,
    deterministic: bool,
    sample_hash_fnv1a64: String,
    elapsed_nanoseconds_median: u128,
    nanoseconds_per_sample_median: f64,
    rms: f64,
    peak: f32,
    maximum_adjacent_step: f32,
    wav: String,
}

#[derive(Serialize)]
struct EngineResult {
    model_id: &'static str,
    profile: &'static str,
    active_voices: usize,
    block_count: usize,
    block_frames: usize,
    nanoseconds_per_frame_median: f64,
    nanoseconds_per_frame_p95: f64,
    nanoseconds_per_frame_p99: f64,
    nanoseconds_per_frame_maximum: f64,
    realtime_budget_fraction_p99: f64,
    sample_hash_fnv1a64: String,
    finite: bool,
    peak: f32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_root = parse_output_root()?;
    fs::create_dir_all(&output_root)?;
    let bank = wavetable_bank()?;
    let mut source_cases = Vec::new();
    for model in [
        ResearchModelId::Baseline,
        ResearchModelId::WavetableMonologue,
    ] {
        for scenario in Scenario::ALL {
            for sample_rate_hz in [48_000.0, 192_000.0] {
                source_cases.push(characterize_source(
                    model,
                    scenario,
                    sample_rate_hz,
                    bank,
                    &output_root,
                )?);
            }
        }
    }

    let mut engine_cases = Vec::new();
    for (model_id, engine_type) in [
        ("baseline-v1", OscillatorEngineType::Blep),
        (
            "korg-monologue-measured-wavetable-v1",
            OscillatorEngineType::Wavetable,
        ),
    ] {
        for profile in [
            "steady-one",
            "steady-four",
            "steady-four-slop",
            "combined-one",
        ] {
            engine_cases.push(characterize_engine(model_id, engine_type, profile));
        }
    }

    let report = Report {
        schema_version: 1,
        source_duration_seconds: SOURCE_SECONDS,
        source_repeats: SOURCE_REPEATS,
        source_cases,
        engine_block_frames: ENGINE_BLOCK_FRAMES,
        engine_blocks: ENGINE_BLOCKS,
        engine_cases,
    };
    let report_path = output_root.join("runtime.json");
    let mut writer = BufWriter::new(File::create(&report_path)?);
    serde_json::to_writer_pretty(&mut writer, &report)?;
    writer.write_all(b"\n")?;
    println!("wrote {}", report_path.display());
    Ok(())
}

fn characterize_source(
    model_id: ResearchModelId,
    scenario: Scenario,
    sample_rate_hz: f32,
    bank: WavetableBank,
    output_root: &Path,
) -> Result<SourceResult, Box<dyn std::error::Error>> {
    let sample_count = (SOURCE_SECONDS * sample_rate_hz) as usize;
    let mut elapsed = Vec::with_capacity(SOURCE_REPEATS);
    let mut hashes = Vec::with_capacity(SOURCE_REPEATS);
    let mut retained = Vec::new();
    for repeat in 0..SOURCE_REPEATS {
        let mut model = if model_id == ResearchModelId::WavetableMonologue {
            ResearchRegistry::create_wavetable(sample_rate_hz, bank)
        } else {
            ResearchRegistry::create(model_id, sample_rate_hz, None)
        }
        .map_err(|error| format!("create {}: {error:?}", model_id.as_str()))?;
        let case = ResearchRenderCase {
            waveform: scenario.waveform(),
            sample_rate_hz,
            frequency_hz: scenario.initial_frequency(),
            shape: 0.35,
            warmup_samples: 0,
            render_samples: sample_count,
            seed: 0x4d57_4459,
            reset_phase: true,
        };
        model
            .configure(case)
            .map_err(|error| format!("configure {}: {error:?}", model_id.as_str()))?;
        let warmup_samples = (4_096.0 * sample_rate_hz / 48_000.0) as usize;
        for _ in 0..warmup_samples {
            black_box(model.next_sample());
        }
        let started = Instant::now();
        let samples = render_dynamic(&mut model, scenario, sample_rate_hz, sample_count)?;
        elapsed.push(started.elapsed().as_nanos());
        hashes.push(hash_samples(&samples));
        if repeat + 1 == SOURCE_REPEATS {
            retained = samples;
        }
    }
    elapsed.sort_unstable();
    let elapsed_median = elapsed[elapsed.len() / 2];
    let deterministic = hashes.windows(2).all(|pair| pair[0] == pair[1]);
    let (rms, peak, maximum_adjacent_step) = signal_metrics(&retained);
    let wav_name = format!(
        "{}-{}-{}khz.wav",
        model_id.as_str(),
        scenario.id(),
        (sample_rate_hz / 1_000.0) as u32
    );
    let wav_path = output_root.join(&wav_name);
    write_float_wav(&wav_path, sample_rate_hz as u32, &retained)?;
    Ok(SourceResult {
        model_id: model_id.as_str(),
        scenario: scenario.id(),
        sample_rate_hz,
        samples: sample_count,
        deterministic,
        sample_hash_fnv1a64: format!("{:016x}", hashes[0]),
        elapsed_nanoseconds_median: elapsed_median,
        nanoseconds_per_sample_median: elapsed_median as f64 / sample_count as f64,
        rms,
        peak,
        maximum_adjacent_step,
        wav: wav_name,
    })
}

fn render_dynamic(
    model: &mut impl OscillatorResearchModel,
    scenario: Scenario,
    sample_rate_hz: f32,
    sample_count: usize,
) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let mut output = Vec::with_capacity(sample_count);
    let mut master_phase = 0.0_f32;
    for index in 0..sample_count {
        let time = index as f32 / sample_rate_hz;
        let progress = index as f32 / sample_count.saturating_sub(1).max(1) as f32;
        match scenario {
            Scenario::PitchShape => {
                let frequency = 55.0 * (1_200.0_f32 / 55.0).powf(progress);
                let shape = 0.5 + 0.45 * (std::f32::consts::TAU * 5.0 * time).sin();
                apply_event(model, ResearchEvent::SetFrequency(frequency))?;
                apply_event(model, ResearchEvent::SetShape(shape))?;
            }
            Scenario::PwmAudio => {
                let shape = 0.475 + 0.475 * (std::f32::consts::TAU * 110.0 * time).sin();
                apply_event(model, ResearchEvent::SetShape(shape))?;
            }
            Scenario::HardSync => {
                let master = if progress < 1.0 / 3.0 {
                    220.0
                } else if progress < 2.0 / 3.0 {
                    880.0 / 3.0
                } else {
                    352.0
                };
                apply_master_wrap(model, &mut master_phase, master, sample_rate_hz)?;
            }
            Scenario::Combined => {
                let frequency = 110.0 * 8.0_f32.powf(progress);
                let shape = 0.475 + 0.475 * (std::f32::consts::TAU * 37.0 * time).sin();
                apply_event(model, ResearchEvent::SetFrequency(frequency))?;
                apply_event(model, ResearchEvent::SetShape(shape))?;
                let master = 146.666_67 + 73.333_33 * progress;
                apply_master_wrap(model, &mut master_phase, master, sample_rate_hz)?;
            }
        }
        output.push(model.next_sample());
    }
    Ok(output)
}

fn apply_master_wrap(
    model: &mut impl OscillatorResearchModel,
    phase: &mut f32,
    frequency_hz: f32,
    sample_rate_hz: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let increment = frequency_hz / sample_rate_hz;
    let next = *phase + increment;
    if next >= 1.0 {
        let offset = ((1.0 - *phase) / increment).clamp(0.0, 1.0);
        apply_event(
            model,
            ResearchEvent::HardSync {
                subsample_offset: offset,
            },
        )?;
    }
    *phase = next - next.floor();
    Ok(())
}

fn apply_event(
    model: &mut impl OscillatorResearchModel,
    event: ResearchEvent,
) -> Result<(), Box<dyn std::error::Error>> {
    model
        .apply_event(event)
        .map_err(|error| format!("dynamic event failed: {error:?}").into())
}

fn characterize_engine(
    model_id: &'static str,
    engine_type: OscillatorEngineType,
    profile: &'static str,
) -> EngineResult {
    let sample_rate_hz = 48_000.0;
    let effects = vec![0.0; 96_000].into_boxed_slice();
    let mut engine =
        SynthEngineWithMemory::<_, 4>::new_with_effects_memory(sample_rate_hz, effects)
            .expect("valid effects memory layout");
    engine.set_wavetable_bank(BankId::Monologue);
    engine.set_oscillator_engine(engine_type);
    configure_engine(&mut engine, profile);
    let notes: &[u8] = if profile.starts_with("steady-four") {
        &[48, 55, 60, 64]
    } else {
        &[57]
    };
    for note in notes {
        engine.note_on(*note, 1.0);
    }
    let mut block = [0.0_f32; ENGINE_BLOCK_FRAMES];
    for _ in 0..64 {
        engine.process_interleaved(&mut block, 1);
    }

    let mut times = Vec::with_capacity(ENGINE_BLOCKS);
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut peak = 0.0_f32;
    let mut finite = true;
    let dynamic_notes = [57, 69, 50, 74];
    let mut current_note = dynamic_notes[0];
    for block_index in 0..ENGINE_BLOCKS {
        if profile == "combined-one" {
            let time = block_index as f32 * ENGINE_BLOCK_FRAMES as f32 / sample_rate_hz;
            let shape = 0.475 + 0.475 * (std::f32::consts::TAU * 3.7 * time).sin();
            engine.set_param(ParamId::Osc1ShapeMod, shape);
            if block_index > 0 && block_index % 200 == 0 {
                engine.note_off(current_note);
                current_note = dynamic_notes[(block_index / 200) % dynamic_notes.len()];
                engine.note_on(current_note, 1.0);
            }
        }
        let started = Instant::now();
        engine.process_interleaved(&mut block, 1);
        times.push(started.elapsed().as_nanos() as f64 / ENGINE_BLOCK_FRAMES as f64);
        for sample in block {
            finite &= sample.is_finite();
            peak = peak.max(sample.abs());
            for byte in sample.to_bits().to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    times.sort_by(f64::total_cmp);
    let percentile = |amount: f64| times[((times.len() - 1) as f64 * amount).round() as usize];
    let p99 = percentile(0.99);
    EngineResult {
        model_id,
        profile,
        active_voices: notes.len(),
        block_count: ENGINE_BLOCKS,
        block_frames: ENGINE_BLOCK_FRAMES,
        nanoseconds_per_frame_median: percentile(0.5),
        nanoseconds_per_frame_p95: percentile(0.95),
        nanoseconds_per_frame_p99: p99,
        nanoseconds_per_frame_maximum: *times.last().expect("non-empty timings"),
        realtime_budget_fraction_p99: p99 / (1_000_000_000.0 / sample_rate_hz as f64),
        sample_hash_fnv1a64: format!("{hash:016x}"),
        finite,
        peak,
    }
}

fn configure_engine(engine: &mut SynthEngineWithMemory<Box<[f32]>, 4>, profile: &str) {
    engine.set_filter_type(FilterType::PassThrough);
    engine.set_param(ParamId::AmpEgAttack, 0.0);
    engine.set_param(ParamId::AmpEgDecay, 0.0);
    engine.set_param(ParamId::AmpEgSustain, 1.0);
    engine.set_param(ParamId::AmpEgRelease, 0.0);
    engine.set_param(ParamId::Osc1Enabled, 1.0);
    engine.set_param(ParamId::Osc2Enabled, f32::from(profile == "combined-one"));
    engine.set_param(ParamId::OscMix, 0.0);
    engine.set_param(ParamId::Osc1Waveform, Waveform::Pulse.index() as f32);
    engine.set_param(ParamId::Osc2Waveform, Waveform::Saw.index() as f32);
    engine.set_param(ParamId::Osc1ShapeMod, 0.35);
    if profile == "steady-four-slop" {
        engine.set_param(ParamId::OscSlop, 1.0);
    }
    if profile == "combined-one" {
        engine.set_param(ParamId::Osc1FineTune, 17.0);
        engine.set_param(ParamId::Osc2FineTune, -13.0);
        engine.set_param(ParamId::Osc2Frequency, 48.0);
        engine.set_param(ParamId::HardSync, 1.0);
        engine.set_param(ParamId::OscSlop, 1.0);
        engine.set_param(ParamId::Osc1Glide, 0.35);
        engine.set_param(ParamId::Osc2Glide, 0.35);
        engine.set_param(ParamId::GlideMode, GlideMode::FixedTime.index() as f32);
        engine.set_param(ParamId::GlideEnabled, 1.0);
    }
}

fn signal_metrics(samples: &[f32]) -> (f64, f32, f32) {
    let mut squared = 0.0_f64;
    let mut peak = 0.0_f32;
    let mut maximum_step = 0.0_f32;
    for (index, sample) in samples.iter().copied().enumerate() {
        squared += f64::from(sample) * f64::from(sample);
        peak = peak.max(sample.abs());
        if index > 0 {
            maximum_step = maximum_step.max((sample - samples[index - 1]).abs());
        }
    }
    ((squared / samples.len() as f64).sqrt(), peak, maximum_step)
}

fn hash_samples(samples: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

fn write_float_wav(path: &Path, sample_rate_hz: u32, samples: &[f32]) -> std::io::Result<()> {
    let data_bytes = (samples.len() * size_of::<f32>()) as u32;
    let mut writer = BufWriter::new(File::create(path)?);
    writer.write_all(b"RIFF")?;
    writer.write_all(&(36 + data_bytes).to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16_u32.to_le_bytes())?;
    writer.write_all(&3_u16.to_le_bytes())?;
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&sample_rate_hz.to_le_bytes())?;
    writer.write_all(&(sample_rate_hz * 4).to_le_bytes())?;
    writer.write_all(&4_u16.to_le_bytes())?;
    writer.write_all(&32_u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_bytes.to_le_bytes())?;
    for sample in samples {
        writer.write_all(&sample.to_le_bytes())?;
    }
    writer.flush()
}

fn wavetable_bank() -> Result<WavetableBank, Box<dyn std::error::Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/analog-osc/banks/korg-monologue-measured-wavetable-v2.f32le");
    let bytes = fs::read(&path)?;
    if bytes.len() % size_of::<f32>() != 0 {
        return Err(format!("{} contains a partial f32", path.display()).into());
    }
    let samples = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    WavetableBank::new(Box::leak(samples), &MONOLOGUE_WAVETABLE_BANK_PROFILE)
        .map_err(|error| format!("invalid measured bank: {error:?}").into())
}

fn parse_output_root() -> Result<PathBuf, String> {
    let mut arguments = std::env::args().skip(1);
    let mut output = PathBuf::from("target/analog-osc/dynamic-characterization-v1");
    while let Some(argument) = arguments.next() {
        if argument == "--output-root" {
            output = PathBuf::from(
                arguments
                    .next()
                    .ok_or_else(|| "--output-root requires a path".to_owned())?,
            );
        } else {
            return Err(format!("unknown argument {argument:?}"));
        }
    }
    Ok(output)
}
