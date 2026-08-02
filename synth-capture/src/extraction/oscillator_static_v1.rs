use std::{fs::File, path::Path};

use ndarray::{Array1, Array2};
use ndarray_npy::NpzWriter;
use num_complex::Complex;
use rustfft::FftPlanner;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    domain::{CaptureCase, CaseKind, FrequencyHz, OscillatorWaveform, ScientificRole},
    extraction::{
        CaptureExtractor, ExtractionError, ExtractionSummary, ExtractorDescriptor, wav_reader,
    },
    project::{CaptureProject, CaseStatus, sha256_file},
    protocols::{OSCILLATOR_STATIC_V1_ID, ProtocolDescriptor},
};

pub const EXTRACTOR_ID: &str = "oscillator-static-v1";
pub const EXTRACTOR_REVISION: u32 = 2;
pub const PHASE_BINS: usize = 2048;
pub const HARMONICS: usize = 256;
pub const MAX_CYCLES: usize = 1024;

const WAVEFORM_SLUGS: &[(OscillatorWaveform, &str)] = &[
    (OscillatorWaveform::Saw, "saw"),
    (OscillatorWaveform::Triangle, "triangle"),
    (OscillatorWaveform::Pulse, "pulse50"),
];

#[derive(Clone, Debug)]
pub struct PitchExtraction {
    pub median_cycle_raw: Vec<f32>,
    pub median_cycle_normalized: Vec<f32>,
    pub complex_harmonics: Vec<Complex<f32>>,
    pub frequency_hz: f64,
    pub raw_dc: f64,
    pub raw_rms: f64,
    pub raw_peak: f64,
    pub normalization_scale: f64,
    pub crest_factor: f64,
    pub duty_above_midpoint: f64,
    pub period_jitter_ppm: f64,
    pub cycle_amplitude_cv: f64,
    pub cycles_accepted: usize,
    pub cycles_rejected: usize,
}

#[derive(Clone, Debug, Default)]
pub struct OscillatorStaticExtractorV1;

impl OscillatorStaticExtractorV1 {
    pub fn extract_pitch(
        samples: &[f32],
        sample_rate_hz: f64,
        expected_frequency: Option<f64>,
    ) -> Result<PitchExtraction, ExtractionError> {
        extract_pitch(
            samples,
            sample_rate_hz,
            PHASE_BINS,
            HARMONICS,
            MAX_CYCLES,
            expected_frequency,
        )
    }
}

impl CaptureExtractor for OscillatorStaticExtractorV1 {
    fn descriptor(&self) -> ExtractorDescriptor {
        ExtractorDescriptor {
            id: EXTRACTOR_ID.to_string(),
            revision: EXTRACTOR_REVISION.to_string(),
        }
    }

    fn supports(&self, protocol: &ProtocolDescriptor) -> bool {
        protocol.id == OSCILLATOR_STATIC_V1_ID
    }

    fn extract(
        &self,
        project: &CaptureProject,
        output: &Path,
    ) -> Result<ExtractionSummary, ExtractionError> {
        if !self.supports(&project.document().protocol) {
            return Err(ExtractionError::UnsupportedProtocol(
                project.document().protocol.id.clone(),
            ));
        }
        ensure_project_complete(project)?;

        std::fs::create_dir_all(output).map_err(|err| ExtractionError::Io(err.to_string()))?;

        let sample_rate = project.document().protocol_config.sample_rate.get();
        let mut waveform_files = Vec::new();
        let mut note_count = 0usize;
        let mut waveform_count = 0usize;

        for &(waveform, slug) in WAVEFORM_SLUGS {
            let mut cases: Vec<&CaptureCase> = project
                .document()
                .cases
                .iter()
                .filter(|case| {
                    case.kind == CaseKind::Stimulated && case.tags.waveform == Some(waveform)
                })
                .collect();
            cases.sort_by_key(|case| case.tags.note.map(|note| note.get()).unwrap_or(0));

            if cases.is_empty() {
                continue;
            }
            waveform_count += 1;

            let mut median_cycles = Array2::<f32>::zeros((cases.len(), PHASE_BINS));
            let mut median_cycles_raw = Array2::<f32>::zeros((cases.len(), PHASE_BINS));
            let mut harmonics = Array2::<Complex<f32>>::zeros((cases.len(), HARMONICS + 1));
            let mut measured_frequency_hz = Array1::<f64>::zeros(cases.len());
            let mut nominal_midi = Array1::<f64>::zeros(cases.len());
            let mut raw_dc = Array1::<f64>::zeros(cases.len());
            let mut raw_rms = Array1::<f64>::zeros(cases.len());
            let mut raw_peak = Array1::<f64>::zeros(cases.len());
            let mut normalization_scale = Array1::<f64>::zeros(cases.len());
            let mut period_jitter_ppm = Array1::<f64>::zeros(cases.len());
            let mut cycle_amplitude_cv = Array1::<f64>::zeros(cases.len());
            let mut role_codes = Array1::<u8>::zeros(cases.len());
            let mut pitch_records = Vec::with_capacity(cases.len());
            let mut wav_fingerprints = Vec::with_capacity(cases.len());

            for (row, case) in cases.iter().enumerate() {
                let entry = project
                    .state()
                    .cases
                    .get(&case.id)
                    .ok_or_else(|| ExtractionError::Incomplete(case.id.clone()))?;
                let audio_path = project.final_audio_path(&case.id);
                let wav = wav_reader::read_mono_float32(&audio_path)?;
                wav_reader::require_sample_rate(&wav, sample_rate)?;
                if let Some(frames) = entry.exact_frames {
                    wav_reader::require_frame_count(&wav, frames)?;
                }
                let wav_sha = entry
                    .wav_sha256
                    .clone()
                    .ok_or_else(|| ExtractionError::Incomplete(case.id.clone()))?;
                let actual_sha =
                    sha256_file(&audio_path).map_err(|err| ExtractionError::Io(err.to_string()))?;
                if actual_sha != wav_sha {
                    return Err(ExtractionError::ChecksumMismatch {
                        case_id: case.id.clone(),
                        expected: wav_sha,
                        found: actual_sha,
                    });
                }

                let note = case
                    .tags
                    .note
                    .ok_or_else(|| ExtractionError::Message(format!("{} missing note", case.id)))?;
                let expected = case
                    .expected_fundamental_hz
                    .map(FrequencyHz::get)
                    .unwrap_or_else(|| note.frequency_hz().get());
                let extracted = extract_pitch(
                    &wav.samples,
                    f64::from(sample_rate),
                    PHASE_BINS,
                    HARMONICS,
                    MAX_CYCLES,
                    Some(expected),
                )
                .map_err(|err| ExtractionError::Case {
                    case_id: case.id.clone(),
                    message: err.to_string(),
                })?;

                for (bin, sample) in extracted.median_cycle_normalized.iter().enumerate() {
                    median_cycles[[row, bin]] = *sample;
                }
                for (bin, sample) in extracted.median_cycle_raw.iter().enumerate() {
                    median_cycles_raw[[row, bin]] = *sample;
                }
                for (harmonic, value) in extracted.complex_harmonics.iter().enumerate() {
                    harmonics[[row, harmonic]] = *value;
                }
                measured_frequency_hz[row] = extracted.frequency_hz;
                nominal_midi[row] = f64::from(note.get());
                raw_dc[row] = extracted.raw_dc;
                raw_rms[row] = extracted.raw_rms;
                raw_peak[row] = extracted.raw_peak;
                normalization_scale[row] = extracted.normalization_scale;
                period_jitter_ppm[row] = extracted.period_jitter_ppm;
                cycle_amplitude_cv[row] = extracted.cycle_amplitude_cv;
                role_codes[row] = role_code(case.role);
                wav_fingerprints.push(WavFingerprint {
                    case_id: case.id.clone(),
                    wav_sha256: wav_sha,
                });
                pitch_records.push(PitchSummaryRecord {
                    case_id: case.id.clone(),
                    pitch_index: row,
                    nominal_midi: note.get(),
                    role: role_name(case.role).to_string(),
                    expected_frequency_hz: expected,
                    measured_frequency_hz: extracted.frequency_hz,
                    pitch_error_cents: cents_error(extracted.frequency_hz, expected),
                    raw_dc: extracted.raw_dc,
                    raw_rms: extracted.raw_rms,
                    raw_peak: extracted.raw_peak,
                    normalization_scale: extracted.normalization_scale,
                    crest_factor: extracted.crest_factor,
                    duty_above_midpoint: extracted.duty_above_midpoint,
                    period_jitter_ppm: extracted.period_jitter_ppm,
                    cycle_amplitude_cv: extracted.cycle_amplitude_cv,
                    cycles_accepted: extracted.cycles_accepted,
                    cycles_rejected: extracted.cycles_rejected,
                    wav_sha256: wav_fingerprints[row].wav_sha256.clone(),
                });
            }

            note_count += cases.len();
            let npz_name = format!("{slug}-cycles-v1.npz");
            let summary_name = format!("{slug}-summary-v1.json");
            let npz_path = output.join(&npz_name);
            let summary_path = output.join(&summary_name);

            write_waveform_npz(
                &npz_path,
                &median_cycles,
                &median_cycles_raw,
                &harmonics,
                &measured_frequency_hz,
                &nominal_midi,
                &raw_dc,
                &raw_rms,
                &raw_peak,
                &normalization_scale,
                &period_jitter_ppm,
                &cycle_amplitude_cv,
                &role_codes,
            )?;

            let npz_sha256 =
                sha256_file(&npz_path).map_err(|err| ExtractionError::Io(err.to_string()))?;
            let summary = WaveformSummary {
                schema_version: 1,
                extractor_id: EXTRACTOR_ID.to_string(),
                extractor_revision: EXTRACTOR_REVISION,
                project_id: project.document().project_id.clone(),
                target_id: project.document().target.id.clone(),
                target_revision: project.document().target.revision.clone(),
                adapter_revision: project.document().target.adapter_revision.clone(),
                mapping_fingerprint: project.document().target.mapping_fingerprint.clone(),
                protocol_id: project.document().protocol.id.clone(),
                protocol_revision: project.document().protocol.revision.clone(),
                scientific_fingerprint: project.document().scientific_fingerprint.clone(),
                extractor_fingerprint: extractor_fingerprint(),
                waveform: slug.to_string(),
                sample_rate_hz: sample_rate,
                phase_bins: PHASE_BINS,
                harmonics_including_dc: HARMONICS + 1,
                max_cycles_per_pitch: MAX_CYCLES,
                npz_file: npz_name.clone(),
                npz_sha256,
                wav_fingerprints,
                pitches: pitch_records,
            };
            let summary_text = serde_json::to_string_pretty(&summary)
                .map_err(|err| ExtractionError::Io(err.to_string()))?
                + "\n";
            std::fs::write(&summary_path, summary_text)
                .map_err(|err| ExtractionError::Io(err.to_string()))?;

            waveform_files.push(npz_path);
            waveform_files.push(summary_path);
        }

        Ok(ExtractionSummary {
            project_id: project.document().project_id.clone(),
            output_dir: output.to_path_buf(),
            waveform_count,
            note_count,
            files: waveform_files,
            extractor_fingerprint: extractor_fingerprint(),
            scientific_fingerprint: project.document().scientific_fingerprint.clone(),
        })
    }
}

pub fn extract_pitch(
    samples: &[f32],
    sample_rate_hz: f64,
    phase_bins: usize,
    harmonics: usize,
    max_cycles: usize,
    expected_frequency: Option<f64>,
) -> Result<PitchExtraction, ExtractionError> {
    let (cycles, periods, rejected) = robust_cycle_set(
        samples,
        sample_rate_hz,
        phase_bins,
        max_cycles,
        expected_frequency,
    )?;
    let median_cycle_raw = median_cycle(&cycles);
    let spectrum = rfft_normalized(&median_cycle_raw);
    let harmonic_count = (harmonics + 1).min(spectrum.len());
    let complex_harmonics = spectrum[..harmonic_count].to_vec();

    let frequency_hz = sample_rate_hz / median_f64(&periods);
    let raw_dc = mean_f64(&median_cycle_raw);
    let raw_rms = rms_f64(&median_cycle_raw);
    let raw_peak = peak_abs_f64(&median_cycle_raw);
    if raw_peak <= 0.0 {
        return Err(ExtractionError::Message(
            "median cycle peak magnitude is zero".to_string(),
        ));
    }
    let normalization_scale = raw_peak;
    let median_cycle_normalized: Vec<f32> = median_cycle_raw
        .iter()
        .map(|sample| ((f64::from(*sample) - raw_dc) / normalization_scale) as f32)
        .collect();

    let midpoint = {
        let min = median_cycle_raw
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let max = median_cycle_raw
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        f64::from((max + min) * 0.5)
    };
    let duty_above_midpoint = median_cycle_raw
        .iter()
        .filter(|sample| f64::from(**sample) > midpoint)
        .count() as f64
        / median_cycle_raw.len() as f64;

    let mut peak_to_peaks = Vec::with_capacity(cycles.len());
    for cycle in &cycles {
        let min = cycle.iter().copied().fold(f32::INFINITY, f32::min);
        let max = cycle.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        peak_to_peaks.push(f64::from(max - min));
    }
    let ptp_mean = mean_slice_f64(&peak_to_peaks);
    let cycle_amplitude_cv = if ptp_mean <= f64::MIN_POSITIVE {
        0.0
    } else {
        std_slice_f64(&peak_to_peaks) / ptp_mean
    };
    let period_mean = mean_slice_f64(&periods);
    let period_jitter_ppm = if period_mean <= 0.0 {
        0.0
    } else {
        std_slice_f64(&periods) / period_mean * 1_000_000.0
    };

    Ok(PitchExtraction {
        median_cycle_raw,
        median_cycle_normalized,
        complex_harmonics,
        frequency_hz,
        raw_dc,
        raw_rms,
        raw_peak,
        normalization_scale,
        crest_factor: raw_peak / raw_rms.max(f64::MIN_POSITIVE),
        duty_above_midpoint,
        period_jitter_ppm,
        cycle_amplitude_cv,
        cycles_accepted: cycles.len(),
        cycles_rejected: rejected,
    })
}

pub fn estimate_frequency(
    samples: &[f32],
    sample_rate_hz: f64,
    expected_frequency: Option<f64>,
) -> Result<f64, ExtractionError> {
    if samples.len() < 4 {
        return Err(ExtractionError::Message(
            "not enough samples for frequency estimate".to_string(),
        ));
    }
    let mean = mean_f32(samples);
    let mut fft_size = 1usize;
    while fft_size * 2 <= samples.len() {
        fft_size *= 2;
    }
    if fft_size < 4 {
        return Err(ExtractionError::Message(
            "FFT size too small for frequency estimate".to_string(),
        ));
    }

    let mut buffer: Vec<Complex<f64>> = (0..fft_size)
        .map(|index| {
            let window = if fft_size == 1 {
                1.0
            } else {
                0.5 - 0.5
                    * (2.0 * std::f64::consts::PI * index as f64 / (fft_size as f64 - 1.0)).cos()
            };
            Complex {
                re: (f64::from(samples[index]) - mean) * window,
                im: 0.0,
            }
        })
        .collect();
    let mut planner = FftPlanner::<f64>::new();
    planner.plan_fft_forward(fft_size).process(&mut buffer);

    let magnitude_len = fft_size / 2 + 1;
    let magnitudes: Vec<f64> = buffer[..magnitude_len].iter().map(|c| c.norm()).collect();
    let bin_hz = sample_rate_hz / fft_size as f64;
    let (first_bin, last_bin) = match expected_frequency {
        None => {
            let first = (10.0 / bin_hz).floor() as usize;
            let last = ((sample_rate_hz * 0.45) / bin_hz).floor() as usize;
            (first.max(1), last.min(magnitudes.len().saturating_sub(1)))
        }
        Some(expected) => {
            let first = ((expected * 0.8) / bin_hz).floor() as usize;
            let last = ((expected * 1.2) / bin_hz).floor() as usize + 1;
            (first.max(1), last.min(magnitudes.len().saturating_sub(1)))
        }
    };
    if first_bin > last_bin {
        return Err(ExtractionError::Message(
            "frequency search window is empty".to_string(),
        ));
    }
    let mut peak_bin = first_bin;
    let mut peak_mag = magnitudes[first_bin];
    for bin in first_bin..=last_bin {
        if magnitudes[bin] > peak_mag {
            peak_mag = magnitudes[bin];
            peak_bin = bin;
        }
    }
    let mut offset = 0.0;
    if peak_bin > 0 && peak_bin + 1 < magnitudes.len() {
        let left = magnitudes[peak_bin - 1].max(1e-30).ln();
        let center = magnitudes[peak_bin].max(1e-30).ln();
        let right = magnitudes[peak_bin + 1].max(1e-30).ln();
        let denominator = left - 2.0 * center + right;
        if denominator.abs() > 1e-20 {
            offset = (0.5 * (left - right) / denominator).clamp(-0.5, 0.5);
        }
    }
    Ok((peak_bin as f64 + offset) * bin_hz)
}

fn robust_cycle_set(
    samples: &[f32],
    sample_rate_hz: f64,
    phase_bins: usize,
    max_cycles: usize,
    expected_frequency: Option<f64>,
) -> Result<(Vec<Vec<f32>>, Vec<f64>, usize), ExtractionError> {
    let trim = ((sample_rate_hz * 0.25) as usize).min(samples.len() / 10);
    let end = if trim == 0 {
        samples.len()
    } else {
        samples.len().saturating_sub(trim)
    };
    if end <= trim + 4 {
        return Err(ExtractionError::Message(
            "trimmed audio too short".to_string(),
        ));
    }
    let working = &samples[trim..end];
    let frequency = estimate_frequency(working, sample_rate_hz, expected_frequency)?;
    if frequency <= 0.0 {
        return Err(ExtractionError::Message(
            "estimated frequency is non-positive".to_string(),
        ));
    }
    let period = sample_rate_hz / frequency;
    let crossings = phase_landmarks(working, period)?;
    let periods: Vec<f64> = crossings.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let median_period = median_f64(&periods);
    let mut valid_indices = Vec::new();
    let mut rejected = 0usize;
    for (index, period_samples) in periods.iter().enumerate() {
        if *period_samples > median_period * 0.8 && *period_samples < median_period * 1.2 {
            valid_indices.push(index);
        } else {
            rejected += 1;
        }
    }
    if valid_indices.len() < 3 {
        return Err(ExtractionError::Message(
            "too few stable cycles after period rejection".to_string(),
        ));
    }
    if valid_indices.len() > max_cycles {
        let selected = linspace_indices(valid_indices.len(), max_cycles);
        valid_indices = selected
            .into_iter()
            .map(|index| valid_indices[index])
            .collect();
    }

    let mut cycles = Vec::with_capacity(valid_indices.len());
    let mut accepted_periods = Vec::with_capacity(valid_indices.len());
    for &crossing_index in &valid_indices {
        let begin = crossings[crossing_index];
        let period_samples = periods[crossing_index];
        let mut cycle = vec![0.0f32; phase_bins];
        for (bin, sample) in cycle.iter_mut().enumerate() {
            let phase = bin as f64 / phase_bins as f64;
            let x = begin + phase * period_samples;
            *sample = interp_unit_grid(x, working) as f32;
        }
        cycles.push(cycle);
        accepted_periods.push(period_samples);
    }
    Ok((cycles, accepted_periods, rejected))
}

fn phase_landmarks(samples: &[f32], period: f64) -> Result<Vec<f64>, ExtractionError> {
    // A raw analogue-like waveform can cross its midpoint several times per
    // period.  Using the steepest raw crossing can therefore jump between
    // different intra-cycle features as pitch changes.  A half-period moving
    // average strongly suppresses the second and higher harmonics while
    // retaining the fundamental, giving one stable phase landmark per cycle.
    let phase_proxy = fundamental_phase_proxy(samples, period);
    let centered: Vec<f64> = {
        let median = median_f32(&phase_proxy);
        phase_proxy
            .iter()
            .map(|sample| f64::from(*sample) - median)
            .collect()
    };
    let crossings = upward_crossings(&phase_proxy)?;
    let crossing_indices: Vec<usize> = crossings
        .iter()
        .map(|value| value.floor() as usize)
        .collect();
    let slopes: Vec<f64> = crossing_indices
        .iter()
        .map(|&index| centered[index + 1] - centered[index])
        .collect();

    let mut first_candidates = Vec::new();
    for (index, crossing) in crossings.iter().enumerate() {
        if *crossing < period * 1.25 {
            first_candidates.push(index);
        }
    }
    if first_candidates.is_empty() {
        return Err(ExtractionError::Message(
            "could not locate an initial phase landmark".to_string(),
        ));
    }
    let first = first_candidates
        .iter()
        .copied()
        .max_by(|&left, &right| {
            slopes[left]
                .partial_cmp(&slopes[right])
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap();

    let mut landmarks = vec![crossings[first]];
    let mut predicted = landmarks[0] + period;
    while predicted < samples.len() as f64 - period * 0.5 {
        let mut candidates = Vec::new();
        for (index, crossing) in crossings.iter().enumerate() {
            if *crossing >= predicted - period * 0.3 && *crossing <= predicted + period * 0.3 {
                candidates.push(index);
            }
        }
        let landmark = if candidates.is_empty() {
            predicted
        } else {
            let selected = candidates
                .iter()
                .copied()
                .min_by(|&left, &right| {
                    (crossings[left] - predicted)
                        .abs()
                        .partial_cmp(&(crossings[right] - predicted).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap();
            crossings[selected]
        };
        if landmark > landmarks[landmarks.len() - 1] + period * 0.5 {
            landmarks.push(landmark);
        }
        predicted = landmarks[landmarks.len() - 1] + period;
    }
    if landmarks.len() < 4 {
        return Err(ExtractionError::Message(
            "could not identify enough phase landmarks".to_string(),
        ));
    }
    Ok(landmarks)
}

fn fundamental_phase_proxy(samples: &[f32], period: f64) -> Vec<f32> {
    let mut window = (period * 0.5).round().max(1.0) as usize;
    window = window.min(samples.len().max(1));
    let left = window / 2;
    let right = window - left;
    let mut prefix = Vec::with_capacity(samples.len() + 1);
    prefix.push(0.0f64);
    for sample in samples {
        prefix.push(prefix.last().copied().unwrap() + f64::from(*sample));
    }
    (0..samples.len())
        .map(|index| {
            let begin = index.saturating_sub(left);
            let end = (index + right).min(samples.len());
            ((prefix[end] - prefix[begin]) / (end - begin) as f64) as f32
        })
        .collect()
}

fn upward_crossings(samples: &[f32]) -> Result<Vec<f64>, ExtractionError> {
    let median = median_f32(samples);
    let centered: Vec<f64> = samples
        .iter()
        .map(|sample| f64::from(*sample) - median)
        .collect();
    let mut crossings = Vec::new();
    let eps = f64::from(f32::EPSILON);
    for index in 0..centered.len().saturating_sub(1) {
        if centered[index] <= 0.0 && centered[index + 1] > 0.0 {
            let left = centered[index];
            let right = centered[index + 1];
            let denom = (right - left).max(eps);
            let fraction = (-left / denom).clamp(0.0, 1.0);
            crossings.push(index as f64 + fraction);
        }
    }
    if crossings.len() < 3 {
        return Err(ExtractionError::Message(
            "could not find enough upward crossings".to_string(),
        ));
    }
    Ok(crossings)
}

fn median_cycle(cycles: &[Vec<f32>]) -> Vec<f32> {
    let bins = cycles[0].len();
    let mut out = vec![0.0f32; bins];
    let mut column = vec![0.0f64; cycles.len()];
    for bin in 0..bins {
        for (row, cycle) in cycles.iter().enumerate() {
            column[row] = f64::from(cycle[bin]);
        }
        out[bin] = median_f64(&column) as f32;
    }
    out
}

fn rfft_normalized(cycle: &[f32]) -> Vec<Complex<f32>> {
    let n = cycle.len();
    let mut buffer: Vec<Complex<f64>> = cycle
        .iter()
        .map(|sample| Complex {
            re: f64::from(*sample),
            im: 0.0,
        })
        .collect();
    let mut planner = FftPlanner::<f64>::new();
    planner.plan_fft_forward(n).process(&mut buffer);
    let scale = n as f64;
    buffer[..n / 2 + 1]
        .iter()
        .map(|value| Complex {
            re: (value.re / scale) as f32,
            im: (value.im / scale) as f32,
        })
        .collect()
}

fn interp_unit_grid(x: f64, fp: &[f32]) -> f64 {
    if fp.is_empty() {
        return 0.0;
    }
    if x <= 0.0 {
        return f64::from(fp[0]);
    }
    let last = (fp.len() - 1) as f64;
    if x >= last {
        return f64::from(fp[fp.len() - 1]);
    }
    let lo = x.floor() as usize;
    let hi = lo + 1;
    let frac = x - lo as f64;
    let y0 = f64::from(fp[lo]);
    let y1 = f64::from(fp[hi]);
    y0 + (y1 - y0) * frac
}

fn linspace_indices(count: usize, take: usize) -> Vec<usize> {
    if take == 0 {
        return Vec::new();
    }
    if take == 1 {
        return vec![0];
    }
    (0..take)
        .map(|index| {
            let value = index as f64 * (count - 1) as f64 / (take - 1) as f64;
            value as usize
        })
        .collect()
}

fn ensure_project_complete(project: &CaptureProject) -> Result<(), ExtractionError> {
    let report = project
        .verify()
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    if !report.ok {
        let message = report
            .issues
            .iter()
            .map(|issue| {
                format!(
                    "{}{}",
                    issue
                        .case_id
                        .as_ref()
                        .map(|id| format!("{id}: "))
                        .unwrap_or_default(),
                    issue.message
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ExtractionError::Incomplete(message));
    }
    for case in &project.document().cases {
        let entry = project
            .state()
            .cases
            .get(&case.id)
            .ok_or_else(|| ExtractionError::Incomplete(case.id.clone()))?;
        if entry.status != CaseStatus::Complete {
            return Err(ExtractionError::Incomplete(format!(
                "case {} is {:?}",
                case.id, entry.status
            )));
        }
    }
    Ok(())
}

fn write_waveform_npz(
    path: &Path,
    median_cycles: &Array2<f32>,
    median_cycles_raw: &Array2<f32>,
    complex_harmonics: &Array2<Complex<f32>>,
    measured_frequency_hz: &Array1<f64>,
    nominal_midi: &Array1<f64>,
    raw_dc: &Array1<f64>,
    raw_rms: &Array1<f64>,
    raw_peak: &Array1<f64>,
    normalization_scale: &Array1<f64>,
    period_jitter_ppm: &Array1<f64>,
    cycle_amplitude_cv: &Array1<f64>,
    role: &Array1<u8>,
) -> Result<(), ExtractionError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| ExtractionError::Io(err.to_string()))?;
    }
    let file = File::create(path).map_err(|err| ExtractionError::Io(err.to_string()))?;
    let mut npz = NpzWriter::new(file);
    npz.add_array("median_cycles", median_cycles)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("median_cycles_raw", median_cycles_raw)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("complex_harmonics", complex_harmonics)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("measured_frequency_hz", measured_frequency_hz)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("nominal_midi", nominal_midi)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("raw_dc", raw_dc)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("raw_rms", raw_rms)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("raw_peak", raw_peak)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("normalization_scale", normalization_scale)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("period_jitter_ppm", period_jitter_ppm)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("cycle_amplitude_cv", cycle_amplitude_cv)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.add_array("role", role)
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    npz.finish()
        .map_err(|err| ExtractionError::Io(err.to_string()))?;
    Ok(())
}

fn extractor_fingerprint() -> String {
    let mut hasher = Sha256::new();
    hasher.update(EXTRACTOR_ID.as_bytes());
    hasher.update(EXTRACTOR_REVISION.to_le_bytes());
    hasher.update(PHASE_BINS.to_le_bytes());
    hasher.update(HARMONICS.to_le_bytes());
    hasher.update(MAX_CYCLES.to_le_bytes());
    let digest = hasher.finalize();
    let mut fingerprint = String::with_capacity(digest.len() * 2);
    for byte in digest {
        fingerprint.push_str(&format!("{byte:02x}"));
    }
    fingerprint
}

fn role_code(role: ScientificRole) -> u8 {
    match role {
        ScientificRole::Training => 0,
        ScientificRole::Validation => 1,
        ScientificRole::Test => 2,
        ScientificRole::GuardValidation => 3,
        ScientificRole::GuardTraining => 4,
        ScientificRole::NoiseFloor => 5,
    }
}

fn role_name(role: ScientificRole) -> &'static str {
    match role {
        ScientificRole::Training => "training",
        ScientificRole::Validation => "validation",
        ScientificRole::Test => "test",
        ScientificRole::GuardValidation => "guard_validation",
        ScientificRole::GuardTraining => "guard_training",
        ScientificRole::NoiseFloor => "noise_floor",
    }
}

fn cents_error(measured_hz: f64, expected_hz: f64) -> f64 {
    if measured_hz <= 0.0 || expected_hz <= 0.0 {
        return 0.0;
    }
    1200.0 * (measured_hz / expected_hz).log2()
}

fn mean_f32(samples: &[f32]) -> f64 {
    samples.iter().map(|sample| f64::from(*sample)).sum::<f64>() / samples.len() as f64
}

fn mean_f64(samples: &[f32]) -> f64 {
    mean_f32(samples)
}

fn mean_slice_f64(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn std_slice_f64(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = mean_slice_f64(values);
    let variance = values
        .iter()
        .map(|value| {
            let delta = *value - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

fn rms_f64(samples: &[f32]) -> f64 {
    let mean_sq = samples
        .iter()
        .map(|sample| {
            let value = f64::from(*sample);
            value * value
        })
        .sum::<f64>()
        / samples.len() as f64;
    mean_sq.sqrt()
}

fn peak_abs_f64(samples: &[f32]) -> f64 {
    samples
        .iter()
        .map(|sample| f64::from(*sample).abs())
        .fold(0.0, f64::max)
}

fn median_f32(samples: &[f32]) -> f64 {
    let mut values: Vec<f64> = samples.iter().map(|sample| f64::from(*sample)).collect();
    median_f64(&mut values)
}

fn median_f64(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    median_f64_mut(&mut sorted)
}

fn median_f64_mut(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        0.5 * (values[mid - 1] + values[mid])
    }
}

#[derive(Serialize)]
struct WaveformSummary {
    schema_version: u32,
    extractor_id: String,
    extractor_revision: u32,
    project_id: String,
    target_id: String,
    target_revision: String,
    adapter_revision: String,
    mapping_fingerprint: String,
    protocol_id: String,
    protocol_revision: String,
    scientific_fingerprint: String,
    extractor_fingerprint: String,
    waveform: String,
    sample_rate_hz: u32,
    phase_bins: usize,
    harmonics_including_dc: usize,
    max_cycles_per_pitch: usize,
    npz_file: String,
    npz_sha256: String,
    wav_fingerprints: Vec<WavFingerprint>,
    pitches: Vec<PitchSummaryRecord>,
}

#[derive(Serialize)]
struct WavFingerprint {
    case_id: String,
    wav_sha256: String,
}

#[derive(Serialize)]
struct PitchSummaryRecord {
    case_id: String,
    pitch_index: usize,
    nominal_midi: u8,
    role: String,
    expected_frequency_hz: f64,
    measured_frequency_hz: f64,
    pitch_error_cents: f64,
    raw_dc: f64,
    raw_rms: f64,
    raw_peak: f64,
    normalization_scale: f64,
    crest_factor: f64,
    duty_above_midpoint: f64,
    period_jitter_ppm: f64,
    cycle_amplitude_cv: f64,
    cycles_accepted: usize,
    cycles_rejected: usize,
    wav_sha256: String,
}

#[cfg(test)]
mod tests {
    use crate::extraction::oscillator_static_v1::{estimate_frequency, extract_pitch};

    use super::linspace_indices;

    #[test]
    fn estimates_non_bin_centered_sine() {
        let sample_rate = 48_000.0;
        let frequency = 440.0 * 2.0_f64.powf(0.1 / 12.0);
        let samples: Vec<f32> = (0..48_000)
            .map(|index| {
                (2.0 * std::f64::consts::PI * frequency * index as f64 / sample_rate).sin() as f32
            })
            .collect();
        let estimated = estimate_frequency(&samples, sample_rate, Some(440.0)).unwrap();
        let cents = 1200.0 * (estimated / frequency).log2();
        assert!(cents.abs() < 1.0, "cents={cents} estimated={estimated}");
    }

    #[test]
    fn extracts_stable_saw() {
        let sample_rate = 48_000.0;
        let frequency = 220.0;
        let samples: Vec<f32> = (0..96_000)
            .map(|index| {
                let phase = (index as f64 * frequency / sample_rate).fract();
                (2.0 * phase - 1.0) as f32
            })
            .collect();
        let result =
            extract_pitch(&samples, sample_rate, 2048, 256, 1024, Some(frequency)).unwrap();
        let cents = 1200.0 * (result.frequency_hz / frequency).log2();
        assert!(cents.abs() < 2.0, "cents={cents}");
        assert_eq!(result.median_cycle_raw.len(), 2048);
        assert!(result.raw_peak > 0.0);
        assert!((result.normalization_scale - result.raw_peak).abs() < 1e-12);
        let recon_peak = result
            .median_cycle_normalized
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0f32, f32::max);
        assert!((recon_peak - 1.0).abs() < 1e-3);
        assert!(result.cycles_accepted >= 3);
    }

    #[test]
    fn extracts_harmonic_rich_wave_without_switching_phase_landmarks() {
        let sample_rate = 48_000.0;
        let frequency = 200.0;
        let samples: Vec<f32> = (0..96_000)
            .map(|index| {
                let phase = 2.0 * std::f64::consts::PI * frequency * index as f64 / sample_rate;
                // The strong upper partials create several raw upward midpoint
                // crossings in each fundamental period.
                (phase.sin() + 0.9 * (3.0 * phase).sin() + 0.7 * (5.0 * phase).sin()) as f32
            })
            .collect();
        let result =
            extract_pitch(&samples, sample_rate, 2048, 256, 1024, Some(frequency)).unwrap();
        assert!(
            result.period_jitter_ppm < 100.0,
            "jitter={}",
            result.period_jitter_ppm
        );
        assert!(
            result.cycles_rejected <= 1,
            "rejected={}",
            result.cycles_rejected
        );
    }

    #[test]
    fn linspace_matches_numpy_truncation() {
        assert_eq!(linspace_indices(10, 4), vec![0, 3, 6, 9]);
        assert_eq!(linspace_indices(5, 5), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn rejects_constant_signal() {
        let samples = vec![0.25f32; 48_000];
        let err = extract_pitch(&samples, 48_000.0, 2048, 256, 1024, Some(440.0)).unwrap_err();
        assert!(
            err.to_string().contains("crossing") || err.to_string().contains("landmark"),
            "{err}"
        );
    }

    #[test]
    fn rejects_too_short_audio() {
        let samples = vec![0.0f32; 32];
        assert!(extract_pitch(&samples, 48_000.0, 2048, 256, 1024, Some(440.0)).is_err());
    }
}
