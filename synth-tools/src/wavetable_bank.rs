use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use ndarray::Array2;
use ndarray_npy::NpzReader;
use num_complex::Complex;
use rustfft::FftPlanner;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const PHASE_BINS_SOURCE: usize = 2048;
pub const NYQUIST_GUARD: f64 = 0.45;
pub const MIN_ADJACENT_SPECTRAL_COSINE: f64 = 0.90;
pub const ROLE_TRAINING: u8 = 0;
pub const MAX_COMPILED_BANK_BYTES: usize = 20 * 1024 * 1024;
pub const MIP_HARMONIC_LIMITS: [u16; 33] = [
    1023, 860, 723, 607, 510, 428, 359, 301, 253, 212, 178, 149, 125, 105, 88, 73, 61, 51, 42, 35,
    29, 24, 20, 16, 13, 10, 8, 6, 5, 4, 3, 2, 1,
];

const RUNTIME_WAVEFORMS: [&str; 3] = ["saw", "triangle", "pulse"];

#[derive(Debug, Error)]
pub enum MeasuredBankError {
    #[error("{0}")]
    Message(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("npz error: {0}")]
    Npz(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrainingSelection {
    RoleZero,
    EvenRows,
}

#[derive(Clone, Debug)]
pub struct BankRequest {
    pub derived_root: PathBuf,
    pub output_dir: PathBuf,
    pub manifest_dir: PathBuf,
    pub profile_id: String,
    pub target_id: String,
    pub source_sample_rate_hz: f64,
    pub source_waveforms: [String; 3],
    pub training_selection: TrainingSelection,
    pub phase_manifest_path: Option<PathBuf>,
    pub rust_profile_path: Option<PathBuf>,
    pub rust_profile_symbol: String,
}

impl BankRequest {
    pub fn prophet5_defaults() -> Self {
        let research = default_research_root();
        Self {
            derived_root: research.join("captures/arturia-prophet5-v1-r7/derived"),
            output_dir: repository_root().join("target/analog-osc/banks"),
            manifest_dir: research.join("banks"),
            profile_id: "prophet5-wavetable-bank-v2".to_string(),
            target_id: "arturia-prophet5-v1".to_string(),
            source_sample_rate_hz: 96_000.0,
            source_waveforms: ["saw".into(), "triangle".into(), "pulse50".into()],
            training_selection: TrainingSelection::RoleZero,
            phase_manifest_path: None,
            rust_profile_path: Some(
                repository_root().join(
                    "synth-core/src/voice/osc_engine/wavetable_banks/prophet5_profile.rs",
                ),
            ),
            rust_profile_symbol: "PROPHET5_WAVETABLE_BANK_PROFILE".to_string(),
        }
    }

    pub fn monologue_defaults() -> Self {
        let root = repository_root();
        Self {
            derived_root: root.join("target/analog-osc/reference/korg-monologue-v1/derived"),
            output_dir: root.join("target/analog-osc/banks"),
            manifest_dir: root.join("plans/analog-osc/research/banks"),
            profile_id: "korg-monologue-measured-wavetable-v2".to_string(),
            target_id: "korg-monologue-v1".to_string(),
            source_sample_rate_hz: 48_000.0,
            source_waveforms: ["saw".into(), "triangle".into(), "square".into()],
            training_selection: TrainingSelection::EvenRows,
            phase_manifest_path: Some(
                root.join("plans/analog-osc/research/banks/korg-monologue-measured-bank-v1.json"),
            ),
            rust_profile_path: Some(root.join(
                "synth-core/src/voice/osc_engine/wavetable_banks/monologue_profile.rs",
            )),
            rust_profile_symbol: "MONOLOGUE_WAVETABLE_BANK_PROFILE".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BankBuildResult {
    pub binary_path: PathBuf,
    pub manifest_path: PathBuf,
    pub sample_count: usize,
    pub pitch_count_per_waveform: usize,
    pub rust_profile_path: Option<PathBuf>,
}

pub fn default_research_root() -> PathBuf {
    repository_root().join("plans/analog-osc/research")
}

pub fn build_bank(request: &BankRequest) -> Result<BankBuildResult, MeasuredBankError> {
    validate_request(request)?;
    let phase_manifest = request
        .phase_manifest_path
        .as_ref()
        .map(|path| read_json(path))
        .transpose()?;
    let mut bank = Vec::new();
    let mut waveform_metadata = serde_json::Map::new();
    let mut pitch_count = None;

    for (runtime_waveform, source_waveform) in
        RUNTIME_WAVEFORMS.iter().zip(&request.source_waveforms)
    {
        let npz_path = request
            .derived_root
            .join(format!("{source_waveform}-cycles-v1.npz"));
        let phase_shifts = phase_manifest
            .as_ref()
            .map(|manifest| phase_shifts_from_manifest(manifest, source_waveform))
            .transpose()?;
        let tables = waveform_tables(
            &npz_path,
            request.training_selection,
            phase_shifts.as_deref(),
        )?;
        match pitch_count {
            None => pitch_count = Some(tables.training_indices.len()),
            Some(expected) if expected != tables.training_indices.len() => {
                return Err(MeasuredBankError::Message(format!(
                    "{source_waveform}: training pitch count {} != {expected}",
                    tables.training_indices.len()
                )));
            }
            Some(_) => {}
        }
        bank.extend_from_slice(&tables.samples);
        waveform_metadata.insert(
            (*runtime_waveform).to_string(),
            serde_json::json!({
                "source_waveform": source_waveform,
                "source_npz_sha256": sha256_file(&npz_path)?,
                "training_pitch_indices": tables.training_indices,
                "training_frequencies_hz": tables.training_frequencies_hz,
                "maximum_supported_frequency_hz": tables.maximum_supported_frequency_hz,
                "level_policy": "measured source amplitude retained",
                "dc_policy": "measured source DC retained",
                "phase_policy": if phase_shifts.is_some() {
                    "v1 global phase shifts applied in the complex spectrum"
                } else {
                    "extracted landmark phase retained"
                },
                "training_global_phase_shifts_cycles": phase_shifts,
                "adjacent_spectral_cosine": {
                    "minimum_required": MIN_ADJACENT_SPECTRAL_COSINE,
                    "scores": tables.adjacent_spectral_cosines,
                },
            }),
        );
    }

    let pitch_count = pitch_count.expect("three runtime waveforms");
    let table_lengths = mip_table_lengths();
    let mip_offsets = mip_offsets(pitch_count, &table_lengths);
    let samples_per_waveform =
        mip_offsets.last().copied().unwrap() + pitch_count * table_lengths.last().copied().unwrap();
    let expected = RUNTIME_WAVEFORMS.len() * samples_per_waveform;
    if bank.len() != expected || bank.iter().any(|sample| !sample.is_finite()) {
        return Err(MeasuredBankError::Message(format!(
            "invalid generated bank: samples={} expected={expected}",
            bank.len()
        )));
    }
    if bank.len() * 4 > MAX_COMPILED_BANK_BYTES {
        return Err(MeasuredBankError::Message(format!(
            "generated bank is {} bytes; combined-bank cap is {} bytes",
            bank.len() * 4,
            MAX_COMPILED_BANK_BYTES
        )));
    }

    fs::create_dir_all(&request.output_dir)?;
    fs::create_dir_all(&request.manifest_dir)?;
    let binary_path = request
        .output_dir
        .join(format!("{}.f32le", request.profile_id));
    let manifest_path = request
        .manifest_dir
        .join(format!("{}.json", request.profile_id));
    write_samples(&binary_path, &bank)?;

    let mip_metadata: Vec<_> = MIP_HARMONIC_LIMITS
        .iter()
        .zip(&table_lengths)
        .zip(&mip_offsets)
        .map(
            |((&harmonic_limit, &table_length), &sample_offset_per_waveform)| {
                serde_json::json!({
                    "harmonic_limit": harmonic_limit,
                    "table_length": table_length,
                    "sample_offset_per_waveform": sample_offset_per_waveform,
                })
            },
        )
        .collect();
    let mut manifest = serde_json::json!({
        "schema_version": 2,
        "profile_id": request.profile_id,
        "target_id": request.target_id,
        "sample_format": "little-endian IEEE-754 float32",
        "layout": {
            "order": "waveform, mip, training pitch, sample",
            "waveforms": RUNTIME_WAVEFORMS,
            "pitch_count_per_waveform": pitch_count,
            "mip_count": MIP_HARMONIC_LIMITS.len(),
            "mips": mip_metadata,
            "samples_per_waveform": samples_per_waveform,
            "sample_count": bank.len(),
            "nyquist_guard": NYQUIST_GUARD,
            "selection": "richest safe mip plus adjacent leaner mip; log-space blend from a 1024-entry runtime lookup",
        },
        "source": {
            "capture_sample_rate_hz": request.source_sample_rate_hz,
            "capture_sample_rate_role": "provenance only; not a playback compatibility gate",
            "phase_bins": PHASE_BINS_SOURCE,
        },
        "waveforms": waveform_metadata,
        "bank_binary": {
            "path": manifest_binary_path(&binary_path, &manifest_path),
            "bytes": bank.len() * 4,
            "sample_count": bank.len(),
            "fnv1a32": fnv1a32(&bank),
            "sha256": sha256_file(&binary_path)?,
        },
        "identity_warning": if request.target_id.contains("arturia") {
            "Prophet-5 V is a software instrument; this bank is not a Sequential hardware reference."
        } else {
            "Korg Monologue public real-hardware dataset; capture bandwidth limits retained."
        },
    });
    let checksum = manifest_content_sha256(&manifest)?;
    manifest
        .as_object_mut()
        .expect("manifest is an object")
        .insert(
            "manifest_content_sha256".to_string(),
            serde_json::Value::String(checksum.clone()),
        );
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    if let Some(path) = &request.rust_profile_path {
        write_rust_profile(path, request, &manifest, &checksum)?;
    }

    Ok(BankBuildResult {
        binary_path,
        manifest_path,
        sample_count: bank.len(),
        pitch_count_per_waveform: pitch_count,
        rust_profile_path: request.rust_profile_path.clone(),
    })
}

pub fn reconstruct_mip(
    cycle: &[f32],
    harmonic_limit: usize,
    phase_shift_cycles: f64,
) -> Result<Vec<f32>, MeasuredBankError> {
    if cycle.len() != PHASE_BINS_SOURCE || harmonic_limit == 0 || harmonic_limit > 1023 {
        return Err(MeasuredBankError::Message(format!(
            "invalid source cycle/mip: samples={} harmonic_limit={harmonic_limit}",
            cycle.len()
        )));
    }
    let spectrum = complex_spectrum(cycle);
    let table_length = table_length(harmonic_limit);
    let scale = table_length as f64 / PHASE_BINS_SOURCE as f64;
    let mut output = vec![Complex::new(0.0, 0.0); table_length];
    output[0] = spectrum[0] * scale;
    for harmonic in 1..=harmonic_limit {
        let angle = std::f64::consts::TAU * harmonic as f64 * phase_shift_cycles;
        let rotation = Complex::new(angle.cos(), angle.sin());
        output[harmonic] = spectrum[harmonic] * rotation * scale;
        output[table_length - harmonic] = output[harmonic].conj();
    }
    FftPlanner::<f64>::new()
        .plan_fft_inverse(table_length)
        .process(&mut output);
    Ok(output
        .into_iter()
        .map(|sample| (sample.re / table_length as f64) as f32)
        .collect())
}

pub fn write_synthetic_npz(
    path: &Path,
    cycles: &Array2<f32>,
    frequencies: &[f64],
    roles: &[u8],
) -> Result<(), MeasuredBankError> {
    use ndarray::Array1;
    use ndarray_npy::NpzWriter;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    let mut npz = NpzWriter::new(file);
    npz.add_array("median_cycles", cycles)
        .map_err(|error| MeasuredBankError::Npz(error.to_string()))?;
    npz.add_array(
        "measured_frequency_hz",
        &Array1::from_vec(frequencies.to_vec()),
    )
    .map_err(|error| MeasuredBankError::Npz(error.to_string()))?;
    npz.add_array("role", &Array1::from_vec(roles.to_vec()))
        .map_err(|error| MeasuredBankError::Npz(error.to_string()))?;
    npz.finish()
        .map_err(|error| MeasuredBankError::Npz(error.to_string()))?;
    Ok(())
}

struct WaveformTables {
    samples: Vec<f32>,
    training_indices: Vec<usize>,
    training_frequencies_hz: Vec<f64>,
    maximum_supported_frequency_hz: f64,
    adjacent_spectral_cosines: Vec<f64>,
}

fn waveform_tables(
    path: &Path,
    selection: TrainingSelection,
    phase_shifts: Option<&[f64]>,
) -> Result<WaveformTables, MeasuredBankError> {
    let file = File::open(path)?;
    let mut reader =
        NpzReader::new(file).map_err(|error| MeasuredBankError::Npz(error.to_string()))?;
    let cycles: Array2<f32> = reader
        .by_name("median_cycles")
        .map_err(|error| MeasuredBankError::Npz(format!("median_cycles: {error}")))?;
    let frequencies: ndarray::Array1<f64> = reader
        .by_name("measured_frequency_hz")
        .map_err(|error| MeasuredBankError::Npz(format!("measured_frequency_hz: {error}")))?;
    let roles: Option<ndarray::Array1<u8>> = if selection == TrainingSelection::RoleZero {
        Some(
            reader
                .by_name("role")
                .map_err(|error| MeasuredBankError::Npz(format!("role: {error}")))?,
        )
    } else {
        None
    };
    if cycles.nrows() != frequencies.len()
        || roles
            .as_ref()
            .is_some_and(|values| values.len() != cycles.nrows())
        || cycles.ncols() != PHASE_BINS_SOURCE
        || cycles.iter().any(|sample| !sample.is_finite())
    {
        return Err(MeasuredBankError::Message(format!(
            "{}: invalid source shapes or samples",
            path.display()
        )));
    }
    if frequencies
        .iter()
        .any(|frequency| !frequency.is_finite() || *frequency <= 0.0)
        || frequencies
            .windows(2)
            .into_iter()
            .any(|pair| pair[1] <= pair[0])
    {
        return Err(MeasuredBankError::Message(format!(
            "{}: frequencies must be finite, positive, and increasing",
            path.display()
        )));
    }
    let training_indices: Vec<usize> = match selection {
        TrainingSelection::RoleZero => roles
            .as_ref()
            .unwrap()
            .iter()
            .enumerate()
            .filter_map(|(index, role)| (*role == ROLE_TRAINING).then_some(index))
            .collect(),
        TrainingSelection::EvenRows => (0..cycles.nrows()).step_by(2).collect(),
    };
    if training_indices.is_empty() {
        return Err(MeasuredBankError::Message(format!(
            "{}: no training rows",
            path.display()
        )));
    }
    if phase_shifts.is_some_and(|shifts| shifts.len() != training_indices.len()) {
        return Err(MeasuredBankError::Message(format!(
            "{}: phase shift count does not match training rows",
            path.display()
        )));
    }
    let adjacent_spectral_cosines = adjacent_spectral_cosines(&cycles, &training_indices)?;
    if let Some((ordinal, score)) = adjacent_spectral_cosines
        .iter()
        .copied()
        .enumerate()
        .find(|(_, score)| *score < MIN_ADJACENT_SPECTRAL_COSINE)
    {
        return Err(MeasuredBankError::Message(format!(
            "{}: incoherent adjacent training cycles at rows {}/{}: spectral cosine {score:.4} < {MIN_ADJACENT_SPECTRAL_COSINE:.2}",
            path.display(),
            training_indices[ordinal],
            training_indices[ordinal + 1]
        )));
    }
    let mut samples = Vec::new();
    for &limit in &MIP_HARMONIC_LIMITS {
        for (ordinal, &row) in training_indices.iter().enumerate() {
            let cycle: Vec<f32> = cycles.row(row).iter().copied().collect();
            let shift = phase_shifts.map_or(0.0, |values| values[ordinal]);
            samples.extend(reconstruct_mip(&cycle, usize::from(limit), shift)?);
        }
    }
    Ok(WaveformTables {
        samples,
        training_frequencies_hz: training_indices
            .iter()
            .map(|&index| frequencies[index])
            .collect(),
        maximum_supported_frequency_hz: frequencies.iter().copied().fold(0.0, f64::max),
        training_indices,
        adjacent_spectral_cosines,
    })
}

fn validate_request(request: &BankRequest) -> Result<(), MeasuredBankError> {
    if !request.derived_root.is_dir() {
        return Err(MeasuredBankError::Message(format!(
            "derived root not found: {} (run synth-capture extract first)",
            request.derived_root.display()
        )));
    }
    if !request.source_sample_rate_hz.is_finite() || request.source_sample_rate_hz <= 0.0 {
        return Err(MeasuredBankError::Message(
            "source sample rate must be finite and positive".into(),
        ));
    }
    Ok(())
}

fn write_rust_profile(
    path: &Path,
    request: &BankRequest,
    manifest: &serde_json::Value,
    manifest_checksum: &str,
) -> Result<(), MeasuredBankError> {
    let fnv = manifest["bank_binary"]["fnv1a32"]
        .as_u64()
        .ok_or_else(|| MeasuredBankError::Message("manifest fnv1a32 missing".into()))?;
    let sample_count = manifest["bank_binary"]["sample_count"]
        .as_u64()
        .ok_or_else(|| MeasuredBankError::Message("manifest sample count missing".into()))?;
    let samples_per_waveform = manifest["layout"]["samples_per_waveform"]
        .as_u64()
        .ok_or_else(|| MeasuredBankError::Message("samples per waveform missing".into()))?;
    let mut source = String::from(
        "//! Generated by synth-tools wavetable_bank; do not edit.\n\n\
         use crate::dsp::WavetableProfile;\n\n",
    );
    source.push_str(&format!(
        "pub const WAVETABLE_BANK_PROFILE_ID: &str = {:?};\n\
         pub const WAVETABLE_BANK_TARGET_ID: &str = {:?};\n\
         pub const WAVETABLE_BANK_MANIFEST_SHA256: &str = {manifest_checksum:?};\n\
         pub const WAVETABLE_BANK_FNV1A32: u32 = 0x{fnv:08x};\n\
         pub const WAVETABLE_BANK_SAMPLE_COUNT: usize = {sample_count};\n\
         pub const WAVETABLE_BANK_SAMPLES_PER_WAVEFORM: usize = {samples_per_waveform};\n\
         pub const WAVETABLE_BANK_SOURCE_SAMPLE_RATE_HZ: f32 = {:.9e}_f32;\n\n",
        request.profile_id, request.target_id, request.source_sample_rate_hz
    ));
    write_integer_array(
        &mut source,
        "WAVETABLE_MIP_HARMONIC_LIMITS",
        &MIP_HARMONIC_LIMITS,
    );
    let lengths: Vec<u16> = manifest["layout"]["mips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|mip| mip["table_length"].as_u64().unwrap() as u16)
        .collect();
    write_integer_array(&mut source, "WAVETABLE_MIP_TABLE_LENGTHS", &lengths);
    let offsets: Vec<u32> = manifest["layout"]["mips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|mip| mip["sample_offset_per_waveform"].as_u64().unwrap() as u32)
        .collect();
    write_integer_array(&mut source, "WAVETABLE_MIP_OFFSETS", &offsets);
    for (waveform, prefix) in [("saw", "SAW"), ("triangle", "TRIANGLE"), ("pulse", "PULSE")] {
        let frequencies = manifest["waveforms"][waveform]["training_frequencies_hz"]
            .as_array()
            .ok_or_else(|| MeasuredBankError::Message(format!("{waveform} frequencies missing")))?;
        source.push_str(&format!(
            "pub const WAVETABLE_{prefix}_FREQUENCIES_HZ: [f32; {}] = [\n",
            frequencies.len()
        ));
        for frequency in frequencies {
            source.push_str(&format!("    {:.9e}_f32,\n", frequency.as_f64().unwrap()));
        }
        let maximum = manifest["waveforms"][waveform]["maximum_supported_frequency_hz"]
            .as_f64()
            .unwrap();
        source.push_str(&format!(
            "];\n\npub const WAVETABLE_{prefix}_MAXIMUM_HZ: f32 = {maximum:.9e}_f32;\n\n"
        ));
    }
    source.push_str(&format!(
        "pub static {}: WavetableProfile = WavetableProfile {{\n\
             id: WAVETABLE_BANK_PROFILE_ID,\n\
             target_id: WAVETABLE_BANK_TARGET_ID,\n\
             manifest_sha256: WAVETABLE_BANK_MANIFEST_SHA256,\n\
             fnv1a32: WAVETABLE_BANK_FNV1A32,\n\
             sample_count: WAVETABLE_BANK_SAMPLE_COUNT,\n\
             samples_per_waveform: WAVETABLE_BANK_SAMPLES_PER_WAVEFORM,\n\
             source_sample_rate_hz: WAVETABLE_BANK_SOURCE_SAMPLE_RATE_HZ,\n\
             mip_harmonic_limits: &WAVETABLE_MIP_HARMONIC_LIMITS,\n\
             mip_table_lengths: &WAVETABLE_MIP_TABLE_LENGTHS,\n\
             mip_offsets: &WAVETABLE_MIP_OFFSETS,\n\
             legacy_table_length: 0,\n\
             legacy_reference_sample_rate_hz: 0.0,\n\
             saw_hz: &WAVETABLE_SAW_FREQUENCIES_HZ,\n\
             triangle_hz: &WAVETABLE_TRIANGLE_FREQUENCIES_HZ,\n\
             pulse_hz: &WAVETABLE_PULSE_FREQUENCIES_HZ,\n\
             saw_max_hz: WAVETABLE_SAW_MAXIMUM_HZ,\n\
             triangle_max_hz: WAVETABLE_TRIANGLE_MAXIMUM_HZ,\n\
             pulse_max_hz: WAVETABLE_PULSE_MAXIMUM_HZ,\n\
         }};\n",
        request.rust_profile_symbol
    ));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, source)?;
    Ok(())
}

fn write_integer_array<T: std::fmt::Display>(source: &mut String, name: &str, values: &[T]) {
    source.push_str(&format!(
        "pub const {name}: [{}; {}] = [\n",
        std::any::type_name::<T>(),
        values.len()
    ));
    for value in values {
        source.push_str(&format!("    {value},\n"));
    }
    source.push_str("];\n\n");
}

fn phase_shifts_from_manifest(
    manifest: &serde_json::Value,
    waveform: &str,
) -> Result<Vec<f64>, MeasuredBankError> {
    manifest["waveforms"][waveform]["training_global_phase_shifts_cycles"]
        .as_array()
        .ok_or_else(|| MeasuredBankError::Message(format!("{waveform}: phase shifts missing")))?
        .iter()
        .map(|value| {
            value.as_f64().ok_or_else(|| {
                MeasuredBankError::Message(format!("{waveform}: invalid phase shift"))
            })
        })
        .collect()
}

fn adjacent_spectral_cosines(
    cycles: &Array2<f32>,
    training_indices: &[usize],
) -> Result<Vec<f64>, MeasuredBankError> {
    let spectra: Vec<Vec<f64>> = training_indices
        .iter()
        .map(|&index| {
            complex_spectrum(&cycles.row(index).iter().copied().collect::<Vec<_>>())[1..=256]
                .iter()
                .map(|bin| bin.norm())
                .collect()
        })
        .collect();
    spectra
        .windows(2)
        .map(|pair| {
            let dot = pair[0]
                .iter()
                .zip(&pair[1])
                .map(|(a, b)| a * b)
                .sum::<f64>();
            let left = pair[0]
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
            let right = pair[1]
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
            if left <= 1.0e-20 || right <= 1.0e-20 {
                return Err(MeasuredBankError::Message(
                    "training cycle has no usable harmonic energy".into(),
                ));
            }
            Ok((dot / (left * right)).clamp(0.0, 1.0))
        })
        .collect()
}

fn complex_spectrum(cycle: &[f32]) -> Vec<Complex<f64>> {
    let mut spectrum: Vec<_> = cycle
        .iter()
        .map(|sample| Complex::new(f64::from(*sample), 0.0))
        .collect();
    FftPlanner::<f64>::new()
        .plan_fft_forward(cycle.len())
        .process(&mut spectrum);
    spectrum
}

fn mip_table_lengths() -> Vec<usize> {
    MIP_HARMONIC_LIMITS
        .iter()
        .map(|limit| table_length(usize::from(*limit)))
        .collect()
}

fn table_length(harmonic_limit: usize) -> usize {
    (2 * (harmonic_limit + 1)).next_power_of_two().max(64)
}

fn mip_offsets(pitch_count: usize, lengths: &[usize]) -> Vec<usize> {
    let mut offset = 0;
    lengths
        .iter()
        .map(|length| {
            let current = offset;
            offset += pitch_count * length;
            current
        })
        .collect()
}

fn read_json(path: &Path) -> Result<serde_json::Value, MeasuredBankError> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn manifest_binary_path(binary_path: &Path, manifest_path: &Path) -> String {
    if binary_path.parent() == manifest_path.parent() {
        return binary_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("bank.f32le")
            .to_string();
    }
    binary_path
        .strip_prefix(repository_root())
        .unwrap_or(binary_path)
        .to_string_lossy()
        .into_owned()
}

fn write_samples(path: &Path, samples: &[f32]) -> Result<(), MeasuredBankError> {
    let mut file = File::create(path)?;
    for sample in samples {
        file.write_all(&sample.to_le_bytes())?;
    }
    file.sync_all()?;
    Ok(())
}

fn fnv1a32(values: &[f32]) -> u32 {
    values.iter().fold(0x811c_9dc5, |hash, sample| {
        sample.to_le_bytes().iter().fold(hash, |hash, byte| {
            (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
        })
    })
}

fn sha256_file(path: &Path) -> Result<String, MeasuredBankError> {
    Ok(hex_encode(Sha256::digest(fs::read(path)?)))
}

fn manifest_content_sha256(manifest: &serde_json::Value) -> Result<String, MeasuredBankError> {
    Ok(hex_encode(Sha256::digest(serde_json::to_vec(manifest)?)))
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("synth-tools is directly under the repository root")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use ndarray::Array2;
    use rustfft::{FftPlanner, num_complex::Complex};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn hierarchy_and_lengths_match_v2_contract() {
        let mut generated = vec![1023_u16];
        while *generated.last().unwrap() > 1 {
            let previous = *generated.last().unwrap();
            let next = (f64::from(previous) / 2.0_f64.powf(0.25)).floor() as u16;
            if next != previous {
                generated.push(next.max(1));
            }
        }
        assert_eq!(generated, MIP_HARMONIC_LIMITS);
        assert_eq!(MIP_HARMONIC_LIMITS[0], 1023);
        assert_eq!(*MIP_HARMONIC_LIMITS.last().unwrap(), 1);
        assert!(MIP_HARMONIC_LIMITS.windows(2).all(|pair| pair[1] < pair[0]));
        for limit in MIP_HARMONIC_LIMITS {
            let length = table_length(usize::from(limit));
            assert!(length.is_power_of_two());
            assert!(length >= 64);
            assert!(length >= 2 * (usize::from(limit) + 1));
        }
    }

    #[test]
    fn every_phase_rich_mip_preserves_legal_complex_bins_and_removes_the_rest() {
        let cycle: Vec<f32> = (0..PHASE_BINS_SOURCE)
            .map(|index| {
                let phase = std::f64::consts::TAU * index as f64 / PHASE_BINS_SOURCE as f64;
                (0.7 * phase.sin()
                    + 0.2 * (7.0 * phase + 0.37).sin()
                    + 0.08 * (113.0 * phase - 0.21).sin()
                    + 0.02 * (700.0 * phase + 0.49).sin()) as f32
            })
            .collect();
        let source_spectrum = complex_spectrum(&cycle);
        let shift = 0.125;
        for limit in MIP_HARMONIC_LIMITS.map(usize::from) {
            let table = reconstruct_mip(&cycle, limit, shift).unwrap();
            let mut spectrum: Vec<_> = table
                .iter()
                .map(|sample| Complex::new(f64::from(*sample), 0.0))
                .collect();
            FftPlanner::<f64>::new()
                .plan_fft_forward(table.len())
                .process(&mut spectrum);
            assert!(
                spectrum[(limit + 1)..table.len() / 2]
                    .iter()
                    .all(|bin| bin.norm() < 1.0e-3),
                "limit={limit}"
            );
            let legal = [700, 113, 7, 1]
                .into_iter()
                .find(|harmonic| *harmonic <= limit)
                .unwrap();
            let angle = std::f64::consts::TAU * legal as f64 * shift;
            let expected = source_spectrum[legal] / PHASE_BINS_SOURCE as f64
                * Complex::new(angle.cos(), angle.sin());
            let actual = spectrum[legal] / table.len() as f64;
            assert!(
                (actual - expected).norm() < 1.0e-6,
                "limit={limit} harmonic={legal} actual={actual:?} expected={expected:?}"
            );
        }
    }

    #[test]
    fn synthetic_v2_generation_is_deterministic() {
        let directory = tempdir().unwrap();
        let derived = directory.path().join("derived");
        write_fixture(&derived, false);
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        let first_result = build_bank(&request(&derived, &first)).unwrap();
        let second_result = build_bank(&request(&derived, &second)).unwrap();
        assert_eq!(
            fs::read(first_result.binary_path).unwrap(),
            fs::read(second_result.binary_path).unwrap()
        );
        assert_eq!(
            fs::read(first_result.manifest_path).unwrap(),
            fs::read(second_result.manifest_path).unwrap()
        );
    }

    #[test]
    fn incoherent_sources_are_rejected() {
        let directory = tempdir().unwrap();
        let derived = directory.path().join("derived");
        write_fixture(&derived, true);
        let error = build_bank(&request(&derived, &directory.path().join("output"))).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("incoherent adjacent training cycles")
        );
    }

    fn request(derived: &Path, output: &Path) -> BankRequest {
        BankRequest {
            derived_root: derived.to_path_buf(),
            output_dir: output.to_path_buf(),
            manifest_dir: output.to_path_buf(),
            profile_id: "synthetic-v2".into(),
            target_id: "synthetic".into(),
            source_sample_rate_hz: 96_000.0,
            source_waveforms: ["saw".into(), "triangle".into(), "pulse50".into()],
            training_selection: TrainingSelection::RoleZero,
            phase_manifest_path: None,
            rust_profile_path: None,
            rust_profile_symbol: "SYNTHETIC_PROFILE".into(),
        }
    }

    fn write_fixture(derived: &Path, incoherent: bool) {
        for waveform in ["saw", "triangle", "pulse50"] {
            let mut cycles = Array2::<f32>::zeros((3, PHASE_BINS_SOURCE));
            for row in 0..3 {
                for index in 0..PHASE_BINS_SOURCE {
                    let phase = std::f64::consts::TAU * index as f64 / PHASE_BINS_SOURCE as f64;
                    let harmonic = if incoherent && row == 1 { 97.0 } else { 1.0 };
                    cycles[[row, index]] =
                        (harmonic * phase).sin() as f32 * (1.0 + row as f32 * 0.01);
                }
            }
            write_synthetic_npz(
                &derived.join(format!("{waveform}-cycles-v1.npz")),
                &cycles,
                &[110.0, 123.0, 138.0],
                &[ROLE_TRAINING; 3],
            )
            .unwrap();
        }
    }
}
