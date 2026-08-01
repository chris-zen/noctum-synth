use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use crate::audio::AudioError;

pub fn write_float_wav(path: &Path, sample_rate: u32, samples: &[f32]) -> Result<(), AudioError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| AudioError::Io(err.to_string()))?;
    }
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer =
        WavWriter::create(path, spec).map_err(|err| AudioError::Io(err.to_string()))?;
    for sample in samples {
        writer
            .write_sample(*sample)
            .map_err(|err| AudioError::Io(err.to_string()))?;
    }
    writer
        .finalize()
        .map_err(|err| AudioError::Io(err.to_string()))?;
    let file = File::options()
        .append(true)
        .open(path)
        .map_err(|err| AudioError::Io(err.to_string()))?;
    file.sync_all()
        .map_err(|err| AudioError::Io(err.to_string()))?;
    Ok(())
}

pub fn write_float_wav_streaming<I>(
    path: &Path,
    sample_rate: u32,
    samples: I,
) -> Result<u64, AudioError>
where
    I: IntoIterator<Item = f32>,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| AudioError::Io(err.to_string()))?;
    }
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let file = File::create(path).map_err(|err| AudioError::Io(err.to_string()))?;
    let mut buffered = BufWriter::new(file);
    let mut writer =
        WavWriter::new(&mut buffered, spec).map_err(|err| AudioError::Io(err.to_string()))?;
    let mut frames = 0u64;
    for sample in samples {
        writer
            .write_sample(sample)
            .map_err(|err| AudioError::Io(err.to_string()))?;
        frames += 1;
    }
    writer
        .finalize()
        .map_err(|err| AudioError::Io(err.to_string()))?;
    buffered
        .flush()
        .map_err(|err| AudioError::Io(err.to_string()))?;
    buffered
        .into_inner()
        .map_err(|err| AudioError::Io(err.to_string()))?
        .sync_all()
        .map_err(|err| AudioError::Io(err.to_string()))?;
    Ok(frames)
}

pub fn read_float_wav(path: &Path) -> Result<(u32, Vec<f32>), AudioError> {
    let mut reader = WavReader::open(path).map_err(|err| AudioError::Io(err.to_string()))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_format != SampleFormat::Float || spec.bits_per_sample != 32
    {
        return Err(AudioError::Config(format!(
            "expected mono float32 WAV, got {}ch {:?} {}-bit",
            spec.channels, spec.sample_format, spec.bits_per_sample
        )));
    }
    let samples = reader
        .samples::<f32>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| AudioError::Io(err.to_string()))?;
    Ok((spec.sample_rate, samples))
}
