//! Real-time audio via `cpal`.
//!
//! An output stream renders the synth engine and mixes in an optional input
//! stream (e.g. for monitoring an external source). Both streams are fully
//! configured and built before either is started, and they share the same
//! sample rate so the input can be summed directly into the output.
//!
//! The file is organised in two parts:
//!   1. **Stream setup** — all the `cpal` device/stream plumbing, in flow order.
//!   2. **Audio-thread processing** — the callback logic, free of `cpal` types.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SupportedStreamConfig};
use rtrb::RingBuffer;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use synth_core::{ControlMessage, FilterOversampling, FilterType, SynthEngine, VOICE_PACKS};

/// How long to wait for `cpal` to switch the device sample rate and build a
/// stream. CoreAudio rate changes can take longer than the default, so give
/// them generous headroom before treating the build as failed.
const STREAM_BUILD_TIMEOUT: Duration = Duration::from_secs(5);

use crate::engine::{self, AudioBlock, AudioMetrics, SynthEngineAudio};

// ============================================================================
// Stream setup
// ============================================================================

/// Spawns the audio thread: configures the output and optional input streams,
/// then starts them together and keeps them alive for the process lifetime.
pub fn start_audio(
    engine_audio: SynthEngineAudio,
    output_filter: Option<String>,
    input_filter: Option<String>,
    desired_rate: Option<u32>,
    filter_oversampling: FilterOversampling,
    filter_type: FilterType,
) {
    std::thread::spawn(move || {
        let host = cpal::default_host();

        let Some(output) = open_output(&host, output_filter.as_deref(), desired_rate) else {
            eprintln!("No audio output device available; audio disabled.");
            return;
        };

        // Configure the optional input at the output's sample rate so the two
        // streams match and the input can be summed in directly.
        let input = input_filter.as_deref().and_then(|filter| {
            open_input(&host, filter, output.sample_rate as u32, output.channels)
        });
        let (input_stream, input_consumer) = match input {
            Some(input) => (Some(input.stream), Some(input.consumer)),
            None => (None, None),
        };

        // Build the output stream (but don't start it yet).
        let renderer = Renderer::new(
            engine_audio,
            output.sample_rate,
            output.channels,
            input_consumer,
            filter_oversampling,
            filter_type,
        );
        let Some(output_stream) = build_output_stream(&output.device, output.config, renderer)
        else {
            eprintln!("Failed to build audio output stream; audio disabled.");
            return;
        };

        // Everything is configured: start both streams together.
        if let Some(ref stream) = input_stream {
            if let Err(err) = stream.play() {
                eprintln!("Failed to start audio input stream: {err}");
            }
        }
        if let Err(err) = output_stream.play() {
            eprintln!("Failed to start audio output stream: {err}");
            return;
        }

        // Park forever, keeping both streams (and their callbacks) alive.
        std::thread::park();
        drop((input_stream, output_stream));
    });
}

// --- output ---------------------------------------------------------------

/// A resolved output device and its negotiated stream configuration.
struct Output {
    device: cpal::Device,
    config: SupportedStreamConfig,
    sample_rate: f32,
    channels: usize,
}

/// Resolves the output device (by name filter, else default) and its config.
fn open_output(
    host: &cpal::Host,
    filter: Option<&str>,
    desired_rate: Option<u32>,
) -> Option<Output> {
    let devices: Vec<cpal::Device> = host
        .output_devices()
        .map(|devices| devices.collect())
        .unwrap_or_default();

    eprintln!("Available audio outputs:");
    for (index, device) in devices.iter().enumerate() {
        eprintln!("  [{index}] {}", device_name(device));
    }

    let device = match filter {
        Some(filter) => find_device(&devices, filter).or_else(|| {
            eprintln!(
                "No audio output matching \"{filter}\"; it may be busy, unplugged, or \
                 claimed by an aggregate device. Falling back to the system default."
            );
            host.default_output_device()
        }),
        None => host.default_output_device(),
    }?;
    eprintln!("Using output: {}", device_name(&device));

    let config = choose_output_config(&device, desired_rate)?;
    let sample_rate = config.sample_rate() as f32;
    let channels = config.channels() as usize;
    eprintln!(
        "Audio: {}Hz, {}ch, {:?}, buffer {:?}",
        sample_rate as u32,
        channels,
        config.sample_format(),
        config.buffer_size(),
    );

    Some(Output {
        device,
        config,
        sample_rate,
        channels,
    })
}

/// Picks a stereo F32 output config, preferring `desired_rate` when supported
/// and otherwise falling back to the device default.
fn choose_output_config(
    device: &cpal::Device,
    desired_rate: Option<u32>,
) -> Option<SupportedStreamConfig> {
    let default = device.default_output_config().ok()?;

    if let Some(rate) = desired_rate {
        let at_rate = device.supported_output_configs().ok().and_then(|configs| {
            configs
                .filter(|config| {
                    config.sample_format() == SampleFormat::F32 && config.contains_rate(rate)
                })
                .min_by_key(|config| {
                    // Prefer stereo, then the smallest channel count that supports the rate.
                    if config.channels() == 2 {
                        0
                    } else {
                        config.channels()
                    }
                })
                .map(|config| config.with_sample_rate(rate))
        });
        // A device may *advertise* a rate (via `supported_output_configs`) yet
        // still refuse to actually switch to it — e.g. when it is clock-locked
        // by an active aggregate device or an external clock source. CoreAudio
        // reports this only when the stream is built, so verify buildability
        // here and fall back to the device default when the switch is rejected.
        if let Some(config) = at_rate {
            if stream_builds(device, &config) {
                return Some(config);
            }
            eprintln!(
                "Device advertises {rate}Hz but refused to switch to it \
                 (likely clock-locked by an aggregate device or external clock); \
                 using device default."
            );
        } else {
            eprintln!("Requested sample rate {rate}Hz unsupported; using device default.");
        }
    }

    let default_rate = default.sample_rate();
    let stereo_f32 = device.supported_output_configs().ok().and_then(|configs| {
        configs
            .filter(|config| config.sample_format() == SampleFormat::F32)
            .min_by_key(|config| {
                if config.channels() == 2 {
                    0
                } else {
                    config.channels()
                }
            })
            .map(|config| {
                if config.contains_rate(default_rate) {
                    config.with_sample_rate(default_rate)
                } else {
                    let clamped =
                        default_rate.clamp(config.min_sample_rate(), config.max_sample_rate());
                    config.with_sample_rate(clamped)
                }
            })
    });

    stereo_f32.or(Some(default))
}

/// Attempts to build (and immediately drop) a no-op output stream to confirm
/// the device will actually accept `config`. This surfaces CoreAudio rate/format
/// rejections that only appear at build time rather than during enumeration.
fn stream_builds(device: &cpal::Device, config: &SupportedStreamConfig) -> bool {
    device
        .build_output_stream(
            config.clone().into(),
            |_data: &mut [f32], _info: &cpal::OutputCallbackInfo| {},
            |_err| {},
            Some(STREAM_BUILD_TIMEOUT),
        )
        .is_ok()
}

/// Builds the output stream, wiring the audio callback to the [`Renderer`].
fn build_output_stream(
    device: &cpal::Device,
    config: SupportedStreamConfig,
    mut renderer: Renderer,
) -> Option<cpal::Stream> {
    device
        .build_output_stream(
            config.into(),
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| renderer.render(data),
            |err| eprintln!("audio output error: {err}"),
            Some(STREAM_BUILD_TIMEOUT),
        )
        .map_err(|err| eprintln!("build_output_stream error: {err}"))
        .ok()
}

// --- input ----------------------------------------------------------------

/// A built (but not yet started) input stream and the ring it feeds.
struct Input {
    stream: cpal::Stream,
    consumer: rtrb::Consumer<f32>,
}

/// Resolves the named input device, requiring it to run at exactly `sample_rate`
/// (to match the output), and builds its capture stream. Returns `None` (with a
/// logged reason) when the input is unavailable, disabling input.
fn open_input(
    host: &cpal::Host,
    filter: &str,
    sample_rate: u32,
    out_channels: usize,
) -> Option<Input> {
    let devices: Vec<cpal::Device> = host
        .input_devices()
        .map(|devices| devices.collect())
        .unwrap_or_default();

    let Some(device) = find_device(&devices, filter) else {
        eprintln!("No audio input matching \"{filter}\"; input disabled.");
        return None;
    };

    let Some(config) = choose_input_config(&device, sample_rate) else {
        eprintln!(
            "Input \"{}\" cannot run at {}Hz (must match output rate); input disabled.",
            device_name(&device),
            sample_rate
        );
        log_supported_input_configs(&device);
        return None;
    };

    let in_channels = config.channels() as usize;
    let capacity = engine::MAX_AUDIO_BUF * out_channels * 4;
    let (producer, consumer) = RingBuffer::<f32>::new(capacity);
    let capture = InputCapture {
        producer,
        in_channels,
        out_channels,
    };
    let stream = build_input_stream(&device, config, capture)?;

    eprintln!(
        "Using input: {} at {}Hz, {}ch",
        device_name(&device),
        sample_rate,
        in_channels
    );
    Some(Input { stream, consumer })
}

/// Selects an F32 input config running at exactly `sample_rate`, or `None`.
fn choose_input_config(device: &cpal::Device, sample_rate: u32) -> Option<SupportedStreamConfig> {
    device.supported_input_configs().ok().and_then(|configs| {
        configs
            .filter(|config| {
                config.sample_format() == SampleFormat::F32 && config.contains_rate(sample_rate)
            })
            .map(|config| config.with_sample_rate(sample_rate))
            .next()
    })
}

/// Builds the input stream, wiring the capture callback to [`InputCapture`].
fn build_input_stream(
    device: &cpal::Device,
    config: SupportedStreamConfig,
    mut capture: InputCapture,
) -> Option<cpal::Stream> {
    device
        .build_input_stream(
            config.into(),
            move |data: &[f32], _info: &cpal::InputCallbackInfo| capture.capture(data),
            |err| eprintln!("audio input error: {err}"),
            Some(STREAM_BUILD_TIMEOUT),
        )
        .map_err(|err| eprintln!("build_input_stream error: {err}"))
        .ok()
}

/// Logs a device's supported input configs, to explain why it was rejected.
fn log_supported_input_configs(device: &cpal::Device) {
    let Ok(configs) = device.supported_input_configs() else {
        return;
    };
    eprintln!("  supported input configs:");
    for config in configs {
        eprintln!(
            "    {}ch {:?} {}-{}Hz",
            config.channels(),
            config.sample_format(),
            config.min_sample_rate(),
            config.max_sample_rate(),
        );
    }
}

// --- device helpers (also used by the settings UI) ------------------------

pub fn list_output_devices() -> Vec<String> {
    device_names(cpal::default_host().output_devices().ok())
}

pub fn list_input_devices() -> Vec<String> {
    device_names(cpal::default_host().input_devices().ok())
}

fn device_names<I: Iterator<Item = cpal::Device>>(devices: Option<I>) -> Vec<String> {
    let Some(devices) = devices else {
        return Vec::new();
    };
    devices
        .filter_map(|device| {
            device
                .description()
                .ok()
                .map(|desc| desc.name().to_string())
        })
        .collect()
}

fn device_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|desc| desc.name().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

fn find_device(devices: &[cpal::Device], filter: &str) -> Option<cpal::Device> {
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
}

// ============================================================================
// Audio-thread processing (no `cpal` types below this line)
// ============================================================================

/// Renders the synth on the audio thread, mixes in captured input, and reports
/// spectrum blocks and timing metrics back to the UI.
struct Renderer {
    engine_audio: SynthEngineAudio,
    engine: SynthEngine<VOICE_PACKS>,
    timing: AudioTiming,
    input: Option<rtrb::Consumer<f32>>,
    input_enabled: Arc<AtomicBool>,
    sample_rate: f32,
    channels: usize,
}

impl Renderer {
    fn new(
        engine_audio: SynthEngineAudio,
        sample_rate: f32,
        channels: usize,
        input: Option<rtrb::Consumer<f32>>,
        filter_oversampling: FilterOversampling,
        filter_type: FilterType,
    ) -> Self {
        let input_enabled = engine_audio.input_enabled.clone();
        let mut engine = SynthEngine::<VOICE_PACKS>::new(sample_rate);
        engine.set_filter_oversampling(filter_oversampling);
        engine.set_filter_type(filter_type);
        eprintln!(
            "Filter oversampling: {:?} ({}x)",
            filter_oversampling,
            filter_oversampling.factor(sample_rate)
        );
        Self {
            engine_audio,
            engine,
            timing: AudioTiming::default(),
            input,
            input_enabled,
            sample_rate,
            channels,
        }
    }

    /// Fills one interleaved output buffer: drains control messages, renders the
    /// synth, mixes in any input, and publishes spectrum/timing feedback.
    fn render(&mut self, data: &mut [f32]) {
        let callback_start = Instant::now();
        let frames = data.len() / self.channels;
        let deadline = Duration::from_secs_f64(frames as f64 / self.sample_rate as f64);

        // Oversampling can be changed from the settings UI in bursts. Apply
        // only the last requested mode per callback so the audio thread does
        // not repeatedly clear decimator state while rendering.
        let mut pending_oversampling = None;
        let mut pending_filter_type = None;

        self.engine_audio.control.drain(|message| match message {
            ControlMessage::SetFilterOversampling(oversampling) => {
                pending_oversampling = Some(oversampling);
            }
            ControlMessage::SetFilterType(filter_type) => {
                pending_filter_type = Some(filter_type);
            }
            message => self.engine.handle_control(message),
        });

        if let Some(oversampling) = pending_oversampling {
            self.engine.set_filter_oversampling(oversampling);
        }
        if let Some(filter_type) = pending_filter_type {
            self.engine.set_filter_type(filter_type);
        }

        let render_start = Instant::now();
        self.engine.process_interleaved(data, self.channels);
        let mut analysis_block = capture_synth_block(data, self.channels);
        if let Some(consumer) = self.input.as_mut() {
            consume_input(
                data,
                consumer,
                self.channels,
                frames,
                self.input_enabled.load(Ordering::Relaxed),
                &mut analysis_block,
            );
        }
        let render_elapsed = render_start.elapsed();

        self.engine_audio
            .feedback
            .set_active_voices(self.engine.active_voice_count());

        self.engine_audio.feedback.push_audio_block(analysis_block);

        if let Some(metrics) =
            self.timing
                .record(callback_start.elapsed(), render_elapsed, deadline)
        {
            self.engine_audio.feedback.push_metrics(metrics);
        }
    }
}

/// Copies the pre-input synth render into a synchronized analysis block.
fn capture_synth_block(data: &[f32], channels: usize) -> AudioBlock {
    let mut block = AudioBlock::default();
    let frame_count = (data.len() / channels).min(engine::MAX_AUDIO_BUF);
    for (index, frame) in data[..frame_count * channels]
        .chunks_exact(channels)
        .enumerate()
    {
        block.output_left[index] = frame[0];
        block.output_right[index] = frame.get(1).copied().unwrap_or(frame[0]);
    }
    block.len = frame_count as u16;
    block
}

/// Captures an input device's frames into a ring buffer for the output thread,
/// mapping the device's channel layout onto the output's channel count.
struct InputCapture {
    producer: rtrb::Producer<f32>,
    in_channels: usize,
    out_channels: usize,
}

impl InputCapture {
    fn capture(&mut self, data: &[f32]) {
        for frame in data.chunks_exact(self.in_channels) {
            for out in 0..self.out_channels {
                let sample = if out < self.in_channels {
                    frame[out]
                } else {
                    frame[0]
                };
                let _ = self.producer.push(sample);
            }
        }
    }
}

/// Captures input samples for analysis and optionally mixes them into the output.
///
/// Consumes only whole frames so the ring's read position stays frame-aligned;
/// a partially written frame is left in place until the input callback completes
/// it, preventing permanent left/right channel desync on underrun.
///
/// Because the input and output streams run on independent clocks (and the input
/// stream starts filling the ring before the output stream begins draining it),
/// buffered latency can build up and drift. Before consuming we drop the oldest
/// whole frames beyond a small target so monitoring latency stays low and bounded.
fn consume_input(
    data: &mut [f32],
    consumer: &mut rtrb::Consumer<f32>,
    channels: usize,
    block_frames: usize,
    mix_enabled: bool,
    analysis_block: &mut AudioBlock,
) {
    // Keep at most a couple of output blocks buffered to absorb jitter without
    // accumulating unbounded latency.
    let target_frames = (block_frames * 2).max(1);
    let available_frames = consumer.slots() / channels;
    if available_frames > target_frames {
        let drop_samples = (available_frames - target_frames) * channels;
        for _ in 0..drop_samples {
            if consumer.pop().is_err() {
                break;
            }
        }
    }

    for (frame_index, frame) in data.chunks_exact_mut(channels).enumerate() {
        if consumer.slots() < channels {
            break;
        }
        let mut input_left = 0.0;
        let mut input_right = None;
        for (channel, sample) in frame.iter_mut().enumerate() {
            if let Ok(input) = consumer.pop() {
                if channel == 0 {
                    input_left = input;
                } else if channel == 1 {
                    input_right = Some(input);
                }
                if mix_enabled {
                    *sample = (*sample + input).clamp(-1.0, 1.0);
                }
            }
        }
        if frame_index < engine::MAX_AUDIO_BUF {
            analysis_block.input_left[frame_index] = input_left;
            analysis_block.input_right[frame_index] = input_right.unwrap_or(input_left);
        }
    }
}

/// Accumulates callback/render timing and emits an [`AudioMetrics`] snapshot
/// roughly once per second.
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
    ) -> Option<AudioMetrics> {
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
            return None;
        }

        let callbacks = self.callbacks.max(1);
        let metrics = AudioMetrics {
            deadline_ms: deadline.as_secs_f64() * 1_000.0,
            callback_avg_ms: self.callback_total.as_secs_f64() * 1_000.0 / callbacks as f64,
            callback_max_ms: self.callback_max.as_secs_f64() * 1_000.0,
            render_avg_ms: self.render_total.as_secs_f64() * 1_000.0 / callbacks as f64,
            render_max_ms: self.render_max.as_secs_f64() * 1_000.0,
            overruns: self.overruns,
            render_overruns: self.render_overruns,
            callbacks,
        };

        *self = Self {
            last_report: Some(now),
            ..Self::default()
        };

        Some(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::consume_input;
    use crate::engine::AudioBlock;
    use rtrb::RingBuffer;

    #[test]
    fn mix_input_only_consumes_whole_frames() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(16);
        for value in [0.1, 0.2, 0.3] {
            producer.push(value).unwrap();
        }

        let mut data = [0.0f32; 8];
        let mut analysis = AudioBlock::default();
        consume_input(&mut data, &mut consumer, 2, 1024, true, &mut analysis);

        assert_eq!(data, [0.1, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(analysis.input_left[0], 0.1);
        assert_eq!(analysis.input_right[0], 0.2);
        assert_eq!(consumer.slots(), 1, "partial frame stays buffered");
    }

    #[test]
    fn mix_input_stays_frame_aligned_across_underrun() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(16);
        producer.push(0.1).unwrap();
        producer.push(0.2).unwrap();
        producer.push(0.3).unwrap();

        let mut first = [0.0f32; 4];
        let mut analysis = AudioBlock::default();
        consume_input(&mut first, &mut consumer, 2, 1024, true, &mut analysis);
        assert_eq!(first, [0.1, 0.2, 0.0, 0.0]);

        producer.push(0.4).unwrap();
        let mut second = [0.0f32; 4];
        consume_input(&mut second, &mut consumer, 2, 1024, true, &mut analysis);
        assert_eq!(
            second,
            [0.3, 0.4, 0.0, 0.0],
            "left-channel sample must not shift into the right channel"
        );
    }

    #[test]
    fn mix_input_sums_and_clamps() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(16);
        producer.push(0.8).unwrap();
        producer.push(-2.0).unwrap();

        let mut data = [0.5f32, 0.5];
        let mut analysis = AudioBlock::default();
        consume_input(&mut data, &mut consumer, 2, 1024, true, &mut analysis);
        assert_eq!(data, [1.0, -1.0]);
    }

    #[test]
    fn mix_input_drops_oldest_excess_to_bound_latency() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(32);
        for i in 0..6 {
            producer.push(i as f32 / 16.0).unwrap();
            producer.push(i as f32 / 16.0 + 1.0 / 32.0).unwrap();
        }

        let mut data = [0.0f32; 4];
        let mut analysis = AudioBlock::default();
        consume_input(&mut data, &mut consumer, 2, 1, true, &mut analysis);

        assert_eq!(data, [0.25, 0.28125, 0.3125, 0.34375]);
        assert_eq!(consumer.slots(), 0, "no stale frames left buffered");
    }

    #[test]
    fn muted_input_is_captured_without_being_mixed() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);
        producer.push(0.25).unwrap();
        producer.push(-0.5).unwrap();

        let mut data = [0.75f32, 0.75];
        let mut analysis = AudioBlock::default();
        consume_input(&mut data, &mut consumer, 2, 1024, false, &mut analysis);

        assert_eq!(data, [0.75, 0.75]);
        assert_eq!(analysis.input_left[0], 0.25);
        assert_eq!(analysis.input_right[0], -0.5);
        assert_eq!(consumer.slots(), 0);
    }
}
