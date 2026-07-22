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
use parking_lot::RwLock;
use rtrb::RingBuffer;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use synth_core::{
    ControlMessage, FilterOversampling, FilterType, SynthEngineWithMemory, VOICE_PACKS,
};

/// How long to wait for `cpal` to switch the device sample rate and build a
/// stream. CoreAudio rate changes can take longer than the default, so give
/// them generous headroom before treating the build as failed.
const STREAM_BUILD_TIMEOUT: Duration = Duration::from_secs(5);
/// Pause after stopping streams or switching the output rate so CoreAudio can
/// settle before input devices (e.g. BlackHole) are opened or started.
const DEVICE_SETTLE_DELAY: Duration = Duration::from_millis(200);
const RECONNECT_INTERVAL: Duration = Duration::from_millis(500);
fn log_audio(message: impl AsRef<str>) {
    eprintln!("[audio] {}", message.as_ref());
}

fn wait_for_device_settle() {
    thread::sleep(DEVICE_SETTLE_DELAY);
}

fn describe_config(config: &AudioConfig) -> String {
    let rate = config
        .sample_rate
        .map(|rate| format!("{rate} Hz"))
        .unwrap_or_else(|| "device default".to_string());
    let output = config.output_device.as_deref().unwrap_or("system default");
    let input = config.input_device.as_deref().unwrap_or("none");
    format!("output={output}, input={input}, rate={rate}")
}

use crate::engine::{
    self, AudioBlock, AudioMetrics, SynthEngineAudio, SynthEngineBridge, rebind_audio_channels,
};

// ============================================================================
// Audio manager
// ============================================================================

#[derive(Clone)]
pub struct AudioConfig {
    pub output_device: Option<String>,
    pub input_device: Option<String>,
    pub sample_rate: Option<u32>,
    pub filter_oversampling: FilterOversampling,
    pub filter_type: FilterType,
}

#[derive(Clone)]
pub struct AppliedAudioConfig {
    pub sample_rate: u32,
    pub sample_rate_setting: Option<u32>,
    pub applying: bool,
    pub error: Option<String>,
    pub generation: u64,
}

impl Default for AppliedAudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 0,
            sample_rate_setting: None,
            applying: false,
            error: None,
            generation: 0,
        }
    }
}

#[derive(Clone)]
pub struct AudioManager {
    request_tx: mpsc::Sender<AudioConfig>,
    applied: Arc<RwLock<AppliedAudioConfig>>,
}

impl AudioManager {
    pub fn start(
        bridge: SynthEngineBridge,
        engine_audio: SynthEngineAudio,
        initial: AudioConfig,
    ) -> Self {
        let (request_tx, request_rx) = mpsc::channel();
        let applied = Arc::new(RwLock::new(AppliedAudioConfig::default()));
        let applied_thread = applied.clone();
        std::thread::spawn(move || {
            run_audio_thread(bridge, engine_audio, initial, request_rx, applied_thread);
        });
        Self {
            request_tx,
            applied,
        }
    }

    pub fn apply(&self, config: AudioConfig) {
        {
            let mut applied = self.applied.write();
            applied.applying = true;
            applied.error = None;
        }
        let _ = self.request_tx.send(config);
    }

    pub fn applied(&self) -> AppliedAudioConfig {
        self.applied.read().clone()
    }
}

#[allow(dead_code)]
struct AudioSession(Option<cpal::Stream>, cpal::Stream);

fn run_audio_thread(
    bridge: SynthEngineBridge,
    mut engine_audio: SynthEngineAudio,
    initial: AudioConfig,
    request_rx: mpsc::Receiver<AudioConfig>,
    applied: Arc<RwLock<AppliedAudioConfig>>,
) {
    let host = cpal::default_host();
    let mut session: Option<AudioSession> = None;
    let mut generation: u64 = 0;
    let mut last_good_config = initial.clone();
    let disconnected = Arc::new(AtomicBool::new(false));
    let session_generation = Arc::new(AtomicU64::new(0));

    let options = |mode, fallback_allowed| SessionOptions {
        mode,
        fallback_allowed,
        disconnected: disconnected.clone(),
        session_generation: session_generation.clone(),
    };

    log_audio("Starting audio engine");
    log_available_outputs(&host);

    session_generation.fetch_add(1, Ordering::SeqCst);
    let init_options = options(SessionMode::Initial, true);
    match start_session(&host, engine_audio, &initial, &init_options) {
        Ok((new_session, info)) => {
            generation += 1;
            let mut applied_state = applied_audio_config(&info, None);
            applied_state.generation = generation;
            *applied.write() = applied_state;
            session = Some(new_session);
            last_good_config = effective_config(&initial, &info);
        }
        Err(err) => {
            log_audio(&format!("Failed to start audio: {err}"));
            *applied.write() = AppliedAudioConfig {
                applying: false,
                error: Some(err),
                generation,
                ..AppliedAudioConfig::default()
            };
        }
    }

    loop {
        match request_rx.recv_timeout(RECONNECT_INTERVAL) {
            Ok(config) => {
                log_audio(&format!(
                    "Restarting audio ({})...",
                    describe_config(&config)
                ));
                drop(session.take());
                log_audio("Stopped previous audio session");
                wait_for_device_settle();
                engine_audio = rebind_audio_channels(&bridge);
                disconnected.store(false, Ordering::SeqCst);
                session_generation.fetch_add(1, Ordering::SeqCst);

                match probe_session(&host, &config, true) {
                    Ok(probed) => {
                        let build_opts = options(SessionMode::Restart, true);
                        match build_session(&host, engine_audio, &config, &probed, &build_opts) {
                            Ok(new_session) => {
                                let info = session_info(&config, &probed);
                                generation += 1;
                                let mut applied_state = applied_audio_config(&info, None);
                                applied_state.generation = generation;
                                *applied.write() = applied_state;
                                session = Some(new_session);
                                last_good_config = effective_config(&config, &info);
                            }
                            Err(err) => {
                                log_audio(&format!("Failed to apply audio config: {err}"));
                                engine_audio = rebind_audio_channels(&bridge);
                                session_generation.fetch_add(1, Ordering::SeqCst);
                                let rec = options(SessionMode::Recovery, true);
                                match start_session(&host, engine_audio, &last_good_config, &rec) {
                                    Ok((recovered, info)) => {
                                        generation += 1;
                                        let mut applied_state = applied_audio_config(
                                            &info,
                                            Some(format!("{err} (reverted to previous settings)")),
                                        );
                                        applied_state.generation = generation;
                                        *applied.write() = applied_state;
                                        session = Some(recovered);
                                    }
                                    Err(recover_err) => {
                                        *applied.write() = AppliedAudioConfig {
                                            applying: false,
                                            error: Some(format!(
                                                "{err}; recovery failed: {recover_err}"
                                            )),
                                            generation,
                                            ..applied.read().clone()
                                        };
                                    }
                                }
                            }
                        }
                    }
                    Err(err) => {
                        log_audio(&format!("Audio config probe failed: {err}"));
                        session_generation.fetch_add(1, Ordering::SeqCst);
                        let rec = options(SessionMode::Recovery, true);
                        match start_session(&host, engine_audio, &last_good_config, &rec) {
                            Ok((recovered, info)) => {
                                generation += 1;
                                let mut applied_state = applied_audio_config(
                                    &info,
                                    Some(format!("{err} (kept previous settings)")),
                                );
                                applied_state.generation = generation;
                                *applied.write() = applied_state;
                                session = Some(recovered);
                            }
                            Err(recover_err) => {
                                *applied.write() = AppliedAudioConfig {
                                    applying: false,
                                    error: Some(format!("{err}; recovery failed: {recover_err}")),
                                    generation,
                                    ..applied.read().clone()
                                };
                            }
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !disconnected.load(Ordering::SeqCst) {
                    continue;
                }
                log_audio(
                    "Audio device disconnected — will reconnect automatically when available",
                );
                drop(session.take());
                wait_for_device_settle();

                loop {
                    match request_rx.recv_timeout(RECONNECT_INTERVAL) {
                        Ok(config) => {
                            log_audio(&format!(
                                "Reconfiguring while disconnected ({})...",
                                describe_config(&config)
                            ));
                            engine_audio = rebind_audio_channels(&bridge);
                            disconnected.store(false, Ordering::SeqCst);
                            session_generation.fetch_add(1, Ordering::SeqCst);

                            match probe_session(&host, &config, true) {
                                Ok(probed) => {
                                    let build_opts = options(SessionMode::Restart, true);
                                    match build_session(
                                        &host,
                                        engine_audio,
                                        &config,
                                        &probed,
                                        &build_opts,
                                    ) {
                                        Ok(new_session) => {
                                            let info = session_info(&config, &probed);
                                            generation += 1;
                                            let mut applied_state =
                                                applied_audio_config(&info, None);
                                            applied_state.generation = generation;
                                            *applied.write() = applied_state;
                                            session = Some(new_session);
                                            last_good_config = effective_config(&config, &info);
                                            break;
                                        }
                                        Err(err) => {
                                            log_audio(&format!(
                                                "Reconfig failed while reconnecting: {err}"
                                            ));
                                            engine_audio = rebind_audio_channels(&bridge);
                                            session_generation.fetch_add(1, Ordering::SeqCst);
                                            let rec = options(SessionMode::Recovery, true);
                                            match start_session(
                                                &host,
                                                engine_audio,
                                                &last_good_config,
                                                &rec,
                                            ) {
                                                Ok((recovered, info)) => {
                                                    generation += 1;
                                                    let mut applied_state = applied_audio_config(
                                                        &info,
                                                        Some(format!(
                                                            "{err} (reverted to previous settings)"
                                                        )),
                                                    );
                                                    applied_state.generation = generation;
                                                    *applied.write() = applied_state;
                                                    session = Some(recovered);
                                                    break;
                                                }
                                                Err(recover_err) => {
                                                    *applied.write() = AppliedAudioConfig {
                                                        applying: false,
                                                        error: Some(format!(
                                                            "{err}; recovery failed: {recover_err}"
                                                        )),
                                                        generation,
                                                        ..applied.read().clone()
                                                    };
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    log_audio(&format!("Audio config probe failed: {err}"));
                                    session_generation.fetch_add(1, Ordering::SeqCst);
                                    let rec = options(SessionMode::Recovery, true);
                                    match start_session(
                                        &host,
                                        engine_audio,
                                        &last_good_config,
                                        &rec,
                                    ) {
                                        Ok((recovered, info)) => {
                                            generation += 1;
                                            let mut applied_state = applied_audio_config(
                                                &info,
                                                Some(format!("{err} (kept previous settings)")),
                                            );
                                            applied_state.generation = generation;
                                            *applied.write() = applied_state;
                                            session = Some(recovered);
                                            break;
                                        }
                                        Err(recover_err) => {
                                            *applied.write() = AppliedAudioConfig {
                                                applying: false,
                                                error: Some(format!(
                                                    "{err}; recovery failed: {recover_err}"
                                                )),
                                                generation,
                                                ..applied.read().clone()
                                            };
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            engine_audio = rebind_audio_channels(&bridge);
                            session_generation.fetch_add(1, Ordering::SeqCst);
                            let rec = options(SessionMode::Recovery, false);
                            match start_session(&host, engine_audio, &last_good_config, &rec) {
                                Ok((new_session, info)) => {
                                    generation += 1;
                                    let mut applied_state = applied_audio_config(&info, None);
                                    applied_state.generation = generation;
                                    *applied.write() = applied_state;
                                    session = Some(new_session);
                                    disconnected.store(false, Ordering::SeqCst);
                                    log_audio("Reconnected to audio device");
                                    break;
                                }
                                Err(_) => {
                                    // Device not yet available — retry next interval
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

struct SessionInfo {
    sample_rate: u32,
    sample_rate_setting: Option<u32>,
}

fn session_info(_config: &AudioConfig, probed: &ProbedSession) -> SessionInfo {
    SessionInfo {
        sample_rate: probed.sample_rate,
        sample_rate_setting: probed.output.sample_rate_setting,
    }
}

fn applied_audio_config(info: &SessionInfo, error: Option<String>) -> AppliedAudioConfig {
    AppliedAudioConfig {
        sample_rate: info.sample_rate,
        sample_rate_setting: info.sample_rate_setting,
        applying: false,
        error,
        generation: 0,
    }
}

fn effective_config(config: &AudioConfig, info: &SessionInfo) -> AudioConfig {
    AudioConfig {
        output_device: config.output_device.clone(),
        input_device: config.input_device.clone(),
        sample_rate: info.sample_rate_setting,
        filter_oversampling: config.filter_oversampling,
        filter_type: config.filter_type,
    }
}

struct ProbedOutput {
    output: Output,
}

struct ProbedSession {
    output: Output,
    sample_rate: u32,
}

#[derive(Clone, Copy)]
enum SessionMode {
    Initial,
    Restart,
    Recovery,
}

#[derive(Clone)]
struct SessionOptions {
    mode: SessionMode,
    fallback_allowed: bool,
    disconnected: Arc<AtomicBool>,
    session_generation: Arc<AtomicU64>,
}

fn probe_session(
    host: &cpal::Host,
    config: &AudioConfig,
    fallback_allowed: bool,
) -> Result<ProbedSession, String> {
    let ProbedOutput { output } = probe_output(
        host,
        config.output_device.as_deref(),
        config.sample_rate,
        fallback_allowed,
    )?;
    let sample_rate = output.sample_rate as u32;

    if let Some(filter) = config.input_device.as_deref() {
        let devices: Vec<cpal::Device> = host
            .input_devices()
            .map(|devices| devices.collect())
            .unwrap_or_default();
        let Some(device) = find_device(&devices, filter) else {
            return Err(format!("No audio input matching \"{filter}\"."));
        };
        if choose_input_config(&device, sample_rate).is_none() {
            return Err(format!(
                "Input \"{}\" cannot run at {sample_rate}Hz (must match output rate).",
                device_name(&device)
            ));
        }
    }

    Ok(ProbedSession {
        output,
        sample_rate,
    })
}

fn probe_output(
    host: &cpal::Host,
    filter: Option<&str>,
    desired_rate: Option<u32>,
    fallback_allowed: bool,
) -> Result<ProbedOutput, String> {
    open_output(host, filter, desired_rate, false, fallback_allowed)
        .map(|output| ProbedOutput { output })
        .ok_or_else(|| "No audio output device available.".to_string())
}

fn start_session(
    host: &cpal::Host,
    engine_audio: SynthEngineAudio,
    config: &AudioConfig,
    options: &SessionOptions,
) -> Result<(AudioSession, SessionInfo), String> {
    let probed = probe_session(host, config, options.fallback_allowed)?;
    let session = build_session(host, engine_audio, config, &probed, options)?;
    Ok((session, session_info(config, &probed)))
}

fn build_session(
    host: &cpal::Host,
    engine_audio: SynthEngineAudio,
    config: &AudioConfig,
    probed: &ProbedSession,
    options: &SessionOptions,
) -> Result<AudioSession, String> {
    let output = &probed.output;
    let output_name = device_name(&output.device);

    log_audio(&format!(
        "Configuring output \"{output_name}\" at {} Hz",
        probed.sample_rate
    ));
    if !stream_builds(&output.device, &output.config) {
        return Err(format!(
            "Output \"{output_name}\" refused {} Hz",
            probed.sample_rate
        ));
    }
    wait_for_device_settle();

    let (input_stream, input_consumer) = if let Some(filter) = config.input_device.as_deref() {
        log_audio(&format!(
            "Opening input \"{filter}\" at {} Hz",
            probed.sample_rate
        ));
        let input = open_input(
            host,
            filter,
            probed.sample_rate,
            output.channels,
            options.disconnected.clone(),
            options.session_generation.clone(),
        )
        .ok_or_else(|| format!("Failed to open audio input \"{filter}\"."))?;
        (Some(input.stream), Some(input.consumer))
    } else {
        (None, None)
    };

    let renderer = Renderer::new(
        engine_audio,
        output.sample_rate,
        output.channels,
        input_consumer,
        config.filter_oversampling,
        config.filter_type,
    );
    let output_stream = build_output_stream(
        &output.device,
        output.config.clone(),
        renderer,
        options.disconnected.clone(),
        options.session_generation.clone(),
    )
    .ok_or_else(|| "Failed to build audio output stream.".to_string())?;

    log_audio("Starting output stream");
    output_stream
        .play()
        .map_err(|err| format!("Failed to start audio output stream: {err}"))?;

    if input_stream.is_some() {
        log_audio("Waiting for output clock to settle before starting input");
        wait_for_device_settle();
        if let Some(ref stream) = input_stream {
            log_audio("Starting input stream");
            stream
                .play()
                .map_err(|err| format!("Failed to start audio input stream: {err}"))?;
        }
    }

    match options.mode {
        SessionMode::Initial => {
            log_audio(&format!("Audio session ready at {} Hz", probed.sample_rate))
        }
        SessionMode::Restart => log_audio(&format!(
            "Audio restart complete at {} Hz",
            probed.sample_rate
        )),
        SessionMode::Recovery => {
            log_audio(&format!("Audio recovered at {} Hz", probed.sample_rate))
        }
    }

    Ok(AudioSession(input_stream, output_stream))
}

// ============================================================================
// Stream setup
// ============================================================================

// --- output ---------------------------------------------------------------

/// A resolved output device and its negotiated stream configuration.
struct Output {
    device: cpal::Device,
    config: SupportedStreamConfig,
    sample_rate: f32,
    channels: usize,
    sample_rate_setting: Option<u32>,
}

fn sample_rate_setting_for_ui(requested: Option<u32>, actual_hz: u32) -> Option<u32> {
    match requested {
        None => None,
        Some(requested_hz) if requested_hz == actual_hz => Some(requested_hz),
        Some(_) => Some(actual_hz),
    }
}

/// Resolves the output device (by name filter, else default) and its config.
fn open_output(
    host: &cpal::Host,
    filter: Option<&str>,
    desired_rate: Option<u32>,
    verify_build: bool,
    fallback_allowed: bool,
) -> Option<Output> {
    let devices: Vec<cpal::Device> = host
        .output_devices()
        .map(|devices| devices.collect())
        .unwrap_or_default();

    let device = match (filter, fallback_allowed) {
        (Some(filter), true) => find_device(&devices, filter).or_else(|| {
            log_audio(&format!(
                "No audio output matching \"{filter}\"; falling back to system default"
            ));
            host.default_output_device()
        }),
        (Some(filter), false) => find_device(&devices, filter).or_else(|| {
            log_audio(&format!(
                "Audio output \"{filter}\" not available — waiting for device"
            ));
            None
        }),
        (None, _) => host.default_output_device(),
    }?;

    let config = choose_output_config(&device, desired_rate, verify_build)?;
    let sample_rate = config.sample_rate() as f32;
    let actual_hz = sample_rate as u32;
    let channels = config.channels() as usize;
    let sample_rate_setting = sample_rate_setting_for_ui(desired_rate, actual_hz);

    Some(Output {
        device,
        config,
        sample_rate,
        channels,
        sample_rate_setting,
    })
}

fn log_available_outputs(host: &cpal::Host) {
    let devices: Vec<cpal::Device> = host
        .output_devices()
        .map(|devices| devices.collect())
        .unwrap_or_default();
    log_audio("Available audio outputs:");
    for (index, device) in devices.iter().enumerate() {
        log_audio(&format!("  [{index}] {}", device_name(device)));
    }
}

/// Picks a stereo F32 output config, preferring `desired_rate` when supported
/// and otherwise falling back to the device default.
fn choose_output_config(
    device: &cpal::Device,
    desired_rate: Option<u32>,
    verify_build: bool,
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
            if !verify_build || stream_builds(device, &config) {
                return Some(config);
            }
            log_audio(&format!(
                "Device advertises {rate} Hz but refused to switch to it \
                 (likely clock-locked); using device default"
            ));
        } else {
            log_audio(&format!(
                "Requested sample rate {rate} Hz unsupported; using device default"
            ));
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
    disconnected: Arc<AtomicBool>,
    session_generation: Arc<AtomicU64>,
) -> Option<cpal::Stream> {
    let my_gen = session_generation.load(Ordering::SeqCst);
    device
        .build_output_stream(
            config.into(),
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| renderer.render(data),
            move |err| {
                log_audio(&format!("output stream error: {err}"));
                if session_generation.load(Ordering::SeqCst) == my_gen {
                    disconnected.store(true, Ordering::SeqCst);
                }
            },
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
    disconnected: Arc<AtomicBool>,
    session_generation: Arc<AtomicU64>,
) -> Option<Input> {
    let devices: Vec<cpal::Device> = host
        .input_devices()
        .map(|devices| devices.collect())
        .unwrap_or_default();

    let Some(device) = find_device(&devices, filter) else {
        log_audio(&format!(
            "No audio input matching \"{filter}\"; input disabled"
        ));
        return None;
    };

    let Some(config) = choose_input_config(&device, sample_rate) else {
        log_audio(&format!(
            "Input \"{}\" cannot run at {sample_rate} Hz (must match output rate); input disabled",
            device_name(&device)
        ));
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
    let stream = build_input_stream(&device, config, capture, disconnected, session_generation)?;

    log_audio(&format!(
        "Input \"{}\" configured at {sample_rate} Hz, {}ch",
        device_name(&device),
        in_channels
    ));
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
    disconnected: Arc<AtomicBool>,
    session_generation: Arc<AtomicU64>,
) -> Option<cpal::Stream> {
    let my_gen = session_generation.load(Ordering::SeqCst);
    device
        .build_input_stream(
            config.into(),
            move |data: &[f32], _info: &cpal::InputCallbackInfo| capture.capture(data),
            move |err| {
                log_audio(&format!("input stream error: {err}"));
                if session_generation.load(Ordering::SeqCst) == my_gen {
                    disconnected.store(true, Ordering::SeqCst);
                }
            },
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
            "    {}ch {:?} {}-{} Hz",
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
    engine: SynthEngineWithMemory<VOICE_PACKS, Box<[f32]>>,
    timing: AudioTiming,
    input: Option<rtrb::Consumer<f32>>,
    input_enabled: Arc<AtomicBool>,
    sample_rate: f32,
    channels: usize,
    last_midi_clock_status: synth_core::MidiClockStatus,
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
        // Stereo delays need one second of history per channel to match the
        // Rev2's documented maximum. Heap storage avoids a large audio-thread
        // stack object at high host sample rates.
        let effects_memory = vec![0.0; sample_rate.max(1.0) as usize * 2].into_boxed_slice();
        let mut engine = SynthEngineWithMemory::<VOICE_PACKS, _>::new_with_effects_memory(
            sample_rate,
            effects_memory,
        );
        engine.set_filter_oversampling(filter_oversampling);
        engine.set_filter_type(filter_type);
        log_audio(&format!(
            "Filter oversampling: {:?} ({}x)",
            filter_oversampling,
            filter_oversampling.factor(sample_rate)
        ));
        let last_midi_clock_status = engine.midi_clock_status();
        Self {
            engine_audio,
            engine,
            timing: AudioTiming::default(),
            input,
            input_enabled,
            sample_rate,
            channels,
            last_midi_clock_status,
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

        let clock_status = self.engine.midi_clock_status();
        if clock_status != self.last_midi_clock_status
            && self.engine_audio.feedback.push_midi_clock(clock_status)
        {
            self.last_midi_clock_status = clock_status;
        }

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
