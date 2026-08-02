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

pub const TABLE_LENGTH: usize = 2048;
/// Playback rate the generated tables are safe for. Source captures may use a
/// higher rate; that must not leak into the runtime anti-aliasing policy.
pub const DEFAULT_REFERENCE_SAMPLE_RATE_HZ: f64 = 48_000.0;
pub const NYQUIST_GUARD: f64 = 0.45;
pub const MIN_ADJACENT_SPECTRAL_COSINE: f64 = 0.90;
pub const ROLE_TRAINING: u8 = 0;
pub const PHASE_BINS_SOURCE: usize = 2048;

pub const WAVEFORMS: [&str; 3] = ["saw", "triangle", "pulse50"];

const DEFAULT_PROFILE_ID: &str = "prophet5-wavetable-bank-v1";
const DEFAULT_TARGET_ID: &str = "prophet5-v1";

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

#[derive(Clone, Debug)]
pub struct BankRequest {
    pub derived_root: PathBuf,
    pub output_dir: PathBuf,
    pub profile_id: String,
    pub target_id: String,
    pub reference_sample_rate_hz: f64,
    pub rust_profile_path: Option<PathBuf>,
}

impl BankRequest {
    pub fn prophet5_defaults() -> Self {
        let research = default_research_root();
        Self {
            derived_root: research.join("captures/arturia-prophet5-v1-r7/derived"),
            output_dir: research.join("banks"),
            profile_id: DEFAULT_PROFILE_ID.to_string(),
            target_id: DEFAULT_TARGET_ID.to_string(),
            reference_sample_rate_hz: DEFAULT_REFERENCE_SAMPLE_RATE_HZ,
            rust_profile_path: Some(
                repository_root().join("synth-core/src/dsp/wavetable_bank_profile_prophet5.rs"),
            ),
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

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("synth-tools must live directly under the repository root")
        .to_path_buf()
}

pub fn build_bank(request: &BankRequest) -> Result<BankBuildResult, MeasuredBankError> {
    if !request.derived_root.is_dir() {
        return Err(MeasuredBankError::Message(format!(
            "derived root not found: {} (run synth-capture extract first)",
            request.derived_root.display()
        )));
    }

    let mut banks = Vec::new();
    let mut waveform_metadata = serde_json::Map::new();
    let mut pitch_count = None;

    for waveform in WAVEFORMS {
        let npz_path = request
            .derived_root
            .join(format!("{waveform}-cycles-v1.npz"));
        let WaveformTables {
            tables,
            training_indices,
            training_frequencies_hz,
            harmonic_limits,
            guard_frequencies_hz,
            maximum_supported_frequency_hz,
            adjacent_spectral_cosines,
        } = waveform_tables(&npz_path, request.reference_sample_rate_hz)?;

        let count = training_indices.len();
        if count == 0 {
            return Err(MeasuredBankError::Message(format!(
                "{waveform}: no training rows (role == {ROLE_TRAINING})"
            )));
        }
        match pitch_count {
            None => pitch_count = Some(count),
            Some(expected) if expected != count => {
                return Err(MeasuredBankError::Message(format!(
                    "{waveform}: training pitch count {count} != {expected}"
                )));
            }
            Some(_) => {}
        }

        banks.extend_from_slice(&tables);
        waveform_metadata.insert(
            waveform.to_string(),
            serde_json::json!({
                "source_npz_sha256": sha256_file(&npz_path)?,
                "training_pitch_indices": training_indices,
                "training_frequencies_hz": training_frequencies_hz,
                "phase_policy": "extraction upward-crossing landmark (no production-source alignment)",
                "pitch_safe_harmonic_limits": harmonic_limits,
                "pitch_guard_frequencies_hz": guard_frequencies_hz,
                "maximum_supported_frequency_hz": maximum_supported_frequency_hz,
                "adjacent_spectral_cosine": {
                    "minimum_required": MIN_ADJACENT_SPECTRAL_COSINE,
                    "scores": adjacent_spectral_cosines,
                },
            }),
        );
    }

    let pitch_count = pitch_count.unwrap();
    let sample_count = banks.len();
    let expected = WAVEFORMS.len() * pitch_count * TABLE_LENGTH;
    if sample_count != expected || banks.iter().any(|sample| !sample.is_finite()) {
        return Err(MeasuredBankError::Message(format!(
            "invalid generated bank: samples={sample_count} expected={expected}"
        )));
    }

    fs::create_dir_all(&request.output_dir)?;
    let binary_path = request
        .output_dir
        .join(format!("{}.f32le", request.profile_id));
    let manifest_path = request
        .output_dir
        .join(format!("{}.json", request.profile_id));

    {
        let mut file = File::create(&binary_path)?;
        for sample in &banks {
            file.write_all(&sample.to_le_bytes())?;
        }
        file.sync_all()?;
    }

    let mut manifest = serde_json::json!({
        "schema_version": 1,
        "profile_id": request.profile_id,
        "target_id": request.target_id,
        "sample_format": "little-endian IEEE-754 float32",
        "layout": {
            "order": "waveform, training pitch, sample",
            "waveforms": WAVEFORMS,
            "pitch_count_per_waveform": pitch_count,
            "table_length": TABLE_LENGTH,
            "reference_sample_rate_hz": request.reference_sample_rate_hz,
            "nyquist_guard": NYQUIST_GUARD,
            "pitch_safety_policy": "each training table is truncated for the next training pitch; the final table is truncated for the highest measured pitch",
            "sample_count": sample_count,
        },
        "phase_bins_source": PHASE_BINS_SOURCE,
        "waveforms": waveform_metadata,
        "bank_binary": {
            "path": binary_path.file_name().and_then(|name| name.to_str()).unwrap_or("bank.f32le"),
            "bytes": sample_count * 4,
            "sample_count": sample_count,
            "fnv1a32": fnv1a32(&banks),
            "sha256": sha256_file(&binary_path)?,
        },
        "identity_warning": "Prophet-5 V is a software instrument. This bank is not a Sequential/Prophet hardware reference.",
        "prior_work": {
            "role": "methodological_prior_for_static_chromatic_protocol_and_cycle_extraction",
            "dataset": "Korg Monologue analog VCO — Simionato/Fasciani",
            "doi": "10.5281/zenodo.15196138",
            "record_url": "https://zenodo.org/records/15196138",
            "license": "CC-BY-4.0",
            "paper_url": "https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf",
        },
    });

    let checksum = manifest_content_sha256(&manifest)?;
    manifest
        .as_object_mut()
        .ok_or_else(|| MeasuredBankError::Message("manifest root must be object".into()))?
        .insert(
            "manifest_content_sha256".to_string(),
            serde_json::Value::String(checksum.clone()),
        );

    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    if let Some(path) = &request.rust_profile_path {
        write_rust_profile(path, &manifest, &checksum)?;
    }

    Ok(BankBuildResult {
        binary_path,
        manifest_path,
        sample_count,
        pitch_count_per_waveform: pitch_count,
        rust_profile_path: request.rust_profile_path.clone(),
    })
}

fn write_rust_profile(
    path: &Path,
    manifest: &serde_json::Value,
    manifest_checksum: &str,
) -> Result<(), MeasuredBankError> {
    let mut source = String::from(
        "//! Generated by synth-tools wavetable_bank; do not edit.\n\n\
use super::wavetable_bank::WavetableProfile;\n\n",
    );
    let profile_id = manifest["profile_id"]
        .as_str()
        .ok_or_else(|| MeasuredBankError::Message("manifest profile_id missing".into()))?;
    let target_id = manifest["target_id"]
        .as_str()
        .ok_or_else(|| MeasuredBankError::Message("manifest target_id missing".into()))?;
    let fnv = manifest["bank_binary"]["fnv1a32"]
        .as_u64()
        .ok_or_else(|| MeasuredBankError::Message("manifest fnv1a32 missing".into()))?;
    let sample_count = manifest["bank_binary"]["sample_count"]
        .as_u64()
        .ok_or_else(|| MeasuredBankError::Message("manifest sample_count missing".into()))?;
    let reference_rate = manifest["layout"]["reference_sample_rate_hz"]
        .as_f64()
        .ok_or_else(|| MeasuredBankError::Message("manifest reference rate missing".into()))?;
    source.push_str(&format!(
        "pub const WAVETABLE_BANK_PROFILE_ID: &str = {profile_id:?};\n\
pub const WAVETABLE_BANK_TARGET_ID: &str = {target_id:?};\n\
pub const WAVETABLE_BANK_MANIFEST_SHA256: &str = {manifest_checksum:?};\n\
pub const WAVETABLE_BANK_FNV1A32: u32 = 0x{fnv:08x};\n\
pub const WAVETABLE_BANK_SAMPLE_COUNT: usize = {sample_count};\n\
pub const WAVETABLE_BANK_TABLE_LENGTH: usize = {TABLE_LENGTH};\n\
pub const WAVETABLE_BANK_REFERENCE_SAMPLE_RATE_HZ: f32 = {reference_rate:.9e}_f32;\n\n"
    ));
    for (waveform, prefix) in [
        ("saw", "SAW"),
        ("triangle", "TRIANGLE"),
        ("pulse50", "PULSE"),
    ] {
        let frequencies = manifest["waveforms"][waveform]["training_frequencies_hz"]
            .as_array()
            .ok_or_else(|| MeasuredBankError::Message(format!("{waveform} frequencies missing")))?;
        source.push_str(&format!(
            "pub const WAVETABLE_{prefix}_FREQUENCIES_HZ: [f32; {}] = [\n",
            frequencies.len()
        ));
        for value in frequencies {
            source.push_str(&format!(
                "    {:.9e}_f32,\n",
                value
                    .as_f64()
                    .ok_or_else(|| MeasuredBankError::Message(format!(
                        "{waveform} frequency is not numeric"
                    )))?
            ));
        }
        let maximum = manifest["waveforms"][waveform]["maximum_supported_frequency_hz"]
            .as_f64()
            .ok_or_else(|| MeasuredBankError::Message(format!("{waveform} maximum missing")))?;
        source.push_str(&format!(
            "];\n\npub const WAVETABLE_{prefix}_MAXIMUM_HZ: f32 = {maximum:.9e}_f32;\n\n"
        ));
    }
    source.push_str(
        "pub static PROPHET5_WAVETABLE_BANK_PROFILE: WavetableProfile =\n\
    WavetableProfile {\n\
        id: WAVETABLE_BANK_PROFILE_ID,\n\
        target_id: WAVETABLE_BANK_TARGET_ID,\n\
        manifest_sha256: WAVETABLE_BANK_MANIFEST_SHA256,\n\
        fnv1a32: WAVETABLE_BANK_FNV1A32,\n\
        sample_count: WAVETABLE_BANK_SAMPLE_COUNT,\n\
        table_length: WAVETABLE_BANK_TABLE_LENGTH,\n\
        reference_sample_rate_hz: WAVETABLE_BANK_REFERENCE_SAMPLE_RATE_HZ,\n\
        saw_hz: &WAVETABLE_SAW_FREQUENCIES_HZ,\n\
        triangle_hz: &WAVETABLE_TRIANGLE_FREQUENCIES_HZ,\n\
        pulse_hz: &WAVETABLE_PULSE_FREQUENCIES_HZ,\n\
        saw_max_hz: WAVETABLE_SAW_MAXIMUM_HZ,\n\
        triangle_max_hz: WAVETABLE_TRIANGLE_MAXIMUM_HZ,\n\
        pulse_max_hz: WAVETABLE_PULSE_MAXIMUM_HZ,\n\
    };\n",
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, source)?;
    Ok(())
}

struct WaveformTables {
    tables: Vec<f32>,
    training_indices: Vec<usize>,
    training_frequencies_hz: Vec<f64>,
    harmonic_limits: Vec<usize>,
    guard_frequencies_hz: Vec<f64>,
    maximum_supported_frequency_hz: f64,
    adjacent_spectral_cosines: Vec<f64>,
}

fn waveform_tables(
    npz_path: &Path,
    reference_sample_rate_hz: f64,
) -> Result<WaveformTables, MeasuredBankError> {
    let file = File::open(npz_path)?;
    let mut reader = NpzReader::new(file).map_err(|err| MeasuredBankError::Npz(err.to_string()))?;
    let cycles: Array2<f32> = reader
        .by_name("median_cycles.npy")
        .or_else(|_| reader.by_name("median_cycles"))
        .map_err(|err| MeasuredBankError::Npz(format!("median_cycles: {err}")))?;
    let frequencies: ndarray::Array1<f64> = reader
        .by_name("measured_frequency_hz.npy")
        .or_else(|_| reader.by_name("measured_frequency_hz"))
        .map_err(|err| MeasuredBankError::Npz(format!("measured_frequency_hz: {err}")))?;
    let roles: ndarray::Array1<u8> = reader
        .by_name("role.npy")
        .or_else(|_| reader.by_name("role"))
        .map_err(|err| MeasuredBankError::Npz(format!("role: {err}")))?;

    if cycles.nrows() != frequencies.len() || cycles.nrows() != roles.len() {
        return Err(MeasuredBankError::Message(format!(
            "{}: shape mismatch cycles={} freq={} role={}",
            npz_path.display(),
            cycles.nrows(),
            frequencies.len(),
            roles.len()
        )));
    }
    if cycles.ncols() != TABLE_LENGTH {
        return Err(MeasuredBankError::Message(format!(
            "{}: expected {TABLE_LENGTH} phase bins, got {}",
            npz_path.display(),
            cycles.ncols()
        )));
    }
    if cycles.iter().any(|sample| !sample.is_finite()) {
        return Err(MeasuredBankError::Message(format!(
            "{}: median_cycles contains non-finite samples",
            npz_path.display()
        )));
    }
    if frequencies
        .iter()
        .any(|frequency| !frequency.is_finite() || *frequency <= 0.0)
        || frequencies
            .iter()
            .zip(frequencies.iter().skip(1))
            .any(|(left, right)| right <= left)
    {
        return Err(MeasuredBankError::Message(format!(
            "{}: measured frequencies must be finite, positive, and strictly increasing",
            npz_path.display()
        )));
    }

    let training_indices: Vec<usize> = roles
        .iter()
        .enumerate()
        .filter_map(|(index, role)| (*role == ROLE_TRAINING).then_some(index))
        .collect();
    if training_indices.is_empty() {
        return Err(MeasuredBankError::Message(format!(
            "{}: no training rows",
            npz_path.display()
        )));
    }

    let adjacent_spectral_cosines = adjacent_spectral_cosines(&cycles, &training_indices)?;
    if let Some((ordinal, score)) = adjacent_spectral_cosines
        .iter()
        .copied()
        .enumerate()
        .find(|(_, score)| *score < MIN_ADJACENT_SPECTRAL_COSINE)
    {
        let left = training_indices[ordinal];
        let right = training_indices[ordinal + 1];
        return Err(MeasuredBankError::Message(format!(
            "{}: incoherent adjacent training cycles at rows {left}/{right} ({:.3} Hz/{:.3} Hz): spectral cosine {score:.4} < {MIN_ADJACENT_SPECTRAL_COSINE:.2}; reject capture before bank generation",
            npz_path.display(),
            frequencies[left],
            frequencies[right]
        )));
    }

    let maximum_supported_frequency_hz = frequencies
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let mut tables = Vec::with_capacity(training_indices.len() * TABLE_LENGTH);
    let mut training_frequencies_hz = Vec::with_capacity(training_indices.len());
    let mut harmonic_limits = Vec::with_capacity(training_indices.len());
    let mut guard_frequencies_hz = Vec::with_capacity(training_indices.len());

    for (ordinal, &index) in training_indices.iter().enumerate() {
        let guard_frequency = if ordinal + 1 < training_indices.len() {
            frequencies[training_indices[ordinal + 1]]
        } else {
            maximum_supported_frequency_hz
        };
        if guard_frequency <= 0.0 {
            return Err(MeasuredBankError::Message(format!(
                "{}: non-positive guard frequency at training ordinal {ordinal}",
                npz_path.display()
            )));
        }
        let row = cycles.row(index);
        let cycle: Vec<f32> = row.iter().copied().collect();
        let (table, harmonic_limit) =
            pitch_safe_table(&cycle, guard_frequency, reference_sample_rate_hz)?;
        tables.extend_from_slice(&table);
        training_frequencies_hz.push(frequencies[index]);
        harmonic_limits.push(harmonic_limit);
        guard_frequencies_hz.push(guard_frequency);
    }

    Ok(WaveformTables {
        tables,
        training_indices,
        training_frequencies_hz,
        harmonic_limits,
        guard_frequencies_hz,
        maximum_supported_frequency_hz,
        adjacent_spectral_cosines,
    })
}

fn adjacent_spectral_cosines(
    cycles: &Array2<f32>,
    training_indices: &[usize],
) -> Result<Vec<f64>, MeasuredBankError> {
    let spectra: Vec<Vec<f64>> = training_indices
        .iter()
        .map(|&index| magnitude_spectrum(&cycles.row(index).iter().copied().collect::<Vec<_>>()))
        .collect();
    spectra
        .windows(2)
        .map(|pair| {
            let dot = pair[0]
                .iter()
                .zip(&pair[1])
                .map(|(left, right)| left * right)
                .sum::<f64>();
            let left_norm = pair[0]
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
            let right_norm = pair[1]
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
            if left_norm <= 1e-20 || right_norm <= 1e-20 {
                return Err(MeasuredBankError::Message(
                    "training cycle has no usable harmonic energy".to_string(),
                ));
            }
            Ok((dot / (left_norm * right_norm)).clamp(0.0, 1.0))
        })
        .collect()
}

fn magnitude_spectrum(cycle: &[f32]) -> Vec<f64> {
    let mut buffer: Vec<Complex<f64>> = cycle
        .iter()
        .map(|sample| Complex::new(f64::from(*sample), 0.0))
        .collect();
    let mut planner = FftPlanner::<f64>::new();
    planner.plan_fft_forward(TABLE_LENGTH).process(&mut buffer);
    // Ignore DC and compare enough harmonics to expose gross pitch-dependent
    // coloration while remaining insensitive to phase alignment.
    buffer[1..=256].iter().map(|bin| bin.norm()).collect()
}

pub fn pitch_safe_table(
    cycle: &[f32],
    guard_frequency_hz: f64,
    reference_sample_rate_hz: f64,
) -> Result<(Vec<f32>, usize), MeasuredBankError> {
    if cycle.len() != TABLE_LENGTH {
        return Err(MeasuredBankError::Message(format!(
            "cycle length {} != {TABLE_LENGTH}",
            cycle.len()
        )));
    }
    let mut buffer: Vec<Complex<f64>> = cycle
        .iter()
        .map(|sample| Complex {
            re: f64::from(*sample),
            im: 0.0,
        })
        .collect();
    let mut planner = FftPlanner::<f64>::new();
    planner.plan_fft_forward(TABLE_LENGTH).process(&mut buffer);

    let spectrum_len = TABLE_LENGTH / 2 + 1;
    let harmonic_cap = (TABLE_LENGTH / 2).saturating_sub(1);
    let nyquist_cap =
        ((NYQUIST_GUARD * reference_sample_rate_hz) / guard_frequency_hz).floor() as usize;
    let harmonic_limit = spectrum_len
        .saturating_sub(1)
        .min(harmonic_cap)
        .min(nyquist_cap);

    for bin in (harmonic_limit + 1)..spectrum_len {
        buffer[bin] = Complex::new(0.0, 0.0);
    }
    // Hermitian symmetry for irfft via full FFT inverse
    for bin in 1..TABLE_LENGTH / 2 {
        let mirror = TABLE_LENGTH - bin;
        if bin > harmonic_limit {
            buffer[mirror] = Complex::new(0.0, 0.0);
        } else {
            buffer[mirror] = buffer[bin].conj();
        }
    }

    planner.plan_fft_inverse(TABLE_LENGTH).process(&mut buffer);
    let scale = TABLE_LENGTH as f64;
    let table: Vec<f32> = buffer
        .iter()
        .map(|value| (value.re / scale) as f32)
        .collect();
    Ok((table, harmonic_limit))
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
        .map_err(|err| MeasuredBankError::Npz(err.to_string()))?;
    npz.add_array(
        "measured_frequency_hz",
        &Array1::from_vec(frequencies.to_vec()),
    )
    .map_err(|err| MeasuredBankError::Npz(err.to_string()))?;
    npz.add_array("role", &Array1::from_vec(roles.to_vec()))
        .map_err(|err| MeasuredBankError::Npz(err.to_string()))?;
    npz.finish()
        .map_err(|err| MeasuredBankError::Npz(err.to_string()))?;
    Ok(())
}

fn fnv1a32(values: &[f32]) -> u32 {
    let mut result = 0x811c_9dc5_u32;
    for sample in values {
        for byte in sample.to_le_bytes() {
            result ^= u32::from(byte);
            result = result.wrapping_mul(0x0100_0193);
        }
    }
    result
}

fn sha256_file(path: &Path) -> Result<String, MeasuredBankError> {
    let bytes = fs::read(path)?;
    Ok(hex_encode(&Sha256::digest(bytes)))
}

fn manifest_content_sha256(manifest: &serde_json::Value) -> Result<String, MeasuredBankError> {
    let bytes = serde_json::to_vec(manifest)?;
    Ok(hex_encode(&Sha256::digest(bytes)))
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        BankRequest, ROLE_TRAINING, TABLE_LENGTH, WAVEFORMS, build_bank, pitch_safe_table,
        write_synthetic_npz,
    };
    use ndarray::Array2;
    use tempfile::tempdir;

    #[test]
    fn pitch_safe_table_is_finite() {
        let cycle: Vec<f32> = (0..TABLE_LENGTH)
            .map(|index| {
                let phase = index as f64 / TABLE_LENGTH as f64;
                (2.0 * phase - 1.0) as f32
            })
            .collect();
        let (table, limit) = pitch_safe_table(&cycle, 440.0, 96_000.0).unwrap();
        assert_eq!(table.len(), TABLE_LENGTH);
        assert!(table.iter().all(|sample| sample.is_finite()));
        assert!(limit > 0);
    }

    #[test]
    fn builds_bank_from_synthetic_npz() {
        let dir = tempdir().unwrap();
        let derived = dir.path().join("derived");
        std::fs::create_dir_all(&derived).unwrap();
        let output = dir.path().join("banks");
        let rust_profile = dir.path().join("profile.rs");

        for waveform in WAVEFORMS {
            let mut cycles = Array2::<f32>::zeros((4, TABLE_LENGTH));
            for row in 0..4 {
                for bin in 0..TABLE_LENGTH {
                    let phase = bin as f64 / TABLE_LENGTH as f64;
                    cycles[[row, bin]] = ((row + 1) as f32) * (2.0 * phase - 1.0) as f32 * 0.1;
                }
            }
            let frequencies = [110.0, 123.0, 138.0, 155.0];
            let roles = [ROLE_TRAINING, 1, ROLE_TRAINING, 2];
            write_synthetic_npz(
                &derived.join(format!("{waveform}-cycles-v1.npz")),
                &cycles,
                &frequencies,
                &roles,
            )
            .unwrap();
        }

        let result = build_bank(&BankRequest {
            derived_root: derived,
            output_dir: output.clone(),
            profile_id: "test-measured-bank-v1".into(),
            target_id: "test-target".into(),
            reference_sample_rate_hz: 96_000.0,
            rust_profile_path: Some(rust_profile.clone()),
        })
        .unwrap();

        assert_eq!(result.pitch_count_per_waveform, 2);
        assert_eq!(result.sample_count, WAVEFORMS.len() * 2 * TABLE_LENGTH);
        assert!(result.binary_path.is_file());
        assert!(result.manifest_path.is_file());
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(result.manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["profile_id"], "test-measured-bank-v1");
        assert!(manifest["manifest_content_sha256"].as_str().unwrap().len() == 64);
        assert!(manifest["bank_binary"]["fnv1a32"].as_u64().is_some());
        assert!(output.join("test-measured-bank-v1.f32le").is_file());
        let source = std::fs::read_to_string(rust_profile).unwrap();
        assert!(source.contains("WAVETABLE_BANK_REFERENCE_SAMPLE_RATE_HZ"));
        assert!(source.contains("reference_sample_rate_hz:"));
    }

    #[test]
    fn rejects_incoherent_training_cycles() {
        let dir = tempdir().unwrap();
        let derived = dir.path().join("derived");
        std::fs::create_dir_all(&derived).unwrap();
        for waveform in WAVEFORMS {
            let mut cycles = Array2::<f32>::zeros((3, TABLE_LENGTH));
            for bin in 0..TABLE_LENGTH {
                let phase = bin as f64 / TABLE_LENGTH as f64;
                cycles[[0, bin]] = (2.0 * phase - 1.0) as f32;
                cycles[[1, bin]] = (2.0 * std::f64::consts::PI * 97.0 * phase).sin() as f32;
                cycles[[2, bin]] = (2.0 * phase - 1.0) as f32;
            }
            write_synthetic_npz(
                &derived.join(format!("{waveform}-cycles-v1.npz")),
                &cycles,
                &[110.0, 123.0, 138.0],
                &[ROLE_TRAINING; 3],
            )
            .unwrap();
        }
        let error = build_bank(&BankRequest {
            derived_root: derived,
            output_dir: dir.path().join("banks"),
            profile_id: "bad-bank".into(),
            target_id: "bad-target".into(),
            reference_sample_rate_hz: 48_000.0,
            rust_profile_path: None,
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("incoherent adjacent training cycles")
        );
    }
}
