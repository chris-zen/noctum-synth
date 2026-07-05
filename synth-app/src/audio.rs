//! Real-time audio output via `cpal`.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, SupportedStreamConfig};
use std::time::{Duration, Instant};

use synth_core::SynthEngine;

use crate::engine::{self, AudioBlock, SynthEngineAudio};

pub fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    match host.output_devices() {
        Ok(devices) => devices
            .filter_map(|device| {
                device
                    .description()
                    .ok()
                    .map(|desc| desc.name().to_string())
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub fn start_audio(mut engine_audio: SynthEngineAudio, device_filter: Option<String>) {
    std::thread::spawn(move || {
        let host = cpal::default_host();
        let devices: Vec<cpal::Device> = host
            .output_devices()
            .map(|devices| devices.collect())
            .unwrap_or_default();

        eprintln!("Available audio outputs:");
        for (index, device) in devices.iter().enumerate() {
            let name = device
                .description()
                .map(|desc| desc.name().to_string())
                .unwrap_or_else(|_| "unknown".into());
            eprintln!("  [{index}] {name}");
        }

        let device = if let Some(ref filter) = device_filter {
            let filter_lower = filter.to_lowercase();
            devices
                .iter()
                .find(|device| {
                    device
                        .description()
                        .map(|desc| desc.name().to_lowercase().contains(&filter_lower))
                        .unwrap_or(false)
                })
                .cloned()
        } else {
            host.default_output_device()
        };

        let device = device
            .or_else(|| host.default_output_device())
            .expect("no audio output device");

        let name = device
            .description()
            .map(|desc| desc.name().to_string())
            .unwrap_or_else(|_| "unknown".into());
        eprintln!("Using: {name}");

        let config = choose_output_config(&device).expect("no output config");
        let sample_rate = config.sample_rate() as f32;
        let buf_samples = config.buffer_size();
        let channels = config.channels() as usize;
        eprintln!(
            "Audio: {}Hz, {}ch, {:?}, buffer {:?}",
            sample_rate as u32,
            channels,
            config.sample_format(),
            buf_samples
        );

        let mut engine = SynthEngine::new(sample_rate);
        let mut timing = AudioTiming::default();

        let stream = device
            .build_output_stream(
                config.into(),
                move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    let callback_start = Instant::now();
                    let frames = data.len() / channels;
                    let deadline = Duration::from_secs_f64(frames as f64 / sample_rate as f64);

                    engine_audio
                        .control
                        .drain(|message| engine.handle_control(message));
                    let render_start = Instant::now();
                    engine.process_interleaved(data, channels);
                    let render_elapsed = render_start.elapsed();

                    engine_audio
                        .feedback
                        .set_active_voices(engine.active_voice_count());

                    let mut block = AudioBlock::default();
                    let frame_count = (data.len() / channels).min(engine::MAX_AUDIO_BUF);
                    for (frame_index, frame) in data[..frame_count * channels]
                        .chunks_exact(channels)
                        .enumerate()
                    {
                        block.left[frame_index] = frame[0];
                        block.right[frame_index] = frame.get(1).copied().unwrap_or(frame[0]);
                    }
                    block.len = frame_count as u16;
                    engine_audio.feedback.push_audio_block(block);

                    timing.record(callback_start.elapsed(), render_elapsed, deadline, frames);
                },
                |err| eprintln!("audio error: {err}"),
                None,
            )
            .expect("failed to build audio stream");

        stream.play().expect("failed to start audio stream");
        std::thread::park();
    });
}

fn choose_output_config(device: &cpal::Device) -> Option<SupportedStreamConfig> {
    let default = device.default_output_config().ok()?;
    let default_rate = default.sample_rate();

    let stereo_f32 = device.supported_output_configs().ok().and_then(|configs| {
        configs
            .filter(|config| config.channels() == 2 && config.sample_format() == SampleFormat::F32)
            .map(|config| {
                if config.contains_rate(default_rate) {
                    config.with_sample_rate(default_rate)
                } else {
                    config.with_sample_rate(clamp_sample_rate(
                        default_rate,
                        config.min_sample_rate(),
                        config.max_sample_rate(),
                    ))
                }
            })
            .next()
    });

    stereo_f32.or(Some(default))
}

fn clamp_sample_rate(rate: SampleRate, min: SampleRate, max: SampleRate) -> SampleRate {
    rate.clamp(min, max)
}

#[derive(Default)]
struct AudioTiming {
    callbacks: u64,
    overruns: u64,
    render_overruns: u64,
    callback_total: Duration,
    render_total: Duration,
    callback_max: Duration,
    render_max: Duration,
    last_report: Option<Instant>,
}

impl AudioTiming {
    fn record(
        &mut self,
        callback_elapsed: Duration,
        render_elapsed: Duration,
        deadline: Duration,
        frames: usize,
    ) {
        self.callbacks += 1;
        self.callback_total += callback_elapsed;
        self.render_total += render_elapsed;
        self.callback_max = self.callback_max.max(callback_elapsed);
        self.render_max = self.render_max.max(render_elapsed);

        if callback_elapsed > deadline {
            self.overruns += 1;
        }
        if render_elapsed > deadline {
            self.render_overruns += 1;
        }

        let now = Instant::now();
        let last_report = self.last_report.get_or_insert(now);
        if now.duration_since(*last_report) < Duration::from_secs(1) {
            return;
        }

        let callbacks = self.callbacks.max(1);
        eprintln!(
            "Audio timing: frames={} deadline={:.3}ms callback avg/max={:.3}/{:.3}ms render avg/max={:.3}/{:.3}ms overruns={}/{} render_overruns={}/{}",
            frames,
            deadline.as_secs_f64() * 1_000.0,
            self.callback_total.as_secs_f64() * 1_000.0 / callbacks as f64,
            self.callback_max.as_secs_f64() * 1_000.0,
            self.render_total.as_secs_f64() * 1_000.0 / callbacks as f64,
            self.render_max.as_secs_f64() * 1_000.0,
            self.overruns,
            callbacks,
            self.render_overruns,
            callbacks,
        );

        *self = Self {
            last_report: Some(now),
            ..Self::default()
        };
    }
}
