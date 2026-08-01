use num_complex::Complex;
use rustfft::FftPlanner;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::{CaseKind, FrequencyHz, PitchErrorCents};

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ValidationError {
    #[error("{0}")]
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignalMetrics {
    pub rms: f32,
    pub peak: f32,
    pub dc: f32,
    pub estimated_frequency_hz: Option<f64>,
    pub clipping: bool,
    pub overflow: bool,
    pub dc_warning: bool,
}

#[derive(Clone, Debug)]
pub struct ValidationInput<'a> {
    pub samples: &'a [f32],
    pub kind: CaseKind,
    pub expected_frames: u64,
    pub expected_fundamental_hz: Option<FrequencyHz>,
    pub permitted_pitch_error_cents: PitchErrorCents,
    pub sample_rate_hz: u32,
    pub overflow: bool,
}

pub fn validate_take(input: ValidationInput<'_>) -> Result<SignalMetrics, ValidationError> {
    if input.overflow {
        return Err(ValidationError::Failed(
            "audio callback overflow or error".to_string(),
        ));
    }
    if input.samples.len() as u64 != input.expected_frames {
        return Err(ValidationError::Failed(format!(
            "frame count mismatch: expected {}, got {}",
            input.expected_frames,
            input.samples.len()
        )));
    }
    if input.samples.iter().any(|sample| !sample.is_finite()) {
        return Err(ValidationError::Failed(
            "non-finite sample in recording".to_string(),
        ));
    }

    let metrics = measure(
        input.samples,
        input.sample_rate_hz,
        input.expected_fundamental_hz,
    );
    if metrics.clipping {
        return Err(ValidationError::Failed(
            "clipping detected (|sample| >= 0.999)".to_string(),
        ));
    }

    let rms_db = if metrics.rms > 0.0 {
        20.0 * metrics.rms.log10()
    } else {
        f32::NEG_INFINITY
    };

    match input.kind {
        CaseKind::Silence => {
            if rms_db > -72.0 {
                return Err(ValidationError::Failed(format!(
                    "silence RMS too high: {rms_db:.2} dBFS"
                )));
            }
        }
        CaseKind::Stimulated => {
            if rms_db < -48.0 {
                return Err(ValidationError::Failed(format!(
                    "stimulated RMS too low: {rms_db:.2} dBFS"
                )));
            }
            if let (Some(expected), Some(measured)) = (
                input.expected_fundamental_hz,
                metrics.estimated_frequency_hz,
            ) {
                let cents = 1200.0 * (measured / expected.get()).log2();
                if cents.abs() > input.permitted_pitch_error_cents.get() {
                    return Err(ValidationError::Failed(format!(
                        "pitch error {cents:.2} cents exceeds limit"
                    )));
                }
            } else {
                return Err(ValidationError::Failed(
                    "unable to estimate fundamental frequency".to_string(),
                ));
            }
        }
    }

    Ok(metrics)
}

fn measure(samples: &[f32], sample_rate_hz: u32, expected: Option<FrequencyHz>) -> SignalMetrics {
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut peak = 0.0f32;
    let mut clipping = false;
    for sample in samples {
        let value = f64::from(*sample);
        sum += value;
        sum_sq += value * value;
        let abs = sample.abs();
        if abs > peak {
            peak = abs;
        }
        if abs >= 0.999 {
            clipping = true;
        }
    }
    let n = samples.len().max(1) as f64;
    let dc = (sum / n) as f32;
    let rms = (sum_sq / n).sqrt() as f32;
    let estimated_frequency_hz =
        expected.and_then(|freq| estimate_frequency(samples, sample_rate_hz, freq.get()));
    SignalMetrics {
        rms,
        peak,
        dc,
        estimated_frequency_hz,
        clipping,
        overflow: false,
        dc_warning: dc.abs() > 0.1,
    }
}

fn estimate_frequency(samples: &[f32], sample_rate_hz: u32, expected_hz: f64) -> Option<f64> {
    if samples.len() < 64 || expected_hz <= 0.0 {
        return None;
    }
    let mut size = 1usize;
    while size < samples.len().min(16384) {
        size <<= 1;
    }
    size = size
        .max(256)
        .min(samples.len().next_power_of_two().min(16384));
    let mut buffer: Vec<Complex<f32>> = samples
        .iter()
        .take(size)
        .enumerate()
        .map(|(index, sample)| {
            let window =
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * index as f32 / (size as f32 - 1.0)).cos();
            Complex {
                re: sample * window,
                im: 0.0,
            }
        })
        .collect();
    while buffer.len() < size {
        buffer.push(Complex { re: 0.0, im: 0.0 });
    }

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(size);
    fft.process(&mut buffer);

    let bin_hz = f64::from(sample_rate_hz) / size as f64;
    let expected_bin = (expected_hz / bin_hz).round() as isize;
    let low = (expected_bin - (expected_bin / 5).max(2)).max(1);
    let high = (expected_bin + (expected_bin / 5).max(2)).min((size as isize / 2) - 2);
    if low >= high {
        return None;
    }

    let mut best_bin = low;
    let mut best_mag = 0.0f32;
    for bin in low..=high {
        let mag = buffer[bin as usize].norm_sqr();
        if mag > best_mag {
            best_mag = mag;
            best_bin = bin;
        }
    }

    let prev = buffer[(best_bin - 1) as usize].norm().max(1e-12).ln();
    let center = buffer[best_bin as usize].norm().max(1e-12).ln();
    let next = buffer[(best_bin + 1) as usize].norm().max(1e-12).ln();
    let denom = 2.0 * (2.0 * center - next - prev);
    let delta = if denom.abs() < 1e-9 {
        0.0
    } else {
        ((next - prev) / denom).clamp(-0.5, 0.5)
    };
    Some((best_bin as f64 + f64::from(delta)) * bin_hz)
}

#[cfg(test)]
mod tests {
    use crate::{
        domain::{CaseKind, FrequencyHz, PitchErrorCents},
        validation::{ValidationInput, validate_take},
    };

    #[test]
    fn accepts_clean_stimulated_sine() {
        let sample_rate = 48_000u32;
        let freq = 440.0f64;
        let frames = 4800usize;
        let samples: Vec<f32> = (0..frames)
            .map(|index| {
                (2.0 * std::f64::consts::PI * freq * f64::from(index as u32)
                    / f64::from(sample_rate))
                .sin() as f32
                    * 0.2
            })
            .collect();
        let metrics = validate_take(ValidationInput {
            samples: &samples,
            kind: CaseKind::Stimulated,
            expected_frames: frames as u64,
            expected_fundamental_hz: Some(FrequencyHz::try_new(freq).unwrap()),
            permitted_pitch_error_cents: PitchErrorCents::try_new(50.0).unwrap(),
            sample_rate_hz: sample_rate,
            overflow: false,
        })
        .unwrap();
        assert!(metrics.estimated_frequency_hz.unwrap() > 430.0);
    }

    #[test]
    fn rejects_loud_silence() {
        let samples = vec![0.01f32; 1000];
        let err = validate_take(ValidationInput {
            samples: &samples,
            kind: CaseKind::Silence,
            expected_frames: 1000,
            expected_fundamental_hz: None,
            permitted_pitch_error_cents: PitchErrorCents::try_new(50.0).unwrap(),
            sample_rate_hz: 48_000,
            overflow: false,
        })
        .unwrap_err();
        assert!(err.to_string().contains("silence"));
    }
}
