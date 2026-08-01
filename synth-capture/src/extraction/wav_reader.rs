use std::path::Path;

use crate::{audio::wav::read_float_wav, extraction::ExtractionError};

#[derive(Clone, Debug, PartialEq)]
pub struct FloatWav {
    pub sample_rate_hz: u32,
    pub samples: Vec<f32>,
}

pub fn read_mono_float32(path: &Path) -> Result<FloatWav, ExtractionError> {
    let (sample_rate_hz, samples) =
        read_float_wav(path).map_err(|err| ExtractionError::Wav(err.to_string()))?;
    if samples.is_empty() {
        return Err(ExtractionError::Wav(format!("{} is empty", path.display())));
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(ExtractionError::Wav(format!(
            "{} contains non-finite samples",
            path.display()
        )));
    }
    Ok(FloatWav {
        sample_rate_hz,
        samples,
    })
}

pub fn require_sample_rate(wav: &FloatWav, expected_hz: u32) -> Result<(), ExtractionError> {
    if wav.sample_rate_hz != expected_hz {
        return Err(ExtractionError::Wav(format!(
            "sample rate mismatch: expected {expected_hz}, got {}",
            wav.sample_rate_hz
        )));
    }
    Ok(())
}

pub fn require_frame_count(wav: &FloatWav, expected_frames: u64) -> Result<(), ExtractionError> {
    let actual = wav.samples.len() as u64;
    if actual != expected_frames {
        return Err(ExtractionError::Wav(format!(
            "frame count mismatch: expected {expected_frames}, got {actual}"
        )));
    }
    Ok(())
}
