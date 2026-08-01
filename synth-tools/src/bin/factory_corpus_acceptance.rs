//! Complete Rev2 factory-corpus acceptance for layered playback.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

use synth_core::{
    ControlMessage, LayerId, LayerMode, Patch, SynthEngineWithMemory, VOICE_PACKS,
    dsp::{FilterOversampling, FilterType},
    midi::rev2::{PROGRAM_DATA_SYSEX_LEN, decode},
};

const SAMPLE_RATE: f32 = 48_000.0;
const CALLBACK_FRAMES: usize = 48;
const MEASURE_BLOCKS: usize = 256;
const TRANSITION_BLOCKS: usize = 24;
const EFFECT_SAMPLES_PER_LAYER: usize = 48_000 * 2;
const NOTES: [u8; 4] = [48, 55, 64, 72];
const FILTERS: [FilterType; 2] = [FilterType::GainLimitedTpt, FilterType::HuovilainenLadder];

type ConstrainedEngine = SynthEngineWithMemory<Vec<f32>, { VOICE_PACKS }, 1>;
type DesktopEngine = SynthEngineWithMemory<Vec<f32>, { VOICE_PACKS }, 2>;

#[derive(Clone, Copy, Default)]
struct Metrics {
    peak: f32,
    square_sum: f64,
    samples: u64,
    active_a: usize,
    active_b: usize,
    non_finite: u64,
    limiter_engaged: bool,
    callback_max_ns: u128,
}

impl Metrics {
    fn observe<const LAYERS: usize>(
        &mut self,
        engine: &SynthEngineWithMemory<Vec<f32>, { VOICE_PACKS }, LAYERS>,
        output: &[f32],
        elapsed_ns: u128,
    ) {
        self.callback_max_ns = self.callback_max_ns.max(elapsed_ns);
        self.active_a = self
            .active_a
            .max(engine.layer_active_voice_count(LayerId::A));
        self.active_b = self
            .active_b
            .max(engine.layer_active_voice_count(LayerId::B));
        self.limiter_engaged |= engine.output_limiter_engaged();
        for sample in output.iter().copied() {
            if sample.is_finite() {
                self.peak = self.peak.max(sample.abs());
                self.square_sum += f64::from(sample) * f64::from(sample);
                self.samples += 1;
            } else {
                self.non_finite += 1;
            }
        }
    }

    fn merge(&mut self, other: Self) {
        self.peak = self.peak.max(other.peak);
        self.square_sum += other.square_sum;
        self.samples += other.samples;
        self.active_a = self.active_a.max(other.active_a);
        self.active_b = self.active_b.max(other.active_b);
        self.non_finite += other.non_finite;
        self.limiter_engaged |= other.limiter_engaged;
        self.callback_max_ns = self.callback_max_ns.max(other.callback_max_ns);
    }

    fn rms(self) -> f32 {
        (self.square_sum / self.samples.max(1) as f64).sqrt() as f32
    }
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let source = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("Prophet-Rev2-Factory-Programs/Rev2_Programs_v1.0.syx"));
    let report = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/factory-corpus.csv"));

    let bytes = fs::read(&source).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", source.display());
    });
    assert_eq!(
        bytes.len(),
        512 * PROGRAM_DATA_SYSEX_LEN,
        "factory corpus must contain exactly 512 complete Rev2 programs"
    );

    let mut csv = String::from(
        "bank,program,mode,layer_a,layer_b,peak,rms,active_a,active_b,non_finite,limiter_engaged,callback_max_ns\n",
    );
    let mut mode_counts = [0_usize; 3];
    let mut silent_programs = 0_usize;
    let mut limiter_programs = 0_usize;
    let mut corpus_peak = 0.0_f32;
    let mut corpus_callback_max_ns = 0_u128;

    for (index, message) in bytes.chunks_exact(PROGRAM_DATA_SYSEX_LEN).enumerate() {
        let decoded = decode::program_data(message)
            .unwrap_or_else(|error| panic!("program {index} failed decode: {error:?}"));
        assert_eq!(
            usize::from(decoded.bank) * 128 + usize::from(decoded.program),
            index
        );
        let patch = decoded.patch;
        mode_counts[mode_index(patch.mode)] += 1;
        assert!(
            !patch.layer_a.name.is_empty(),
            "program {index} has no Layer A name"
        );
        assert!(
            !patch.layer_b.name.is_empty(),
            "program {index} has no Layer B name"
        );

        let mut metrics = Metrics::default();
        for filter in FILTERS {
            metrics.merge(measure_constrained(&patch, LayerId::A, filter));
            metrics.merge(measure_constrained(&patch, LayerId::B, filter));
            metrics.merge(measure_stored_mode_and_transitions(&patch, filter));
        }

        assert_eq!(
            metrics.non_finite, 0,
            "program {index} produced non-finite audio"
        );
        assert!(
            metrics.active_a > 0,
            "program {index} never activated Layer A"
        );
        assert!(
            metrics.active_b > 0,
            "program {index} never activated Layer B"
        );
        assert!(
            metrics.peak <= 1.0,
            "program {index} exceeded the output bound"
        );
        if metrics.rms() <= 1.0e-7 {
            silent_programs += 1;
        }
        limiter_programs += usize::from(metrics.limiter_engaged);
        corpus_peak = corpus_peak.max(metrics.peak);
        corpus_callback_max_ns = corpus_callback_max_ns.max(metrics.callback_max_ns);
        csv.push_str(&format!(
            "{},{},{},{:?},{:?},{:.9},{:.9},{},{},{},{},{}\n",
            decoded.bank + 1,
            decoded.program + 1,
            mode_name(patch.mode),
            patch.layer_a.name.as_str(),
            patch.layer_b.name.as_str(),
            metrics.peak,
            metrics.rms(),
            metrics.active_a,
            metrics.active_b,
            metrics.non_finite,
            metrics.limiter_engaged,
            metrics.callback_max_ns,
        ));
    }

    assert_eq!(mode_counts, [174, 266, 72]);
    validate_reported_presets(&bytes);
    write_report(&report, &csv);
    println!(
        "factory corpus passed: programs=512 modes=174/266/72 filters=2 silent={} limiter={} peak={:.6} callback_max_ns={} report={}",
        silent_programs,
        limiter_programs,
        corpus_peak,
        corpus_callback_max_ns,
        report.display()
    );
}

fn configured_constrained(patch: &Patch, filter: FilterType) -> ConstrainedEngine {
    let mut engine = ConstrainedEngine::new_with_effects_memory(
        SAMPLE_RATE,
        vec![0.0; EFFECT_SAMPLES_PER_LAYER],
    )
    .expect("one-layer effects layout is valid");
    engine.set_filter_type(filter);
    engine.set_filter_oversampling(FilterOversampling::Auto);
    engine.apply_patch(patch);
    engine
}

fn configured_desktop(patch: &Patch, filter: FilterType) -> DesktopEngine {
    let mut engine = DesktopEngine::new_with_effects_memory(
        SAMPLE_RATE,
        vec![0.0; EFFECT_SAMPLES_PER_LAYER * 2],
    )
    .expect("two-layer effects layout is valid");
    engine.set_filter_type(filter);
    engine.set_filter_oversampling(FilterOversampling::Auto);
    engine.apply_patch(patch);
    engine
}

fn measure_constrained(patch: &Patch, selected: LayerId, filter: FilterType) -> Metrics {
    let mut engine = configured_constrained(patch, filter);
    if selected == LayerId::B {
        engine.handle_control(ControlMessage::SetEditLayer(LayerId::B));
        let _ = render(&mut engine, TRANSITION_BLOCKS);
    }
    for note in NOTES {
        engine.note_on(note, 1.0);
    }
    let metrics = render(&mut engine, MEASURE_BLOCKS);
    let status = engine.playback_status();
    assert_eq!(status.edit_layer, selected);
    assert_eq!(
        status.rendered_mask,
        if selected == LayerId::A { 0b01 } else { 0b10 }
    );
    assert_eq!(status.degraded, patch.mode != LayerMode::Normal);
    assert_eq!(engine.layer_active_voice_count(selected.other()), 0);
    metrics
}

fn measure_stored_mode_and_transitions(patch: &Patch, filter: FilterType) -> Metrics {
    let mut engine = configured_desktop(patch, filter);
    for note in NOTES {
        engine.note_on(note, 1.0);
    }
    let mut metrics = render(&mut engine, MEASURE_BLOCKS);
    assert_eq!(engine.playback_status().mode, patch.mode);
    assert!(!engine.playback_status().degraded);

    for layer in [LayerId::B, LayerId::A] {
        engine.handle_control(ControlMessage::SetEditLayer(layer));
        metrics.merge(render(&mut engine, TRANSITION_BLOCKS));
    }
    for mode in [
        LayerMode::Normal,
        LayerMode::Stack,
        LayerMode::Split,
        patch.mode,
    ] {
        engine.handle_control(ControlMessage::SetLayerMode(mode));
        metrics.merge(render(&mut engine, TRANSITION_BLOCKS));
    }
    engine.all_notes_off();
    engine.sustain_pedal(false);
    metrics.merge(render(&mut engine, TRANSITION_BLOCKS));
    metrics
}

fn render<const LAYERS: usize>(
    engine: &mut SynthEngineWithMemory<Vec<f32>, { VOICE_PACKS }, LAYERS>,
    blocks: usize,
) -> Metrics {
    let mut metrics = Metrics::default();
    let mut output = [0.0_f32; CALLBACK_FRAMES * 2];
    for _ in 0..blocks {
        let started = Instant::now();
        engine.process_interleaved(&mut output, 2);
        metrics.observe(engine, &output, started.elapsed().as_nanos());
    }
    metrics
}

fn validate_reported_presets(bytes: &[u8]) {
    let cases = [
        (1, "All That Glitter", "All That Glitter B"),
        (5, "BoiteMusique", "BoiteMusique"),
        (18, "Horn Busker", "League Brass"),
        (37, "Sitcom Piano", "Sitcom Pad"),
    ];
    for (program, expected_a, expected_b) in cases {
        let offset = program * PROGRAM_DATA_SYSEX_LEN;
        let patch = decode::program_data(&bytes[offset..offset + PROGRAM_DATA_SYSEX_LEN])
            .unwrap()
            .patch;
        assert_eq!(patch.layer_a.name.as_str(), expected_a);
        assert_eq!(patch.layer_b.name.as_str(), expected_b);
        let frequency = dominant_frequency(&patch);
        assert!(
            (20.0..20_000.0).contains(&frequency),
            "reported preset {expected_a} has invalid dominant frequency {frequency}"
        );
        println!(
            "reported preset passed: F1-{:03} A={:?} B={:?} dominant_hz={:.2}",
            program + 1,
            expected_a,
            expected_b,
            frequency
        );
    }
}

fn dominant_frequency(patch: &Patch) -> f32 {
    const FRAMES: usize = 48_000;
    const FFT_LEN: usize = 32_768;
    let mut engine = configured_desktop(patch, FilterType::GainLimitedTpt);
    engine.note_on(60, 1.0);
    let mut interleaved = vec![0.0_f32; FRAMES * 2];
    engine.process_interleaved(&mut interleaved, 2);
    let start = FRAMES - FFT_LEN;
    let mut spectrum = Vec::with_capacity(FFT_LEN);
    for index in 0..FFT_LEN {
        let phase = index as f32 / (FFT_LEN - 1) as f32;
        let window = 0.5 - 0.5 * (core::f32::consts::TAU * phase).cos();
        spectrum.push(Complex32::new(
            interleaved[(start + index) * 2] * window,
            0.0,
        ));
    }
    FftPlanner::<f32>::new()
        .plan_fft_forward(FFT_LEN)
        .process(&mut spectrum);
    let min_bin = (20.0 * FFT_LEN as f32 / SAMPLE_RATE) as usize;
    let max_bin = (20_000.0 * FFT_LEN as f32 / SAMPLE_RATE) as usize;
    let (bin, power) = spectrum[min_bin..=max_bin]
        .iter()
        .enumerate()
        .map(|(index, value)| (index + min_bin, value.norm_sqr()))
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .unwrap();
    assert!(power > 1.0e-12, "reported preset spectrum is silent");
    bin as f32 * SAMPLE_RATE / FFT_LEN as f32
}

fn write_report(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap_or_else(|error| {
        panic!("failed to write {}: {error}", path.display());
    });
}

const fn mode_index(mode: LayerMode) -> usize {
    match mode {
        LayerMode::Normal => 0,
        LayerMode::Stack => 1,
        LayerMode::Split => 2,
    }
}

const fn mode_name(mode: LayerMode) -> &'static str {
    match mode {
        LayerMode::Normal => "normal",
        LayerMode::Stack => "stack",
        LayerMode::Split => "split",
    }
}

trait OtherLayer {
    fn other(self) -> Self;
}

impl OtherLayer for LayerId {
    fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}
