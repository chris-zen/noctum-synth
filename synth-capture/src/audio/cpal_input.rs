use std::{
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

use cpal::{
    SampleFormat, Stream, StreamConfig, SupportedStreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::audio::{AudioCounters, AudioError, AudioFormat, AudioHealth, AudioInput};

const RING_SECONDS: usize = 8;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const STREAM_BUILD_TIMEOUT: Duration = Duration::from_secs(5);
const DEVICE_SETTLE_DELAY: Duration = Duration::from_millis(200);
const RESET_STABILIZE_ATTEMPTS: usize = 8;

pub struct CpalInputConfig {
    pub device_name: String,
    pub sample_rate: u32,
    pub input_channel: u32,
    pub require_float32: bool,
}

pub struct CpalAudioInput {
    _stream: Stream,
    consumer: Consumer<f32>,
    counters: AudioCounters,
    stop: Arc<std::sync::atomic::AtomicBool>,
    format: AudioFormat,
}

impl CpalAudioInput {
    pub fn open(config: CpalInputConfig) -> Result<Self, AudioError> {
        Self::open_with_stop(config, Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }

    pub fn open_with_stop(
        config: CpalInputConfig,
        stop: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = find_exact_input_device(&host, &config.device_name)?;
        let supported = choose_input_config(&device, &config)?;
        let stream_config: StreamConfig = supported.clone().into();
        let channel_count = stream_config.channels as usize;
        let channel_index = config.input_channel as usize;
        if channel_index >= channel_count {
            return Err(AudioError::Config(format!(
                "input channel {} out of range for {} channel device `{}`",
                config.input_channel, channel_count, config.device_name
            )));
        }
        let capacity = (config.sample_rate as usize)
            .saturating_mul(RING_SECONDS)
            .max(8192);
        let (producer, consumer) = RingBuffer::<f32>::new(capacity);
        let counters = AudioCounters::default();
        let overflow = counters.overflow_handle();
        let errors = counters.error_handle();
        let stop_flag = Arc::clone(&stop);

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_input_stream::<f32, _>(
                &device,
                &stream_config,
                producer,
                channel_count,
                channel_index,
                overflow,
                errors,
                stop_flag,
                |sample| sample,
            )?,
            other => {
                return Err(AudioError::Config(format!(
                    "unsupported sample format {other:?}; Phase 4 requires F32"
                )));
            }
        };
        thread::sleep(DEVICE_SETTLE_DELAY);
        stream
            .play()
            .map_err(|err| AudioError::Stream(err.to_string()))?;
        thread::sleep(DEVICE_SETTLE_DELAY);
        let mut input = Self {
            _stream: stream,
            consumer,
            counters,
            stop,
            format: AudioFormat {
                sample_rate_hz: stream_config.sample_rate,
                channels: stream_config.channels,
                input_channel: config.input_channel,
                native_float32: matches!(supported.sample_format(), SampleFormat::F32),
            },
        };
        input.reset_health();
        Ok(input)
    }

    fn health_error(&self) -> Option<AudioError> {
        AudioError::from_health(&self.counters.snapshot())
    }

    fn drain_available(&mut self) {
        while self.consumer.pop().is_ok() {}
    }
}

impl AudioInput for CpalAudioInput {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn drain_frames(&mut self, frame_count: usize, dest: &mut Vec<f32>) -> Result<(), AudioError> {
        dest.clear();
        dest.reserve(frame_count);
        let deadline = Instant::now() + DRAIN_TIMEOUT;
        while dest.len() < frame_count {
            if self.stop.load(Ordering::SeqCst) {
                return Err(AudioError::Stream("stop requested".to_string()));
            }
            match self.consumer.pop() {
                Ok(sample) => dest.push(sample),
                Err(_) => {
                    if Instant::now() > deadline {
                        return Err(AudioError::Underrun {
                            expected: frame_count,
                            got: dest.len(),
                        });
                    }
                    if let Some(err) = self.health_error() {
                        return Err(err);
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
        if let Some(err) = self.health_error() {
            return Err(err);
        }
        Ok(())
    }

    fn health(&self) -> AudioHealth {
        self.counters.snapshot()
    }

    fn reset_health(&mut self) {
        for _ in 0..RESET_STABILIZE_ATTEMPTS {
            self.drain_available();
            self.counters.reset();
            thread::sleep(Duration::from_millis(1));
            self.drain_available();
            if self.counters.snapshot().is_clean() {
                self.counters.reset();
                return;
            }
            self.counters.reset();
        }
        self.drain_available();
        self.counters.reset();
    }
}

impl Drop for CpalAudioInput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn choose_input_config(
    device: &cpal::Device,
    config: &CpalInputConfig,
) -> Result<SupportedStreamConfig, AudioError> {
    let ranges = device
        .supported_input_configs()
        .map_err(|err| AudioError::Config(err.to_string()))?;
    let matched = ranges
        .filter(|range| {
            range.channels() as u32 > config.input_channel
                && range.contains_rate(config.sample_rate)
                && (!config.require_float32 || matches!(range.sample_format(), SampleFormat::F32))
        })
        .map(|range| range.with_sample_rate(config.sample_rate))
        .find(|supported| {
            !config.require_float32 || matches!(supported.sample_format(), SampleFormat::F32)
        })
        .ok_or_else(|| {
            AudioError::Config(format!(
                "device `{}` has no matching input config for {} Hz channel {}",
                config.device_name, config.sample_rate, config.input_channel
            ))
        })?;
    if config.require_float32 && !matches!(matched.sample_format(), SampleFormat::F32) {
        return Err(AudioError::Config(
            "native float32 input required".to_string(),
        ));
    }
    Ok(matched)
}

fn find_exact_input_device(host: &cpal::Host, requested: &str) -> Result<cpal::Device, AudioError> {
    let devices = host
        .input_devices()
        .map_err(|err| AudioError::Config(err.to_string()))?;
    let mut available = Vec::new();
    let mut match_device = None;
    for device in devices {
        let Ok(description) = device.description() else {
            continue;
        };
        let name = description.name().to_string();
        if name == requested {
            if match_device.is_some() {
                return Err(AudioError::Config(format!(
                    "ambiguous audio input name `{requested}`"
                )));
            }
            match_device = Some(device);
        }
        available.push(name);
    }
    match_device.ok_or_else(|| AudioError::DeviceNotFound {
        requested: requested.to_string(),
        available: available.join(", "),
    })
}

fn build_input_stream<T, F>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut producer: Producer<f32>,
    channel_count: usize,
    channel_index: usize,
    overflow: Arc<std::sync::atomic::AtomicU64>,
    errors: Arc<std::sync::atomic::AtomicU64>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    convert: F,
) -> Result<Stream, AudioError>
where
    T: cpal::SizedSample,
    F: Fn(T) -> f32 + Send + 'static,
{
    let err_fn = move |_err| {
        errors.fetch_add(1, Ordering::AcqRel);
    };
    device
        .build_input_stream(
            *config,
            move |data: &[T], _| {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let mut dropped = 0u64;
                for frame in data.chunks(channel_count) {
                    if channel_index >= frame.len() {
                        continue;
                    }
                    let sample = convert(frame[channel_index]);
                    if producer.push(sample).is_err() {
                        dropped += 1;
                    }
                }
                if dropped > 0 {
                    overflow.fetch_add(dropped, Ordering::AcqRel);
                }
            },
            err_fn,
            Some(STREAM_BUILD_TIMEOUT),
        )
        .map_err(|err| AudioError::Stream(err.to_string()))
}
