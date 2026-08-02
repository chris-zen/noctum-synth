//! Deterministic oscillator research renderer and artifact writer.

use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Instant;

use rustfft::{FftPlanner, num_complex::Complex32};
use serde::Serialize;
use synth_core::dsp::{
    MONOLOGUE_WAVETABLE_BANK_PROFILE, MipWavetableBank, PROPHET5_WAVETABLE_BANK_PROFILE,
    WAVETABLE_BANK_SAMPLES, Waveform, WavetableBank, WavetableProfile, generate_wavetable_bank,
};
use synth_core::{
    OscillatorResearchModel, ResearchComparisonMetrics, ResearchModelFamily, ResearchModelId,
    ResearchRegistry, ResearchRenderCase, ResearchRenderSummary, render_research_case,
};

const ARTIFACT_SCHEMA_VERSION: u32 = 1;
const METRIC_REVISION: u32 = 1;

#[derive(Debug)]
struct Options {
    model: ResearchModelId,
    reference_model: Option<ResearchModelId>,
    waveform: Waveform,
    sample_rate_hz: f32,
    frequency_hz: f32,
    shape: f32,
    warmup_samples: usize,
    render_samples: usize,
    seed: u64,
    output_root: PathBuf,
    parameters: Vec<(String, f32)>,
    list: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            model: ResearchModelId::Baseline,
            reference_model: None,
            waveform: Waveform::Saw,
            sample_rate_hz: 48_000.0,
            frequency_hz: 220.0,
            shape: 0.5,
            warmup_samples: 4_096,
            render_samples: 65_536,
            seed: 0,
            output_root: PathBuf::from("target/analog-osc"),
            parameters: Vec::new(),
            list: false,
        }
    }
}

#[derive(Serialize)]
struct Artifact<'a> {
    schema_version: u32,
    metric_revision: u32,
    git_commit: &'a str,
    dirty_worktree: bool,
    host_os: &'static str,
    host_arch: &'static str,
    build_profile: &'static str,
    model: ModelArtifact<'a>,
    case: CaseArtifact<'a>,
    render: RenderArtifact,
    spectrum: SpectrumArtifact,
    comparison: Option<ComparisonArtifact<'a>>,
    files: FilesArtifact<'a>,
}

#[derive(Serialize)]
struct ModelArtifact<'a> {
    id: &'a str,
    name: &'a str,
    revision: u32,
    family: &'a str,
    real_time_safe: bool,
    bounded_render_cost: bool,
    no_std_compatible: bool,
    mutable_state_bytes: usize,
    immutable_asset_bytes: usize,
    latency_samples: u32,
    profile_id: Option<&'a str>,
    profile_content_sha256: Option<&'a str>,
    model_parameters: &'a [ModelParameterArtifact],
}

#[derive(Serialize)]
struct ModelParameterArtifact {
    id: String,
    value: f32,
}

#[derive(Serialize)]
struct CaseArtifact<'a> {
    id: &'a str,
    waveform: &'a str,
    sample_rate_hz: f32,
    frequency_hz: f32,
    shape: f32,
    pulse_width_percent: Option<f32>,
    warmup_samples: usize,
    render_samples: usize,
    seed: u64,
    reset_phase: bool,
    normalization: &'a str,
    target_id: Option<&'a str>,
    target_manifest_sha256: Option<&'a str>,
}

#[derive(Serialize)]
struct RenderArtifact {
    case_elapsed_nanoseconds: u128,
    nanoseconds_per_processed_sample: f64,
    sample_hash_fnv1a64: String,
    dc: f64,
    rms: f64,
    peak: f32,
    crest_factor: f64,
    duty_above_midpoint: f64,
    measured_frequency_hz: Option<f64>,
    pitch_error_cents: Option<f64>,
}

#[derive(Serialize)]
struct SpectrumArtifact {
    fft_size: usize,
    window: &'static str,
    fundamental_dbfs: f64,
    residual_dbc: f64,
    worst_residual_component_dbc: f64,
    note: &'static str,
}

#[derive(Serialize)]
struct ComparisonArtifact<'a> {
    reference_model_id: &'a str,
    normalized_rms_error: f64,
    maximum_absolute_error: f32,
    correlation: f64,
}

#[derive(Serialize)]
struct FilesArtifact<'a> {
    source_wav: &'a str,
}

#[derive(Clone, Copy)]
struct SpectrumMetrics {
    fft_size: usize,
    fundamental_dbfs: f64,
    residual_dbc: f64,
    worst_residual_component_dbc: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = parse_options()?;
    if options.list {
        for descriptor in ResearchRegistry::descriptors() {
            println!(
                "{}\t{}\t{:?}\treal_time_safe={}\trequires_asset={}",
                descriptor.id,
                descriptor.name,
                descriptor.family,
                descriptor.capabilities.real_time_safe,
                descriptor.requires_external_asset,
            );
        }
        return Ok(());
    }

    let case = ResearchRenderCase {
        waveform: options.waveform,
        sample_rate_hz: options.sample_rate_hz,
        frequency_hz: options.frequency_hz,
        shape: options.shape,
        warmup_samples: options.warmup_samples,
        render_samples: options.render_samples,
        seed: options.seed,
        reset_phase: true,
    };
    let (summary, samples, elapsed, parameter_values) =
        render(options.model, case, &options.parameters)?;
    let spectrum = spectrum_metrics(&samples, case.sample_rate_hz, case.frequency_hz);
    let comparison = if let Some(reference_model) = options.reference_model {
        let (_, reference, _, _) = render(reference_model, case, &[])?;
        Some((
            reference_model,
            ResearchComparisonMetrics::measure(&reference, &samples)
                .map_err(|error| format!("comparison failed: {error:?}"))?,
        ))
    } else {
        None
    };

    let case_id = case_id(case);
    let render_directory = options
        .output_root
        .join("renders")
        .join(summary.descriptor.id);
    let metric_directory = options
        .output_root
        .join("metrics")
        .join(summary.descriptor.id);
    fs::create_dir_all(&render_directory)?;
    fs::create_dir_all(&metric_directory)?;
    let wav_path = render_directory.join(format!("{case_id}.wav"));
    let json_path = metric_directory.join(format!("{case_id}.json"));
    write_float_wav(&wav_path, case.sample_rate_hz as u32, &samples)?;

    let (git_commit, dirty_worktree) = git_state();
    let wav_string = wav_path.to_string_lossy();
    let artifact = artifact(
        &summary,
        &case_id,
        elapsed,
        spectrum,
        comparison,
        &git_commit,
        dirty_worktree,
        &wav_string,
        &parameter_values,
    );
    let mut output = BufWriter::new(File::create(&json_path)?);
    serde_json::to_writer_pretty(&mut output, &artifact)?;
    output.write_all(b"\n")?;
    output.flush()?;

    println!("wrote {}", wav_path.display());
    println!("wrote {}", json_path.display());
    Ok(())
}

fn render(
    id: ResearchModelId,
    case: ResearchRenderCase,
    parameters: &[(String, f32)],
) -> Result<
    (
        ResearchRenderSummary,
        Vec<f32>,
        u128,
        Vec<ModelParameterArtifact>,
    ),
    Box<dyn std::error::Error>,
> {
    let mut model = if matches!(
        id,
        ResearchModelId::WavetableMonologue | ResearchModelId::WavetableProphet5
    ) {
        ResearchRegistry::create_wavetable(case.sample_rate_hz, wavetable_bank(id)?)
    } else {
        let bank = (id == ResearchModelId::Wavetable).then(reference_wavetable_bank);
        ResearchRegistry::create(id, case.sample_rate_hz, bank)
    }
    .map_err(|error| format!("could not construct {}: {error:?}", id.as_str()))?;
    for (parameter, value) in parameters {
        model.set_parameter(parameter, *value).map_err(|error| {
            format!(
                "could not set parameter {parameter:?} on {}: {error:?}",
                id.as_str()
            )
        })?;
    }
    let mut samples = vec![0.0; case.render_samples];
    let started = Instant::now();
    let summary = render_research_case(&mut model, case, &mut samples)
        .map_err(|error| format!("render failed for {}: {error:?}", id.as_str()))?;
    let parameter_values = model
        .parameter_descriptors()
        .iter()
        .map(|descriptor| ModelParameterArtifact {
            id: descriptor.id.to_owned(),
            value: model
                .parameter_value(descriptor.id)
                .expect("every declared parameter must report its value"),
        })
        .collect();
    Ok((
        summary,
        samples,
        started.elapsed().as_nanos(),
        parameter_values,
    ))
}

fn artifact<'a>(
    summary: &'a ResearchRenderSummary,
    case_id: &'a str,
    elapsed_nanoseconds: u128,
    spectrum: SpectrumMetrics,
    comparison: Option<(ResearchModelId, ResearchComparisonMetrics)>,
    git_commit: &'a str,
    dirty_worktree: bool,
    wav_path: &'a str,
    parameter_values: &'a [ModelParameterArtifact],
) -> Artifact<'a> {
    let descriptor = summary.descriptor;
    let target_profile =
        ResearchModelId::parse(descriptor.id).and_then(ResearchRegistry::target_profile_metadata);
    let measured_frequency = summary.signal.measured_frequency_hz;
    Artifact {
        schema_version: ARTIFACT_SCHEMA_VERSION,
        metric_revision: METRIC_REVISION,
        git_commit,
        dirty_worktree,
        host_os: env::consts::OS,
        host_arch: env::consts::ARCH,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        model: ModelArtifact {
            id: descriptor.id,
            name: descriptor.name,
            revision: descriptor.revision,
            family: match descriptor.family {
                ResearchModelFamily::PhaseKernel => "phase_kernel",
                ResearchModelFamily::Stateful => "stateful",
            },
            real_time_safe: descriptor.capabilities.real_time_safe,
            bounded_render_cost: descriptor.bounded_render_cost,
            no_std_compatible: descriptor.no_std_compatible,
            mutable_state_bytes: descriptor.mutable_state_bytes,
            immutable_asset_bytes: descriptor.immutable_asset_bytes,
            latency_samples: descriptor.latency_samples,
            profile_id: target_profile.map(|metadata| metadata.0),
            profile_content_sha256: target_profile.map(|metadata| metadata.2),
            model_parameters: parameter_values,
        },
        case: CaseArtifact {
            id: case_id,
            waveform: waveform_name(summary.case.waveform),
            sample_rate_hz: summary.case.sample_rate_hz,
            frequency_hz: summary.case.frequency_hz,
            shape: summary.case.shape,
            pulse_width_percent: matches!(summary.case.waveform, Waveform::Pulse)
                .then_some((0.5 + 0.49 * summary.case.shape) * 100.0),
            warmup_samples: summary.case.warmup_samples,
            render_samples: summary.case.render_samples,
            seed: summary.case.seed,
            reset_phase: summary.case.reset_phase,
            normalization: "none",
            target_id: target_profile.map(|metadata| metadata.1),
            target_manifest_sha256: None,
        },
        render: RenderArtifact {
            case_elapsed_nanoseconds: elapsed_nanoseconds,
            nanoseconds_per_processed_sample: elapsed_nanoseconds as f64
                / (summary.case.warmup_samples + summary.case.render_samples) as f64,
            sample_hash_fnv1a64: format!("{:016x}", summary.sample_hash_fnv1a64),
            dc: summary.signal.dc,
            rms: summary.signal.rms,
            peak: summary.signal.peak,
            crest_factor: summary.signal.crest_factor,
            duty_above_midpoint: summary.signal.duty_above_midpoint,
            measured_frequency_hz: measured_frequency,
            pitch_error_cents: measured_frequency.map(|frequency| {
                1_200.0 * (frequency / f64::from(summary.case.frequency_hz)).log2()
            }),
        },
        spectrum: SpectrumArtifact {
            fft_size: spectrum.fft_size,
            window: "Hann",
            fundamental_dbfs: spectrum.fundamental_dbfs,
            residual_dbc: spectrum.residual_dbc,
            worst_residual_component_dbc: spectrum.worst_residual_component_dbc,
            note: "Residual excludes guarded legal-harmonic bins; measured target noise/drift must not be called aliasing.",
        },
        comparison: comparison.map(|(reference, metrics)| ComparisonArtifact {
            reference_model_id: reference.as_str(),
            normalized_rms_error: metrics.normalized_rms_error,
            maximum_absolute_error: metrics.maximum_absolute_error,
            correlation: metrics.correlation,
        }),
        files: FilesArtifact {
            source_wav: wav_path,
        },
    }
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options::default();
    let arguments: Vec<String> = env::args().skip(1).collect();
    let mut index = 0;
    while index < arguments.len() {
        let key = &arguments[index];
        if key == "--list" {
            options.list = true;
            index += 1;
            continue;
        }
        if key == "--help" || key == "-h" {
            print_help();
            std::process::exit(0);
        }
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {key}"))?;
        match key.as_str() {
            "--model" => {
                options.model = ResearchModelId::parse(value)
                    .ok_or_else(|| format!("unknown model {value:?}"))?
            }
            "--reference-model" => {
                options.reference_model = Some(
                    ResearchModelId::parse(value)
                        .ok_or_else(|| format!("unknown reference model {value:?}"))?,
                )
            }
            "--waveform" => options.waveform = parse_waveform(value)?,
            "--sample-rate" => options.sample_rate_hz = parse_number(key, value)?,
            "--frequency" => options.frequency_hz = parse_number(key, value)?,
            "--shape" => options.shape = parse_number(key, value)?,
            "--warmup" => options.warmup_samples = parse_number(key, value)?,
            "--samples" => options.render_samples = parse_number(key, value)?,
            "--seed" => options.seed = parse_number(key, value)?,
            "--param" => options.parameters.push(parse_parameter(value)?),
            "--output-root" => options.output_root = PathBuf::from(value),
            _ => return Err(format!("unknown option {key:?}; use --help")),
        }
        index += 2;
    }
    Ok(options)
}

fn parse_number<T: std::str::FromStr>(key: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid value {value:?} for {key}"))
}

fn parse_waveform(value: &str) -> Result<Waveform, String> {
    match value {
        "saw" => Ok(Waveform::Saw),
        "saw-triangle" => Ok(Waveform::SawTri),
        "triangle" => Ok(Waveform::Triangle),
        "pulse" => Ok(Waveform::Pulse),
        _ => Err(format!("unknown waveform {value:?}")),
    }
}

fn parse_parameter(value: &str) -> Result<(String, f32), String> {
    let (id, value) = value
        .split_once('=')
        .ok_or_else(|| format!("parameter {value:?} must use id=value"))?;
    if id.is_empty() {
        return Err("parameter ID cannot be empty".to_owned());
    }
    Ok((id.to_owned(), parse_number("--param", value)?))
}

fn waveform_name(waveform: Waveform) -> &'static str {
    match waveform {
        Waveform::Saw => "saw",
        Waveform::SawTri => "saw_triangle",
        Waveform::Triangle => "triangle",
        Waveform::Pulse => "pulse",
    }
}

fn case_id(case: ResearchRenderCase) -> String {
    format!(
        "{}-{:08.3}hz-{:06.1}khz-shape-{:05.3}",
        waveform_name(case.waveform),
        case.frequency_hz,
        case.sample_rate_hz / 1_000.0,
        case.shape,
    )
    .replace('.', "p")
}

fn reference_wavetable_bank() -> MipWavetableBank {
    static BANK: OnceLock<MipWavetableBank> = OnceLock::new();
    *BANK.get_or_init(|| {
        let mut samples = vec![0.0; WAVETABLE_BANK_SAMPLES];
        generate_wavetable_bank(&mut samples).expect("generate research wavetable bank");
        MipWavetableBank::new(Box::leak(samples.into_boxed_slice()))
            .expect("validate research wavetable bank")
    })
}

fn wavetable_bank(id: ResearchModelId) -> Result<WavetableBank, Box<dyn std::error::Error>> {
    let (file_name, profile): (&str, &'static WavetableProfile) = match id {
        ResearchModelId::WavetableMonologue => (
            "korg-monologue-measured-bank-v1.f32le",
            &MONOLOGUE_WAVETABLE_BANK_PROFILE,
        ),
        ResearchModelId::WavetableProphet5 => (
            "arturia-prophet5-measured-bank-v1.f32le",
            &PROPHET5_WAVETABLE_BANK_PROFILE,
        ),
        _ => return Err("model does not use a wavetable bank".into()),
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/analog-osc/banks")
        .join(file_name);
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "could not read measured bank {}: {error}; copy {file_name} into target/analog-osc/banks/",
            path.display()
        )
    })?;
    if bytes.len() % size_of::<f32>() != 0 {
        return Err(format!("measured bank {} has a partial f32", path.display()).into());
    }
    let mut samples = Vec::with_capacity(bytes.len() / size_of::<f32>());
    for chunk in bytes.chunks_exact(size_of::<f32>()) {
        samples.push(f32::from_le_bytes(
            chunk.try_into().expect("four-byte chunk"),
        ));
    }
    WavetableBank::new(Box::leak(samples.into_boxed_slice()), profile)
        .map_err(|error| format!("invalid measured bank {}: {error:?}", path.display()).into())
}

fn spectrum_metrics(samples: &[f32], sample_rate_hz: f32, expected_hz: f32) -> SpectrumMetrics {
    let fft_size = 1usize << (usize::BITS - samples.len().leading_zeros() - 1);
    let samples = &samples[..fft_size];
    let mut buffer: Vec<Complex32> = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            let window =
                0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / (fft_size - 1) as f32).cos();
            Complex32::new(sample * window, 0.0)
        })
        .collect();
    let window_sum: f64 = (0..fft_size)
        .map(|index| {
            f64::from(
                0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / (fft_size - 1) as f32).cos(),
            )
        })
        .sum();
    let mut planner = FftPlanner::new();
    planner.plan_fft_forward(fft_size).process(&mut buffer);
    let bins = fft_size / 2 + 1;
    let expected_bin = expected_hz * fft_size as f32 / sample_rate_hz;
    let search_radius = (expected_bin * 0.03).ceil().max(4.0) as usize;
    let search_start = (expected_bin as usize).saturating_sub(search_radius).max(1);
    let search_end = (expected_bin as usize + search_radius).min(bins - 1);
    let fundamental_bin = (search_start..=search_end)
        .max_by(|left, right| {
            buffer[*left]
                .norm_sqr()
                .total_cmp(&buffer[*right].norm_sqr())
        })
        .expect("non-empty fundamental search");
    let mut legal = vec![false; bins];
    for bin in 0..=3.min(bins - 1) {
        legal[bin] = true;
    }
    let guard = 4usize;
    let mut harmonic = 1usize;
    while harmonic * fundamental_bin < bins {
        let center = harmonic * fundamental_bin;
        for bin in center.saturating_sub(guard)..=(center + guard).min(bins - 1) {
            legal[bin] = true;
        }
        harmonic += 1;
    }
    let mut legal_power = 0.0_f64;
    let mut residual_power = 0.0_f64;
    let mut worst_residual = 0.0_f64;
    for bin in 1..bins {
        let power = f64::from(buffer[bin].norm_sqr());
        if legal[bin] {
            legal_power += power;
        } else {
            residual_power += power;
            worst_residual = worst_residual.max(power);
        }
    }
    let fundamental_power = (fundamental_bin.saturating_sub(guard)
        ..=(fundamental_bin + guard).min(bins - 1))
        .map(|bin| f64::from(buffer[bin].norm_sqr()))
        .sum::<f64>();
    let fundamental_amplitude = 2.0 * f64::from(buffer[fundamental_bin].norm()) / window_sum;
    SpectrumMetrics {
        fft_size,
        fundamental_dbfs: 20.0 * fundamental_amplitude.max(f64::MIN_POSITIVE).log10(),
        residual_dbc: 10.0
            * (residual_power / legal_power.max(f64::MIN_POSITIVE))
                .max(f64::MIN_POSITIVE)
                .log10(),
        worst_residual_component_dbc: 10.0
            * (worst_residual / fundamental_power.max(f64::MIN_POSITIVE))
                .max(f64::MIN_POSITIVE)
                .log10(),
    }
}

fn write_float_wav(path: &Path, sample_rate_hz: u32, samples: &[f32]) -> std::io::Result<()> {
    let data_bytes = u32::try_from(samples.len() * size_of::<f32>())
        .map_err(|_| std::io::Error::other("WAV is too large"))?;
    let mut output = BufWriter::new(File::create(path)?);
    output.write_all(b"RIFF")?;
    output.write_all(&(36_u32 + data_bytes).to_le_bytes())?;
    output.write_all(b"WAVEfmt ")?;
    output.write_all(&16_u32.to_le_bytes())?;
    output.write_all(&3_u16.to_le_bytes())?;
    output.write_all(&1_u16.to_le_bytes())?;
    output.write_all(&sample_rate_hz.to_le_bytes())?;
    output.write_all(&(sample_rate_hz * 4).to_le_bytes())?;
    output.write_all(&4_u16.to_le_bytes())?;
    output.write_all(&32_u16.to_le_bytes())?;
    output.write_all(b"data")?;
    output.write_all(&data_bytes.to_le_bytes())?;
    for sample in samples {
        output.write_all(&sample.to_le_bytes())?;
    }
    output.flush()
}

fn git_state() -> (String, bool) {
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_none_or(|output| !output.status.success() || !output.stdout.is_empty());
    (commit, dirty)
}

fn print_help() {
    println!(
        "analog_osc_research [options]\n\
         \n\
         --list                       List registered models\n\
         --model ID                   Candidate model (default baseline-v1)\n\
         --reference-model ID         Optional comparison model\n\
         --waveform NAME              saw|saw-triangle|triangle|pulse\n\
         --sample-rate HZ             Default 48000\n\
         --frequency HZ               Default 220\n\
         --shape VALUE                Normalized 0..1, default 0.5\n\
         --warmup SAMPLES             Default 4096\n\
         --samples SAMPLES            Default 65536\n\
         --seed INTEGER               Default 0\n\
         --param ID=VALUE             Repeatable model-specific parameter\n\
         --output-root PATH           Default target/analog-osc"
    );
}

fn size_of<T>() -> usize {
    std::mem::size_of::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectral_residual_separates_legal_tone_from_spur() {
        let sample_rate = 48_000.0_f32;
        let frequency = 375.0_f32;
        let clean: Vec<_> = (0..65_536)
            .map(|index| (std::f32::consts::TAU * frequency * index as f32 / sample_rate).sin())
            .collect();
        let mut contaminated = clean.clone();
        for (index, sample) in contaminated.iter_mut().enumerate() {
            *sample += 0.01 * (std::f32::consts::TAU * 7_011.0 * index as f32 / sample_rate).sin();
        }
        let clean_metrics = spectrum_metrics(&clean, sample_rate, frequency);
        let contaminated_metrics = spectrum_metrics(&contaminated, sample_rate, frequency);
        assert!(clean_metrics.residual_dbc < -80.0);
        assert!(contaminated_metrics.residual_dbc > clean_metrics.residual_dbc + 25.0);
        assert!(contaminated_metrics.worst_residual_component_dbc > -50.0);
    }

    #[test]
    fn float_wav_header_and_size_are_stable() {
        let path = env::temp_dir().join(format!(
            "analog-osc-research-{}-{}.wav",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let samples = [0.0, -0.5, 0.5, 1.0];
        write_float_wav(&path, 48_000, &samples).unwrap();
        let bytes = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(bytes.len(), 44 + samples.len() * size_of::<f32>());
    }

    #[test]
    fn parameter_arguments_require_stable_id_value_syntax() {
        assert_eq!(
            parse_parameter("drive=0.25").unwrap(),
            ("drive".to_owned(), 0.25)
        );
        assert!(parse_parameter("drive").is_err());
        assert!(parse_parameter("=0.25").is_err());
        assert!(parse_parameter("drive=loud").is_err());
    }
}
