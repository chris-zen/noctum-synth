use eframe::egui;
use rustfft::{FftPlanner, num_complex::Complex32};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use synth_core::{LANES, LadderFilter, filter::SELF_OSC_RESONANCE_START};
use wide::f32x4;

use super::spectrum::{self, SpectrumConfig};
use serde::{Deserialize, Serialize};

/// Small impulse used to keep analysis mostly in the filter's linear region.
const ANALYSIS_IMPULSE_GAIN: f32 = 1.0e-4;
/// Sine probe level used for self-oscillating response measurement.
const ANALYSIS_SINE_GAIN: f32 = 0.02;
/// Lowest cutoff shown and analyzed by the filter design view.
const MIN_CUTOFF_HZ: f32 = 20.0;
/// Highest cutoff shown and analyzed by the filter design view.
const MAX_CUTOFF_HZ: f32 = 18_000.0;
/// Samples discarded before measuring each self-oscillating sine probe.
const SINE_PROBE_SETTLE_FRAMES: usize = 512;
/// Samples integrated to measure each self-oscillating sine probe.
const SINE_PROBE_MEASURE_FRAMES: usize = 2048;
/// Fractional-octave width used by the display-only smoothing option.
const SMOOTHING_FRACTIONAL_OCTAVE: f32 = 12.0;
/// Space reserved below the plot for the non-overlapping hover readout.
const HOVER_READOUT_H: f32 = 24.0;

pub struct FilterDesignState {
    pub cutoff: f32,
    pub resonance: f32,
    pub poles: u8,

    pub response_db: Vec<f32>,
    raw_response_db: Vec<f32>,
    smoothed_response_db: Vec<f32>,
    pub peak_freq_hz: f32,
    pub peak_db: f32,
    pub fft_size: usize,
    pub db_top: f32,
    pub db_floor: f32,
    pub log_scale: bool,
    pub sample_rate: f32,
    pub smooth_response: bool,

    pub live_mode: bool,
    pub needs_render: bool,
    pub last_params_hash: u64,
    pending_response: Option<Receiver<AnalysisResult>>,
}

#[derive(Clone, Copy)]
struct AnalysisParams {
    cutoff: f32,
    resonance: f32,
    poles: u8,
    fft_size: usize,
    sample_rate: f32,
    db_floor: f32,
}

struct AnalysisResult {
    hash: u64,
    response_db: Vec<f32>,
    peak_freq_hz: f32,
    peak_db: f32,
}

impl Default for FilterDesignState {
    fn default() -> Self {
        Self {
            cutoff: MAX_CUTOFF_HZ,
            resonance: 0.0,
            poles: 4,
            response_db: Vec::new(),
            raw_response_db: Vec::new(),
            smoothed_response_db: Vec::new(),
            peak_freq_hz: 0.0,
            peak_db: 0.0,
            fft_size: 4096,
            db_top: 24.0,
            db_floor: -96.0,
            log_scale: true,
            sample_rate: 44100.0,
            smooth_response: true,
            live_mode: true,
            needs_render: true,
            last_params_hash: 0,
            pending_response: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct FilterDesignViewConfig {
    pub cutoff: f32,
    pub resonance: f32,
    pub poles: u8,
    pub fft_size: usize,
    pub db_top: f32,
    pub db_floor: f32,
    pub log_scale: bool,
    pub sample_rate: f32,
    pub smooth_response: bool,
    pub live_mode: bool,
}

impl Default for FilterDesignViewConfig {
    fn default() -> Self {
        Self::from_state(&FilterDesignState::default())
    }
}

impl FilterDesignViewConfig {
    pub fn from_state(state: &FilterDesignState) -> Self {
        Self {
            cutoff: state.cutoff,
            resonance: state.resonance,
            poles: state.poles,
            fft_size: state.fft_size,
            db_top: state.db_top,
            db_floor: state.db_floor,
            log_scale: state.log_scale,
            sample_rate: state.sample_rate,
            smooth_response: state.smooth_response,
            live_mode: state.live_mode,
        }
    }

    pub fn apply_to(&self, state: &mut FilterDesignState) {
        state.cutoff = migrate_cutoff_to_hz(self.cutoff);
        state.resonance = self.resonance;
        state.poles = self.poles;
        state.fft_size = self.fft_size;
        state.db_top = self.db_top;
        state.db_floor = self.db_floor;
        state.log_scale = self.log_scale;
        state.sample_rate = self.sample_rate;
        state.smooth_response = self.smooth_response;
        state.live_mode = self.live_mode;
        state.needs_render = true;
        state.pending_response = None;
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut FilterDesignState) {
    let old_hash = param_hash(state);

    // ---- Filter parameters ----
    ui.horizontal(|ui| {
        ui.label("Cutoff:");
        let mut cutoff_norm = cutoff_hz_to_normalized(state.cutoff);
        if ui
            .add(egui::Slider::new(&mut cutoff_norm, 0.0..=1.0).text(""))
            .changed()
        {
            state.cutoff = normalized_to_cutoff_hz(cutoff_norm);
        } else {
            state.cutoff = state.cutoff.clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
        }
        ui.label(format_hz(state.cutoff));
        ui.separator();
        ui.label("Resonance:");
        ui.add(egui::Slider::new(&mut state.resonance, 0.0..=1.0).text(""));
    });

    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Poles:");
        for (index, name) in ["2", "4"].iter().enumerate() {
            let poles_value = if index == 0 { 2u8 } else { 4u8 };
            if ui
                .selectable_label(state.poles == poles_value, *name)
                .clicked()
            {
                state.poles = poles_value;
            }
        }
    });

    ui.add_space(4.0);

    // ---- Render controls ----
    ui.horizontal(|ui| {
        if ui.button("Refresh").clicked() {
            state.needs_render = true;
        }
        ui.checkbox(&mut state.live_mode, "Live");
        ui.separator();
        ui.label("SR:");
        for &(label, sr) in &[("44.1k", 44100.0), ("96k", 96000.0), ("192k", 192000.0)] {
            if ui
                .selectable_label(state.sample_rate == sr, label)
                .clicked()
            {
                state.sample_rate = sr;
            }
        }
    });

    ui.add_space(8.0);

    ui.strong("Frequency Response");
    ui.add_space(4.0);

    // ---- Graph controls ----
    ui.horizontal(|ui| {
        ui.label("FFT:");
        for &size in &[1024, 2048, 4096, 8192] {
            if ui
                .selectable_label(state.fft_size == size, &size.to_string())
                .clicked()
            {
                state.fft_size = size;
                state.needs_render = true;
            }
        }
        ui.separator();
        ui.label("dB top:");
        for &db in &[48.0, 24.0, 12.0, 6.0, 0.0] {
            if ui
                .selectable_label(state.db_top == db, &format!("+{:.0}", db))
                .clicked()
            {
                state.db_top = db;
            }
        }
        ui.separator();
        ui.label("floor:");
        for &(db, label) in &[(-60.0, "-60"), (-96.0, "-96"), (-144.0, "-∞")] {
            if ui.selectable_label(state.db_floor == db, label).clicked() {
                state.db_floor = db;
            }
        }
        ui.separator();
        if ui.selectable_label(state.log_scale, "Log").clicked() {
            state.log_scale = !state.log_scale;
        }
        ui.separator();
        if ui
            .selectable_label(state.smooth_response, "Smooth")
            .on_hover_text("Smooth the displayed curve without changing the measured response.")
            .clicked()
        {
            state.smooth_response = !state.smooth_response;
            update_display_response(state);
        }
        ui.separator();
        ui.label(format!(
            "Peak: {:.1} dB @ {}",
            state.peak_db,
            format_hz(state.peak_freq_hz)
        ));
        if real_self_oscillation_analysis(state) {
            ui.separator();
            ui.colored_label(egui::Color32::from_rgb(255, 180, 90), "Self osc")
                .on_hover_text("The plot is measuring the nonlinear self-oscillating filter path.");
        }
    });

    // ---- Render scheduling ----
    poll_pending_response(ui, state);

    let new_hash = param_hash(state);
    if new_hash != old_hash {
        state.last_params_hash = new_hash;
        if state.live_mode {
            state.needs_render = true;
        }
    }

    if state.raw_response_db.is_empty() {
        state.needs_render = true;
    }

    if state.needs_render {
        start_response_job(state, new_hash);
    }
    if state.pending_response.is_some() {
        ui.ctx().request_repaint();
    }

    // ---- Frequency response graph ----
    if !state.response_db.is_empty() {
        let config = SpectrumConfig {
            fft_size: state.fft_size,
            sample_rate: state.sample_rate,
            db_floor: state.db_floor,
            db_top: state.db_top,
            log_scale: state.log_scale,
            min_freq: 20.0,
        };
        let plot_rect = spectrum::render_spectrum(ui, &state.response_db, &config, HOVER_READOUT_H);
        let hovered_bin = hovered_response_bin(ui, state, plot_rect);
        if let Some(bin) = hovered_bin {
            draw_hovered_bin_highlight(ui, state, plot_rect, bin);
        }
        draw_response_overlay(ui, state, plot_rect);
        draw_hover_readout(ui, state, hovered_bin);
    } else if state.pending_response.is_some() {
        ui.label("Rendering frequency response...");
    } else {
        ui.label("Frequency response will appear after the first render.");
    }
}

fn legacy_normalized_to_cutoff_hz(value: f32) -> f32 {
    (MIN_CUTOFF_HZ * (1000.0f32).powf(value)).clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ)
}

fn normalized_to_cutoff_hz(value: f32) -> f32 {
    let range = MAX_CUTOFF_HZ / MIN_CUTOFF_HZ;
    (MIN_CUTOFF_HZ * range.powf(value.clamp(0.0, 1.0))).clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ)
}

fn cutoff_hz_to_normalized(hz: f32) -> f32 {
    let hz = hz.clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
    (hz / MIN_CUTOFF_HZ).ln() / (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).ln()
}

fn migrate_cutoff_to_hz(value: f32) -> f32 {
    if value <= 1.0 {
        legacy_normalized_to_cutoff_hz(value)
    } else {
        value.clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ)
    }
}

fn format_hz(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else {
        format!("{:.0} Hz", hz)
    }
}

fn param_hash(state: &FilterDesignState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state.cutoff.to_bits().hash(&mut hasher);
    state.resonance.to_bits().hash(&mut hasher);
    state.poles.hash(&mut hasher);
    state.sample_rate.to_bits().hash(&mut hasher);
    state.fft_size.hash(&mut hasher);
    hasher.finish()
}

fn analysis_params(state: &FilterDesignState) -> AnalysisParams {
    AnalysisParams {
        cutoff: state.cutoff,
        resonance: state.resonance,
        poles: state.poles,
        fft_size: state.fft_size,
        sample_rate: state.sample_rate,
        db_floor: state.db_floor,
    }
}

fn start_response_job(state: &mut FilterDesignState, hash: u64) {
    if state.pending_response.is_some() {
        return;
    }

    let params = analysis_params(state);
    let (sender, receiver) = mpsc::channel();
    state.pending_response = Some(receiver);
    state.needs_render = false;

    thread::spawn(move || {
        let result = compute_response(params, hash);
        let _ = sender.send(result);
    });
}

fn poll_pending_response(ui: &mut egui::Ui, state: &mut FilterDesignState) {
    let Some(receiver) = state.pending_response.as_ref() else {
        return;
    };

    match receiver.try_recv() {
        Ok(result) => {
            state.pending_response = None;
            if result.hash == param_hash(state) {
                state.raw_response_db = result.response_db;
                state.peak_freq_hz = result.peak_freq_hz;
                state.peak_db = result.peak_db;
                update_display_response(state);
            } else {
                state.needs_render = true;
            }
        }
        Err(mpsc::TryRecvError::Empty) => {
            ui.ctx().request_repaint();
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            state.pending_response = None;
            state.needs_render = true;
        }
    }
}

fn compute_response(params: AnalysisParams, hash: u64) -> AnalysisResult {
    if real_self_oscillation_analysis_params(params) {
        compute_sine_probe_response(params, hash)
    } else {
        compute_impulse_response(params, hash)
    }
}

fn compute_impulse_response(params: AnalysisParams, hash: u64) -> AnalysisResult {
    let fft_size = params.fft_size;
    let sr = params.sample_rate;

    let impulse = render_analysis_response(params, ANALYSIS_IMPULSE_GAIN);

    let mut complex: Vec<Complex32> = impulse
        .iter()
        .map(|&sample| Complex32::new(sample, 0.0))
        .collect();
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(fft_size);
    fft.process(&mut complex);

    let db: Vec<f32> = (0..fft_size / 2)
        .map(|bin| {
            let re = complex[bin].re;
            let im = complex[bin].im;
            let mag = (re * re + im * im).sqrt() / ANALYSIS_IMPULSE_GAIN;
            20.0 * (mag.max(1e-10)).log10()
        })
        .collect();

    let bin_hz = sr / fft_size as f32;
    if let Some((bin, &peak_db)) = db
        .iter()
        .enumerate()
        .skip(1)
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
    {
        AnalysisResult {
            hash,
            response_db: db,
            peak_freq_hz: bin as f32 * bin_hz,
            peak_db,
        }
    } else {
        AnalysisResult {
            hash,
            response_db: db,
            peak_freq_hz: 0.0,
            peak_db: 0.0,
        }
    }
}

fn compute_sine_probe_response(params: AnalysisParams, hash: u64) -> AnalysisResult {
    let fft_size = params.fft_size;
    let sr = params.sample_rate;
    let bin_hz = sr / fft_size as f32;
    let mut db = vec![params.db_floor; fft_size / 2];
    let mut peak_db = params.db_floor;
    let mut peak_freq_hz = 0.0;
    let measured_bins = db.len().saturating_sub(1);
    let worker_count = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(measured_bins.max(1));
    let chunk_bins = (measured_bins + worker_count - 1) / worker_count;

    thread::scope(|scope| {
        let mut handles = Vec::new();
        for worker in 0..worker_count {
            let start_bin = 1 + worker * chunk_bins;
            let end_bin = (start_bin + chunk_bins).min(db.len());
            if start_bin < end_bin {
                handles.push(
                    scope.spawn(move || {
                        compute_sine_probe_range(params, start_bin, end_bin, bin_hz)
                    }),
                );
            }
        }

        for handle in handles {
            let Ok(chunk) = handle.join() else {
                continue;
            };
            for (index, response_db) in chunk.response_db.into_iter().enumerate() {
                let bin = chunk.start_bin + index;
                db[bin] = response_db;
            }
            if chunk.peak_db > peak_db {
                peak_db = chunk.peak_db;
                peak_freq_hz = chunk.peak_freq_hz;
            }
        }
    });

    AnalysisResult {
        hash,
        response_db: db,
        peak_freq_hz,
        peak_db,
    }
}

struct SineProbeChunk {
    start_bin: usize,
    response_db: Vec<f32>,
    peak_freq_hz: f32,
    peak_db: f32,
}

fn compute_sine_probe_range(
    params: AnalysisParams,
    start_bin: usize,
    end_bin: usize,
    bin_hz: f32,
) -> SineProbeChunk {
    let mut response_db = vec![params.db_floor; end_bin - start_bin];
    let mut peak_db = params.db_floor;
    let mut peak_freq_hz = 0.0;
    let mut bin = start_bin;

    while bin < end_bin {
        let mut freqs = [0.0f32; LANES];
        let mut active = [false; LANES];
        for lane in 0..LANES {
            let target_bin = bin + lane;
            if target_bin < end_bin {
                freqs[lane] = target_bin as f32 * bin_hz;
                active[lane] = true;
            }
        }

        let responses = measure_sine_probe_responses(params, freqs);
        for lane in 0..LANES {
            if active[lane] {
                let target_bin = bin + lane;
                let db = responses[lane];
                response_db[target_bin - start_bin] = db;
                if db > peak_db {
                    peak_db = db;
                    peak_freq_hz = freqs[lane];
                }
            }
        }
        bin += LANES;
    }

    SineProbeChunk {
        start_bin,
        response_db,
        peak_freq_hz,
        peak_db,
    }
}

fn measure_sine_probe_responses(params: AnalysisParams, freq_hz: [f32; LANES]) -> [f32; LANES] {
    let sr = params.sample_rate;
    let phase_step = freq_hz.map(|freq| std::f32::consts::TAU * freq / sr);
    let mut phase = [0.0f32; LANES];
    let mut filter = LadderFilter::default();
    filter.set_cutoff(params.cutoff);
    filter.set_resonance(analysis_resonance(params));
    filter.set_poles(params.poles);
    filter.reset();

    for _ in 0..SINE_PROBE_SETTLE_FRAMES {
        let mut input = [0.0f32; LANES];
        for lane in 0..LANES {
            input[lane] = phase[lane].sin() * ANALYSIS_SINE_GAIN;
            phase[lane] += phase_step[lane];
        }
        let _ = process_analysis_vector_output(&mut filter, f32x4::new(input), sr);
    }

    let mut sin_sum = [0.0f32; LANES];
    let mut cos_sum = [0.0f32; LANES];
    for _ in 0..SINE_PROBE_MEASURE_FRAMES {
        let mut input = [0.0f32; LANES];
        let mut sin = [0.0f32; LANES];
        let mut cos = [0.0f32; LANES];
        for lane in 0..LANES {
            sin[lane] = phase[lane].sin();
            cos[lane] = phase[lane].cos();
            input[lane] = sin[lane] * ANALYSIS_SINE_GAIN;
            phase[lane] += phase_step[lane];
        }
        let output = process_analysis_vector_output(&mut filter, f32x4::new(input), sr);
        for lane in 0..LANES {
            sin_sum[lane] += output[lane] * sin[lane];
            cos_sum[lane] += output[lane] * cos[lane];
        }
    }

    let mut db = [0.0f32; LANES];
    for lane in 0..LANES {
        let output_amp = 2.0
            * (sin_sum[lane] * sin_sum[lane] + cos_sum[lane] * cos_sum[lane]).sqrt()
            / SINE_PROBE_MEASURE_FRAMES as f32;
        db[lane] = 20.0 * (output_amp / ANALYSIS_SINE_GAIN).max(1e-10).log10();
    }
    db
}

fn render_analysis_response(params: AnalysisParams, impulse_gain: f32) -> Vec<f32> {
    let fft_size = params.fft_size;
    let sr = params.sample_rate;
    let mut filter = LadderFilter::default();
    filter.set_cutoff(params.cutoff);
    filter.set_resonance(analysis_resonance(params));
    filter.set_poles(params.poles);
    filter.reset();

    let mut impulse = vec![0.0f32; fft_size];
    for sample_index in 0..fft_size {
        let input = if sample_index == 0 {
            f32x4::splat(impulse_gain)
        } else {
            f32x4::splat(0.0)
        };
        impulse[sample_index] = process_analysis_vector_output(&mut filter, input, sr)[0];
    }
    impulse
}

fn process_analysis_vector_output(
    filter: &mut LadderFilter,
    input: f32x4,
    sample_rate: f32,
) -> [f32; LANES] {
    filter
        .process(
            input,
            f32x4::splat(60.0),
            f32x4::splat(0.0),
            f32x4::splat(1.0),
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            sample_rate,
        )
        .to_array()
}

fn analysis_resonance(params: AnalysisParams) -> f32 {
    params.resonance
}

fn real_self_oscillation_analysis(state: &FilterDesignState) -> bool {
    state.poles == 4 && state.resonance > SELF_OSC_RESONANCE_START
}

fn real_self_oscillation_analysis_params(params: AnalysisParams) -> bool {
    params.poles == 4 && params.resonance > SELF_OSC_RESONANCE_START
}

fn update_display_response(state: &mut FilterDesignState) {
    if state.raw_response_db.is_empty() {
        state.response_db.clear();
        state.smoothed_response_db.clear();
        state.peak_freq_hz = 0.0;
        state.peak_db = 0.0;
        return;
    }

    state.response_db = state.raw_response_db.clone();
    if state.smooth_response {
        state.smoothed_response_db = smooth_response(
            &state.raw_response_db,
            state.sample_rate,
            state.fft_size,
            state.db_floor,
        );
    } else {
        state.smoothed_response_db.clear();
    }
    update_display_peak(state);
}

fn smooth_response(raw: &[f32], sample_rate: f32, fft_size: usize, db_floor: f32) -> Vec<f32> {
    if raw.len() < 3 {
        return raw.to_vec();
    }

    let bin_hz = sample_rate / fft_size as f32;
    let octave_ratio = 2.0f32.powf(0.5 / SMOOTHING_FRACTIONAL_OCTAVE);
    let mut smoothed = raw.to_vec();

    for bin in 1..raw.len() {
        let freq = bin as f32 * bin_hz;
        let lower_bin = ((freq / octave_ratio) / bin_hz).floor().max(1.0) as usize;
        let upper_bin = ((freq * octave_ratio) / bin_hz)
            .ceil()
            .min((raw.len() - 1) as f32) as usize;

        let mut sum = 0.0;
        let mut count = 0usize;
        for &db in &raw[lower_bin..=upper_bin] {
            if db.is_finite() {
                sum += db;
                count += 1;
            }
        }
        smoothed[bin] = if count > 0 {
            sum / count as f32
        } else {
            db_floor
        };
    }

    smoothed
}

fn update_display_peak(state: &mut FilterDesignState) {
    let bin_hz = state.sample_rate / state.fft_size as f32;
    if let Some((bin, &peak_db)) = state
        .response_db
        .iter()
        .enumerate()
        .skip(1)
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
    {
        state.peak_freq_hz = bin as f32 * bin_hz;
        state.peak_db = peak_db;
    } else {
        state.peak_freq_hz = 0.0;
        state.peak_db = 0.0;
    }
}

fn draw_response_overlay(ui: &egui::Ui, state: &FilterDesignState, plot_rect: egui::Rect) {
    let response = overlay_response_db(state);
    if response.len() < 2 {
        return;
    }

    let painter = ui.painter_at(plot_rect);
    let max_freq = state.sample_rate * 0.5;
    let bin_hz = state.sample_rate / state.fft_size as f32;
    let db_range = state.db_top - state.db_floor;

    let mut points = Vec::new();
    for (bin, &db) in response.iter().enumerate().skip(1) {
        let hz = bin as f32 * bin_hz;
        if hz < 20.0 || hz > max_freq {
            continue;
        }
        let x = spectrum::freq_to_x(
            hz,
            state.log_scale,
            20.0,
            max_freq,
            plot_rect.left(),
            plot_rect.right(),
        );
        let normalized =
            ((db.clamp(state.db_floor, state.db_top) - state.db_floor) / db_range).clamp(0.0, 1.0);
        let y = plot_rect.bottom() - plot_rect.height() * normalized;
        points.push(egui::pos2(x, y));
    }

    if points.len() > 1 {
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 214, 102)),
        ));
    }

    draw_frequency_marker(
        &painter,
        plot_rect,
        state.cutoff,
        state,
        egui::Color32::from_rgb(255, 120, 80),
    );
    if state.peak_freq_hz >= 20.0 {
        draw_frequency_marker(
            &painter,
            plot_rect,
            state.peak_freq_hz,
            state,
            egui::Color32::from_rgb(130, 220, 255),
        );
    }
}

fn overlay_response_db(state: &FilterDesignState) -> &[f32] {
    if state.smooth_response && !state.smoothed_response_db.is_empty() {
        &state.smoothed_response_db
    } else {
        &state.response_db
    }
}

fn hovered_response_bin(
    ui: &egui::Ui,
    state: &FilterDesignState,
    plot_rect: egui::Rect,
) -> Option<usize> {
    let pos = ui.ctx().pointer_hover_pos()?;
    if !plot_rect.contains(pos) || state.response_db.len() < 2 {
        return None;
    }

    let max_freq = state.sample_rate * 0.5;
    let bin_hz = state.sample_rate / state.fft_size as f32;
    let x_fraction = ((pos.x - plot_rect.left()) / plot_rect.width()).clamp(0.0, 1.0);
    let freq = if state.log_scale {
        20.0 * (max_freq / 20.0).powf(x_fraction)
    } else {
        max_freq * x_fraction
    };

    let bin = (freq / bin_hz).floor() as usize;
    Some(bin.clamp(1, state.response_db.len() - 1))
}

fn draw_hovered_bin_highlight(
    ui: &egui::Ui,
    state: &FilterDesignState,
    plot_rect: egui::Rect,
    bin: usize,
) {
    if bin >= state.response_db.len() {
        return;
    }

    let max_freq = state.sample_rate * 0.5;
    let bin_hz = state.sample_rate / state.fft_size as f32;
    let db_range = state.db_top - state.db_floor;
    let db = state.response_db[bin].clamp(state.db_floor, state.db_top);
    let height_fraction = ((db - state.db_floor) / db_range).clamp(0.0, 1.0);
    let bar_height = (plot_rect.height() * height_fraction).max(1.0);

    let (x_from, x_to) = if state.log_scale {
        let freq = bin as f32 * bin_hz;
        let next_freq = (bin + 1) as f32 * bin_hz;
        (
            spectrum::freq_to_x(
                freq,
                true,
                20.0,
                max_freq,
                plot_rect.left(),
                plot_rect.right(),
            ),
            spectrum::freq_to_x(
                next_freq,
                true,
                20.0,
                max_freq,
                plot_rect.left(),
                plot_rect.right(),
            ),
        )
    } else {
        let bar_width = plot_rect.width() / (state.fft_size / 2) as f32;
        let x = plot_rect.left() + bin as f32 * bar_width;
        (x, x + bar_width)
    };

    let rect = egui::Rect::from_min_max(
        egui::pos2(x_from, plot_rect.bottom() - bar_height),
        egui::pos2(x_to.max(x_from + 1.0), plot_rect.bottom()),
    );
    let painter = ui.painter_at(plot_rect);
    painter.rect_filled(
        rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(180, 235, 255, 150),
    );
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(230, 250, 255)),
        egui::StrokeKind::Inside,
    );
}

fn draw_hover_readout(ui: &mut egui::Ui, state: &FilterDesignState, hovered_bin: Option<usize>) {
    ui.horizontal(|ui| {
        if let Some(bin) = hovered_bin.filter(|&bin| bin < state.response_db.len()) {
            let freq_hz = bin as f32 * state.sample_rate / state.fft_size as f32;
            let raw_db = state.response_db[bin];
            if state.smooth_response && bin < state.smoothed_response_db.len() {
                ui.label(format!(
                    "Bin: {bin}   Freq: {}   Level: {raw_db:+.2} dB   Smooth: {:+.2} dB",
                    format_hz(freq_hz),
                    state.smoothed_response_db[bin]
                ));
            } else {
                ui.label(format!(
                    "Bin: {bin}   Freq: {}   Level: {raw_db:+.2} dB",
                    format_hz(freq_hz)
                ));
            }
        } else {
            ui.label("Bin: -   Freq: -   Level: -");
        }
    });
}

fn draw_frequency_marker(
    painter: &egui::Painter,
    plot_rect: egui::Rect,
    hz: f32,
    state: &FilterDesignState,
    color: egui::Color32,
) {
    let x = spectrum::freq_to_x(
        hz,
        state.log_scale,
        20.0,
        state.sample_rate * 0.5,
        plot_rect.left(),
        plot_rect.right(),
    );
    if x < plot_rect.left() || x > plot_rect.right() {
        return;
    }

    painter.line_segment(
        [
            egui::pos2(x, plot_rect.top()),
            egui::pos2(x, plot_rect.bottom()),
        ],
        egui::Stroke::new(1.0, color),
    );
}
