use std::{
    env, fs,
    hint::spin_loop,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use serde_json::json;

use synth_core::dsp::{FilterType, Waveform};
use synth_core::{BankId, OscillatorEngineType, ParamId, SynthEngineWithMemory};

const PACKS: usize = 4;
const BLOCK_FRAMES: usize = 64;
const WARMUP_BLOCKS: usize = 64;
const MEASURED_BLOCKS: usize = 2_000;
const SAMPLE_RATES: [f32; 4] = [44_100.0, 48_000.0, 96_000.0, 192_000.0];
const VOICE_COUNTS: [usize; 3] = [1, 4, 16];
const NOTES: [u8; 16] = [
    36, 40, 43, 47, 50, 53, 55, 59, 62, 64, 67, 71, 74, 77, 79, 83,
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (output, soak_seconds, banks) = parse_args()?;
    let thread_priority = configure_benchmark_thread()?;
    let mut bank_reports = Vec::new();
    for bank in banks {
        let mut cases = Vec::new();
        for sample_rate_hz in SAMPLE_RATES {
            for voice_count in VOICE_COUNTS {
                cases.push(measure_case(bank, sample_rate_hz, voice_count));
            }
        }
        let soak = soak(bank, soak_seconds);
        bank_reports.push(json!({
            "profile_id": bank_profile_id(bank),
            "bank_id": bank.id(),
            "cases": cases,
            "soak": soak,
        }));
    }
    let report = json!({
        "schema_version": 2,
        "block_frames": BLOCK_FRAMES,
        "measured_blocks_per_case": MEASURED_BLOCKS,
        "thread_priority": thread_priority,
        "banks": bank_reports,
    });
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_string_pretty(&report)? + "\n")?;
    println!("wrote {}", output.display());
    Ok(())
}

fn bank_profile_id(bank: BankId) -> &'static str {
    match bank {
        BankId::Monologue => "korg-monologue-measured-wavetable-v2",
        BankId::Prophet5 => "prophet5-wavetable-bank-v2",
    }
}

fn measure_case(bank: BankId, sample_rate_hz: f32, voice_count: usize) -> serde_json::Value {
    let mut engine = configured_engine(bank, sample_rate_hz, voice_count, false);
    let mut block = [0.0_f32; BLOCK_FRAMES];
    for _ in 0..WARMUP_BLOCKS {
        engine.process_interleaved(&mut block, 1);
    }
    let mut times = Vec::with_capacity(MEASURED_BLOCKS);
    let mut finite = true;
    for _ in 0..MEASURED_BLOCKS {
        let started = Instant::now();
        engine.process_interleaved(&mut block, 1);
        times.push(started.elapsed().as_nanos() as f64 / BLOCK_FRAMES as f64);
        finite &= block.iter().all(|sample| sample.is_finite());
    }
    times.sort_by(f64::total_cmp);
    let p50 = percentile(&times, 0.50);
    let p95 = percentile(&times, 0.95);
    let p99 = percentile(&times, 0.99);
    let frame_budget_ns = 1_000_000_000.0 / f64::from(sample_rate_hz);
    println!(
        "bank={} rate={sample_rate_hz:.0} voices={voice_count} p99={p99:.1}ns ({:.2}% frame)",
        bank.id(),
        p99 / frame_budget_ns * 100.0
    );
    json!({
        "sample_rate_hz": sample_rate_hz,
        "active_voices": voice_count,
        "nanoseconds_per_frame_p50": p50,
        "nanoseconds_per_frame_p95": p95,
        "nanoseconds_per_frame_p99": p99,
        "realtime_budget_fraction_p99": p99 / frame_budget_ns,
        "finite": finite,
    })
}

fn soak(bank: BankId, seconds: f32) -> serde_json::Value {
    let sample_rate_hz = 48_000.0;
    let block_count = (seconds * sample_rate_hz / BLOCK_FRAMES as f32).round() as usize;
    let deadline_ns = 1_000_000_000.0 * BLOCK_FRAMES as f64 / f64::from(sample_rate_hz);
    let mut engine = configured_engine(bank, sample_rate_hz, 16, true);
    let mut block = [0.0_f32; BLOCK_FRAMES];
    let mut missed_deadlines = 0usize;
    let mut finite = true;
    let mut times = Vec::with_capacity(block_count);
    let block_duration = Duration::from_secs_f64(deadline_ns / 1_000_000_000.0);
    let wall_started = Instant::now();
    let mut next_block = wall_started;
    for block_index in 0..block_count {
        if block_index > 0 {
            next_block += block_duration;
            wait_until(next_block);
            if Instant::now() > next_block + block_duration {
                next_block = Instant::now();
            }
        }
        let shape = 0.5
            + 0.49 * (std::f32::consts::TAU * block_index as f32 / (sample_rate_hz * 0.37)).sin();
        engine.set_param(ParamId::Osc1ShapeMod, shape);
        let started = Instant::now();
        engine.process_interleaved(&mut block, 1);
        let elapsed_ns = started.elapsed().as_nanos() as f64;
        times.push(elapsed_ns);
        missed_deadlines += usize::from(elapsed_ns > deadline_ns);
        finite &= block.iter().all(|sample| sample.is_finite());
    }
    times.sort_by(f64::total_cmp);
    let p99 = percentile(&times, 0.99);
    let maximum = times.last().copied().unwrap_or(0.0);
    let wall_seconds = wall_started.elapsed().as_secs_f64();
    println!(
        "bank={} soak={seconds:.1}s blocks={block_count} p99={:.2}% max={:.2}% missed_deadlines={missed_deadlines} finite={finite}",
        bank.id(),
        p99 / deadline_ns * 100.0,
        maximum / deadline_ns * 100.0,
    );
    json!({
        "sample_rate_hz": sample_rate_hz,
        "active_voices": 16,
        "requested_audio_seconds": seconds,
        "wall_seconds": wall_seconds,
        "blocks": block_count,
        "pacing": "real-time blocks; render duration measured after wake",
        "render_nanoseconds_p99": p99,
        "render_nanoseconds_maximum": maximum,
        "render_budget_fraction_p99": p99 / deadline_ns,
        "render_budget_fraction_maximum": maximum / deadline_ns,
        "missed_deadlines": missed_deadlines,
        "finite": finite,
    })
}

fn wait_until(deadline: Instant) {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        if remaining > Duration::from_micros(100) {
            thread::sleep(remaining - Duration::from_micros(50));
        } else {
            spin_loop();
        }
    }
}

#[cfg(target_os = "macos")]
fn configure_benchmark_thread() -> Result<&'static str, Box<dyn std::error::Error>> {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }
    const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
    let status = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
    if status != 0 {
        return Err(format!("pthread_set_qos_class_self_np failed with status {status}").into());
    }
    Ok("macos-user-interactive")
}

#[cfg(not(target_os = "macos"))]
fn configure_benchmark_thread() -> Result<&'static str, Box<dyn std::error::Error>> {
    Ok("platform-default")
}

fn configured_engine(
    bank: BankId,
    sample_rate_hz: f32,
    voice_count: usize,
    slop: bool,
) -> SynthEngineWithMemory<Box<[f32]>, PACKS> {
    let effects = vec![0.0; (sample_rate_hz as usize * 2).max(96_000)].into_boxed_slice();
    let mut engine =
        SynthEngineWithMemory::<_, PACKS>::new_with_effects_memory(sample_rate_hz, effects)
            .expect("valid effects memory");
    engine.set_filter_type(FilterType::PassThrough);
    engine.set_wavetable_bank(bank);
    engine.set_oscillator_engine(OscillatorEngineType::Wavetable);
    engine.set_param(ParamId::AmpEgAttack, 0.0);
    engine.set_param(ParamId::AmpEgDecay, 0.0);
    engine.set_param(ParamId::AmpEgSustain, 1.0);
    engine.set_param(ParamId::AmpEgRelease, 0.0);
    engine.set_param(ParamId::Osc1Enabled, 1.0);
    engine.set_param(ParamId::Osc2Enabled, 0.0);
    engine.set_param(ParamId::OscMix, 0.0);
    engine.set_param(ParamId::Osc1Waveform, Waveform::Pulse.index() as f32);
    engine.set_param(ParamId::Osc1ShapeMod, 0.35);
    engine.set_param(ParamId::OscSlop, if slop { 1.0 } else { 0.0 });
    for note in NOTES.iter().take(voice_count) {
        engine.note_on(*note, 1.0);
    }
    engine
}

fn percentile(sorted: &[f64], amount: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * amount).round() as usize]
}

fn parse_args() -> Result<(PathBuf, f32, Vec<BankId>), Box<dyn std::error::Error>> {
    let mut output = PathBuf::from("target/analog-osc/multirate-wavetable/runtime-v2.json");
    let mut soak_seconds: f32 = 60.0;
    let mut banks = vec![BankId::Monologue, BankId::Prophet5];
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--output" => output = PathBuf::from(args.next().ok_or("missing --output value")?),
            "--soak-seconds" => {
                soak_seconds = args.next().ok_or("missing --soak-seconds value")?.parse()?;
            }
            "--bank" => {
                let value = args.next().ok_or("missing --bank value")?;
                banks = match value.as_str() {
                    "all" => vec![BankId::Monologue, BankId::Prophet5],
                    "monologue" => vec![BankId::Monologue],
                    "prophet5" => vec![BankId::Prophet5],
                    _ => return Err(format!("unknown --bank value: {value}").into()),
                };
            }
            "--help" | "-h" => {
                println!(
                    "wavetable_multirate_benchmark [--output FILE] [--soak-seconds SECONDS] [--bank all|monologue|prophet5]"
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }
    if !soak_seconds.is_finite() || soak_seconds < 0.0 {
        return Err("--soak-seconds must be finite and non-negative".into());
    }
    Ok((output, soak_seconds, banks))
}
