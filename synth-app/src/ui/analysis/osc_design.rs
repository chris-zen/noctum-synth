use std::f32::consts::TAU;

use eframe::egui;
use eframe::egui::PointerButton;
use eframe::egui::epaint::PathShape;
use rustfft::{FftPlanner, num_complex::Complex32};
use serde::{Deserialize, Serialize};

use synth_core::{
    dsp::{AnalogOscillator, SawMethod, Waveform},
    math::WideF32,
};

use crate::ui::analysis::spectrum::{self, SpectrumConfig};

pub struct OscillatorViewState {
    pub waveform: usize,
    pub saw_method: SawMethod,
    pub shape: f32,

    pub note: f32,
    pub sample_rate: f32,
    pub cycles: usize,
    pub live_mode: bool,

    pub fft_size: usize,
    pub window_type: usize,
    pub show_harmonics: bool,
    pub log_scale: bool,
    pub db_top: f32,   // 12.0, 6.0, or 0.0
    pub db_floor: f32, // -60.0, -96.0, or -144.0

    pub samples: Vec<f32>,
    pub fft_db: Vec<f32>,
    pub last_params_hash: u64,
    pub needs_render: bool,
    pub rendered_waveform: usize,
    pub rendered_method: SawMethod,

    pub zoom_ms: f32,
    pub offset_ms: f32,
    pub show_dots: bool,

    // Drag-to-zoom
    pub wave_drag_start: Option<f32>,
    pub wave_drag_end: Option<f32>,
    pub wave_dragging: bool,

    pub live_frame: u32,
}

impl Default for OscillatorViewState {
    fn default() -> Self {
        Self {
            waveform: 0,
            saw_method: SawMethod::Blep,
            shape: 0.0,
            note: 60.0,
            sample_rate: 44100.0,
            cycles: 1,
            live_mode: true,
            fft_size: 4096,
            window_type: 0,
            show_harmonics: true,
            log_scale: true,
            db_top: 12.0,
            db_floor: -96.0,
            samples: Vec::new(),
            fft_db: Vec::new(),
            last_params_hash: 0,
            needs_render: true,
            rendered_waveform: 0,
            rendered_method: SawMethod::PolyBlep,
            zoom_ms: 7.0,
            offset_ms: 0.0,
            show_dots: true,
            wave_drag_start: None,
            wave_drag_end: None,
            wave_dragging: false,
            live_frame: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SawMethodConfig {
    PolyBlep,
    Blep,
}

impl Default for SawMethodConfig {
    fn default() -> Self {
        Self::Blep
    }
}

impl From<SawMethod> for SawMethodConfig {
    fn from(method: SawMethod) -> Self {
        match method {
            SawMethod::PolyBlep => Self::PolyBlep,
            SawMethod::Blep => Self::Blep,
        }
    }
}

impl From<SawMethodConfig> for SawMethod {
    fn from(method: SawMethodConfig) -> Self {
        match method {
            SawMethodConfig::PolyBlep => Self::PolyBlep,
            SawMethodConfig::Blep => Self::Blep,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct OscDesignViewConfig {
    pub waveform: usize,
    pub saw_method: SawMethodConfig,
    pub shape: f32,
    pub note: f32,
    pub sample_rate: f32,
    pub cycles: usize,
    pub live_mode: bool,
    pub fft_size: usize,
    pub window_type: usize,
    pub show_harmonics: bool,
    pub log_scale: bool,
    pub db_top: f32,
    pub db_floor: f32,
    pub zoom_ms: f32,
    pub offset_ms: f32,
    pub show_dots: bool,
}

impl Default for OscDesignViewConfig {
    fn default() -> Self {
        Self::from_state(&OscillatorViewState::default())
    }
}

impl OscDesignViewConfig {
    pub fn from_state(state: &OscillatorViewState) -> Self {
        Self {
            waveform: state.waveform,
            saw_method: state.saw_method.into(),
            shape: state.shape,
            note: state.note,
            sample_rate: state.sample_rate,
            cycles: state.cycles,
            live_mode: state.live_mode,
            fft_size: state.fft_size,
            window_type: state.window_type,
            show_harmonics: state.show_harmonics,
            log_scale: state.log_scale,
            db_top: state.db_top,
            db_floor: state.db_floor,
            zoom_ms: state.zoom_ms,
            offset_ms: state.offset_ms,
            show_dots: state.show_dots,
        }
    }

    pub fn apply_to(&self, state: &mut OscillatorViewState) {
        state.waveform = self.waveform;
        state.saw_method = self.saw_method.into();
        state.shape = self.shape;
        state.note = self.note;
        state.sample_rate = self.sample_rate;
        state.cycles = self.cycles;
        state.live_mode = self.live_mode;
        state.fft_size = self.fft_size;
        state.window_type = self.window_type;
        state.show_harmonics = self.show_harmonics;
        state.log_scale = self.log_scale;
        state.db_top = self.db_top;
        state.db_floor = self.db_floor;
        state.zoom_ms = self.zoom_ms;
        state.offset_ms = self.offset_ms;
        state.show_dots = self.show_dots;
        state.needs_render = true;
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut OscillatorViewState) {
    // --- Synth params ---
    ui.horizontal(|ui| {
        ui.label("Wave:");
        for (index, name) in ["Saw", "S+T", "Tri", "Pulse"].iter().enumerate() {
            if ui
                .selectable_label(state.waveform == index, *name)
                .clicked()
            {
                state.waveform = index;
            }
        }
        ui.separator();
        if state.waveform == 0 || state.waveform == 2 || state.waveform == 3 {
            ui.label("Method:");
            for (method, name) in [(SawMethod::PolyBlep, "PolyBLEP"), (SawMethod::Blep, "BLEP")] {
                if ui
                    .selectable_label(state.saw_method == method, name)
                    .clicked()
                {
                    state.saw_method = method;
                }
            }
            ui.separator();
        }
        ui.label(format!("Shape: {:.2}", state.shape));
        ui.add(egui::Slider::new(&mut state.shape, 0.0..=1.0).text(""));
    });

    ui.add_space(4.0);

    // --- Analysis params ---
    ui.horizontal(|ui| {
        ui.label("Note:");
        ui.add(egui::Slider::new(&mut state.note, 0.0..=127.0).text(""));
        ui.label(format!(
            "{:.0} ({})",
            state.note,
            note_name(state.note as u8)
        ));
        ui.separator();
        ui.label("SR:");
        for &(label, sr) in &[
            ("44.1k", 44100.0),
            ("48k", 48000.0),
            ("96k", 96000.0),
            ("192k", 192000.0),
        ] {
            if ui
                .selectable_label(state.sample_rate == sr, label)
                .clicked()
            {
                state.sample_rate = sr;
            }
        }
        ui.separator();
        ui.label("Cycles:");
        ui.add(egui::Slider::new(&mut state.cycles, 1..=64).text(""));
        ui.separator();
        if ui.button("Render").clicked() {
            state.needs_render = true;
        }
        ui.checkbox(&mut state.live_mode, "Live");
        if !state.samples.is_empty() && ui.button("Save WAV").clicked() {
            save_wav(state);
        }
    });

    // --- Render scheduling ---
    let current_hash = param_hash(state);
    let params_changed = current_hash != state.last_params_hash;
    let waveform_changed =
        state.waveform != state.rendered_waveform || state.saw_method != state.rendered_method;

    let should_render = if state.samples.is_empty() || waveform_changed || state.needs_render {
        // First render, waveform/method switch, or an explicit request: render now.
        true
    } else if state.live_mode && params_changed {
        // Throttle live re-renders while parameters are being changed.
        state.live_frame = state.live_frame.wrapping_add(1);
        state.live_frame % 6 == 0
    } else {
        false
    };

    if should_render {
        render_oscillator(state);
        state.needs_render = false;
        state.last_params_hash = current_hash;
    }

    ui.add_space(8.0);

    let available = ui.available_size();
    let gap = 12.0;
    let section_h = ((available.y - gap) / 2.0).max(80.0);

    // --- Waveform ---
    ui.allocate_ui(egui::vec2(available.x, section_h), |ui| {
        ui.strong("Waveform");
        ui.add_space(6.0);
        let total_ms = state.samples.len() as f32 / state.sample_rate * 1000.0;
        let min_zoom = 0.001_f32;
        let max_zoom = total_ms.max(min_zoom * 2.0);
        ui.horizontal(|ui| {
            ui.label("Zoom:");
            ui.add(
                egui::Slider::new(&mut state.zoom_ms, min_zoom..=max_zoom)
                    .logarithmic(true)
                    .text("ms"),
            );
            let max_off = (total_ms - state.zoom_ms).max(0.0);
            if max_off > 0.0 {
                ui.separator();
                ui.label("Offset:");
                ui.add(egui::Slider::new(&mut state.offset_ms, 0.0..=max_off).text("ms"));
            }
            ui.separator();
            if ui.button("Reset View").clicked() {
                state.zoom_ms = total_ms;
                state.offset_ms = 0.0;
            }
            ui.separator();
            ui.checkbox(&mut state.show_dots, "Dots");
            ui.separator();
            ui.label("Right-click to reset, drag to zoom");
        });
        ui.add_space(6.0);
        draw_waveform(ui, state);
    });

    ui.add_space(gap);

    // --- Harmonic Analysis ---
    ui.allocate_ui(egui::vec2(available.x, section_h), |ui| {
        ui.strong("Harmonic Analysis");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Window:");
            for (index, name) in ["Hann", "Blackman", "FlatTop", "None"].iter().enumerate() {
                if ui
                    .selectable_label(state.window_type == index, *name)
                    .clicked()
                {
                    state.window_type = index;
                    state.needs_render = true;
                }
            }
            ui.separator();
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
            for &db in &[12.0, 6.0, 0.0] {
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
            if ui
                .selectable_label(state.show_harmonics, "Harmonics")
                .clicked()
            {
                state.show_harmonics = !state.show_harmonics;
            }
            ui.separator();
            if ui.selectable_label(state.log_scale, "Log").clicked() {
                state.log_scale = !state.log_scale;
            }
        });
        ui.add_space(6.0);
        draw_harmonics(ui, state);
    });
}

fn param_hash(state: &OscillatorViewState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state.waveform.hash(&mut hasher);
    state.saw_method.hash(&mut hasher);
    state.shape.to_bits().hash(&mut hasher);
    state.note.to_bits().hash(&mut hasher);
    state.sample_rate.to_bits().hash(&mut hasher);
    state.cycles.hash(&mut hasher);
    state.fft_size.hash(&mut hasher);
    state.window_type.hash(&mut hasher);
    hasher.finish()
}

fn render_oscillator(state: &mut OscillatorViewState) {
    state.rendered_waveform = state.waveform;
    state.rendered_method = state.saw_method;
    let freq = midi_to_hz(state.note);
    let sr = state.sample_rate;
    let samples_per_cycle = (sr / freq).round() as usize;
    let total_samples = samples_per_cycle * state.cycles;
    let length_changed = total_samples != state.samples.len();

    let mut osc = AnalogOscillator::new(sr);
    osc.set_waveform(int_to_waveform(state.waveform));
    osc.set_saw_method(state.saw_method);
    osc.set_shape(state.shape);
    osc.start_phase_lane(0);
    osc.set_frequency(WideF32::splat(freq));

    state.samples.clear();
    state.samples.reserve(total_samples);
    let mut ctx = synth_core::create_render_context!();
    for _ in 0..total_samples {
        state.samples.push(osc.next(&mut ctx).output.to_array()[0]);
    }

    // Generate extra samples for the FFT — as many as needed to fill fft_size
    let fft_total = total_samples.max(state.fft_size);
    let mut fft_samples = Vec::with_capacity(fft_total);
    fft_samples.extend_from_slice(&state.samples);
    while fft_samples.len() < fft_total {
        fft_samples.push(osc.next(&mut ctx).output.to_array()[0]);
    }

    let total_ms = state.samples.len() as f32 / sr * 1000.0;
    if length_changed || state.zoom_ms > total_ms || state.zoom_ms <= 0.0 {
        state.zoom_ms = total_ms;
        state.offset_ms = 0.0;
    }
    compute_fft(state, &fft_samples);
}

fn compute_fft(state: &mut OscillatorViewState, fft_samples: &[f32]) {
    let fft_size = state.fft_size;
    let mut slice = vec![0.0f32; fft_size];
    let copy_len = fft_samples.len().min(fft_size);
    slice[..copy_len].copy_from_slice(&fft_samples[..copy_len]);
    run_fft(&slice, state, fft_size);
}

fn run_fft(data: &[f32], state: &mut OscillatorViewState, fft_size: usize) {
    let windowed: Vec<f32> = data
        .iter()
        .enumerate()
        .map(|(index, &sample)| {
            let window = match state.window_type {
                0 => 0.5 * (1.0 - (TAU * index as f32 / (fft_size - 1) as f32).cos()),
                1 => {
                    let a0 = 0.42;
                    let a1 = 0.5;
                    let a2 = 0.08;
                    a0 - a1 * (TAU * index as f32 / (fft_size - 1) as f32).cos()
                        + a2 * (2.0 * TAU * index as f32 / (fft_size - 1) as f32).cos()
                }
                2 => {
                    let a0 = 0.21557895;
                    let a1 = 0.41663158;
                    let a2 = 0.277263158;
                    let a3 = 0.083578947;
                    let a4 = 0.006947368;
                    a0 - a1 * (TAU * index as f32 / (fft_size - 1) as f32).cos()
                        + a2 * (2.0 * TAU * index as f32 / (fft_size - 1) as f32).cos()
                        - a3 * (3.0 * TAU * index as f32 / (fft_size - 1) as f32).cos()
                        + a4 * (4.0 * TAU * index as f32 / (fft_size - 1) as f32).cos()
                }
                _ => 1.0,
            };
            sample * window
        })
        .collect();

    let mut complex: Vec<Complex32> = windowed
        .iter()
        .map(|&sample| Complex32::new(sample, 0.0))
        .collect();
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(fft_size);
    fft.process(&mut complex);

    let scale = 1.0 / fft_size as f32;
    let mut db_values: Vec<f32> = (0..fft_size / 2)
        .map(|bin| {
            let re = complex[bin].re;
            let im = complex[bin].im;
            let mag = (re * re + im * im).sqrt() * scale;
            20.0 * (mag.max(1e-10)).log10()
        })
        .collect();

    let freq = midi_to_hz(state.note);
    let bin_res = state.sample_rate / fft_size as f32;
    let fund_bin = (freq / bin_res).round() as usize;

    // Search ±2 bins around the expected fundamental for the actual peak.
    // With windowing and non-integer bin alignment, the energy spreads.
    let search_start = fund_bin.saturating_sub(2);
    let search_end = (fund_bin + 3).min(fft_size / 2);
    let mut fund_db = -200.0f32;
    for bin in search_start..search_end {
        if db_values[bin] > fund_db {
            fund_db = db_values[bin];
        }
    }
    if fund_db > -80.0 {
        for value in &mut db_values {
            *value -= fund_db;
        }
    }
    state.fft_db = db_values;
}

const WF_BOTTOM_H: f32 = 14.0;

fn draw_waveform(ui: &mut egui::Ui, state: &mut OscillatorViewState) {
    let available = ui.available_size();
    let y_label_w = 28.0;
    let top_pad = 8.0;
    let plot_h = (available.y - WF_BOTTOM_H).max(40.0);

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available.x, plot_h + WF_BOTTOM_H),
        egui::Sense::click_and_drag(),
    );
    let plot_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + y_label_w, rect.top() + top_pad),
        egui::pos2(rect.right(), rect.bottom() - WF_BOTTOM_H),
    );
    let painter = ui.painter_at(rect);

    painter.rect_filled(plot_rect, 0.0, egui::Color32::from_rgb(20, 20, 24));

    let grid_color = egui::Color32::from_rgb(50, 50, 58);
    let label_color = egui::Color32::from_rgb(120, 120, 130);
    let font_id = egui::FontId::monospace(8.0);
    let center_y = plot_rect.center().y;
    let y_scale = plot_rect.height() * 0.5;
    for row in 0..=8 {
        let grid_y = plot_rect.top() + plot_rect.height() * (row as f32 / 8.0);
        painter.line_segment(
            [
                egui::pos2(plot_rect.left(), grid_y),
                egui::pos2(plot_rect.right(), grid_y),
            ],
            egui::Stroke::new(1.0_f32, grid_color),
        );
        let val = (center_y - grid_y) / y_scale;
        painter.text(
            egui::pos2(plot_rect.left() - 4.0, grid_y),
            egui::Align2::RIGHT_CENTER,
            format!("{val:.2}"),
            font_id.clone(),
            label_color,
        );
    }
    painter.line_segment(
        [
            egui::pos2(plot_rect.left(), center_y),
            egui::pos2(plot_rect.right(), center_y),
        ],
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 70, 80)),
    );

    // Right-click reset
    if response.clicked_by(PointerButton::Secondary) {
        let total_ms = state.samples.len() as f32 / state.sample_rate * 1000.0;
        state.zoom_ms = total_ms;
        state.offset_ms = 0.0;
    }

    // Drag-to-zoom
    if response.drag_started_by(PointerButton::Primary) {
        if let Some(pos) = response.hover_pos() {
            if plot_rect.contains(pos) {
                let nx = ((pos.x - plot_rect.left()) / plot_rect.width()).clamp(0.0, 1.0);
                state.wave_drag_start = Some(nx);
                state.wave_drag_end = None;
                state.wave_dragging = true;
            }
        }
    }
    if state.wave_dragging && response.dragged_by(PointerButton::Primary) {
        if let Some(pos) = response.hover_pos() {
            let nx = ((pos.x - plot_rect.left()) / plot_rect.width()).clamp(0.0, 1.0);
            state.wave_drag_end = Some(nx);
        }
    }
    let still_dragging = response.dragged_by(PointerButton::Primary);
    if state.wave_dragging && !still_dragging {
        if let (Some(start), Some(end)) = (state.wave_drag_start, state.wave_drag_end) {
            let (x0, x1) = if start < end {
                (start, end)
            } else {
                (end, start)
            };
            // Convert normalised positions in the current view to absolute time
            let visible_ms = state.zoom_ms;
            let new_zoom = (x1 - x0) * visible_ms;
            let new_offset = state.offset_ms + x0 * visible_ms;
            let total_ms = state.samples.len() as f32 / state.sample_rate * 1000.0;
            if new_zoom > total_ms * 0.0001 && new_zoom < total_ms {
                state.zoom_ms = new_zoom;
                state.offset_ms = new_offset.clamp(0.0, (total_ms - new_zoom).max(0.0));
            }
        }
        state.wave_drag_start = None;
        state.wave_drag_end = None;
        state.wave_dragging = false;
    }

    // Draw waveform
    if !state.samples.is_empty() {
        let sr = state.sample_rate;
        let total_ms = state.samples.len() as f32 / sr * 1000.0;
        let visible_ms = state.zoom_ms.min(total_ms);
        let max_off = (total_ms - visible_ms).max(0.0);
        state.offset_ms = state.offset_ms.clamp(0.0, max_off);

        let s0 = (state.offset_ms / 1000.0 * sr) as usize;
        let n_vis = (visible_ms / 1000.0 * sr) as usize;
        let send = (s0 + n_vis).min(state.samples.len());

        let num_pts = send.saturating_sub(s0);
        let points: Vec<egui::Pos2> = (s0..send)
            .map(|sample_index| {
                let fraction = (sample_index - s0) as f32 / num_pts.max(1) as f32;
                let point_x = plot_rect.left() + plot_rect.width() * fraction;
                let point_y = center_y - state.samples[sample_index] * y_scale;
                egui::pos2(point_x, point_y.clamp(plot_rect.top(), plot_rect.bottom()))
            })
            .collect();

        if points.len() >= 2 {
            painter.add(PathShape::line(
                points,
                egui::Stroke::new(1.2_f32, egui::Color32::from_rgb(100, 220, 140)),
            ));
        }

        // Sample dots — recompute positions to avoid clone
        if state.show_dots && num_pts <= 2000 {
            for sample_index in s0..send {
                let fraction = (sample_index - s0) as f32 / num_pts.max(1) as f32;
                let point_x = plot_rect.left() + plot_rect.width() * fraction;
                let point_y = center_y - state.samples[sample_index] * y_scale;
                let point_y = point_y.clamp(plot_rect.top(), plot_rect.bottom());
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        egui::pos2(point_x, point_y),
                        egui::vec2(2.0, 2.0),
                    ),
                    0.0,
                    egui::Color32::from_rgba_premultiplied(100, 220, 140, 200),
                );
            }
        }
    } else {
        painter.text(
            plot_rect.center(),
            egui::Align2::CENTER_CENTER,
            "Click Render",
            egui::FontId::monospace(12.0),
            egui::Color32::from_rgb(120, 120, 130),
        );
    }

    // Selection rectangle
    if let (Some(start), Some(end)) = (state.wave_drag_start, state.wave_drag_end) {
        let x0 = plot_rect.left() + plot_rect.width() * start.min(end);
        let x1 = plot_rect.left() + plot_rect.width() * start.max(end);
        let sr = egui::Rect::from_min_max(
            egui::pos2(x0, plot_rect.top()),
            egui::pos2(x1, plot_rect.bottom()),
        );
        painter.rect_filled(
            sr,
            0.0,
            egui::Color32::from_rgba_premultiplied(100, 180, 255, 40),
        );
        painter.rect_stroke(
            sr,
            0.0,
            egui::Stroke::new(
                1.0_f32,
                egui::Color32::from_rgba_premultiplied(100, 180, 255, 120),
            ),
            egui::StrokeKind::Inside,
        );
    }

    // Frequency axis at bottom
    let total_ms = state.samples.len() as f32 / state.sample_rate * 1000.0;
    let visible_ms = state.zoom_ms.min(total_ms);
    let start_ms = state.offset_ms;
    let end_ms = start_ms + visible_ms;
    let label_y = plot_rect.bottom() + 2.0;
    // Tick every nice round number
    let tick_interval = nice_tick_interval(visible_ms, 5.0);
    let mut tick_ms = (start_ms / tick_interval).ceil() * tick_interval;
    while tick_ms <= end_ms {
        let tick_x = plot_rect.left() + plot_rect.width() * ((tick_ms - start_ms) / visible_ms);
        painter.line_segment(
            [
                egui::pos2(tick_x, plot_rect.bottom()),
                egui::pos2(tick_x, plot_rect.bottom() + 4.0),
            ],
            egui::Stroke::new(1.0_f32, grid_color),
        );
        painter.text(
            egui::pos2(tick_x, label_y),
            egui::Align2::CENTER_TOP,
            format!("{tick_ms:.0}ms"),
            egui::FontId::monospace(9.0),
            egui::Color32::from_rgb(120, 120, 130),
        );
        tick_ms += tick_interval;
    }
}

fn nice_tick_interval(range: f32, target_ticks: f32) -> f32 {
    let rough = range / target_ticks;
    let exp = 10.0f32.powf(rough.log10().floor());
    let mant = rough / exp;
    let nice = if mant < 1.5 {
        1.0
    } else if mant < 3.5 {
        2.0
    } else if mant < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * exp
}

fn draw_harmonics(ui: &mut egui::Ui, state: &mut OscillatorViewState) {
    if state.fft_db.is_empty() {
        ui.label("Click Render to compute FFT.");
        return;
    }

    let config = SpectrumConfig {
        fft_size: state.fft_size,
        sample_rate: state.sample_rate,
        db_floor: state.db_floor,
        db_top: state.db_top,
        log_scale: state.log_scale,
        min_freq: 20.0,
    };
    let plot_rect = spectrum::render_spectrum(ui, &state.fft_db, &config, 0.0);

    // Harmonics overlay
    if state.show_harmonics {
        let freq = midi_to_hz(state.note);
        let max_freq = state.sample_rate / 2.0;
        let max_h = (max_freq / freq) as usize;
        // Derive the bin layout from the data we actually have, not from
        // state.fft_size, which may have changed before the FFT was recomputed.
        let num_bins = state.fft_db.len();
        let bin_hz = max_freq / num_bins.max(1) as f32;
        let painter = ui.painter_at(plot_rect);
        for harmonic in 1..=max_h {
            let hz = freq * harmonic as f32;
            let marker_x = spectrum::freq_to_x(
                hz,
                state.log_scale,
                20.0,
                max_freq,
                plot_rect.left(),
                plot_rect.right(),
            );
            if marker_x > plot_rect.right() {
                continue;
            }
            let bin = (hz / bin_hz).round() as usize;
            if bin < num_bins {
                let db_val = state.fft_db[bin].min(state.db_top).max(state.db_floor);
                let db_range = state.db_top - state.db_floor;
                let marker_y = plot_rect.bottom()
                    - plot_rect.height() * ((db_val - state.db_floor) / db_range).clamp(0.0, 1.0);
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        egui::pos2(marker_x, marker_y),
                        egui::vec2(4.0, 4.0),
                    ),
                    0.0,
                    egui::Color32::from_rgb(255, 120, 60),
                );
            }
        }
    }
}

fn midi_to_hz(note: f32) -> f32 {
    440.0 * 2.0f32.powf((note - 69.0) / 12.0)
}

fn note_name(note: u8) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", names[(note % 12) as usize], (note as i32 / 12) - 1)
}

fn int_to_waveform(value: usize) -> Waveform {
    match value {
        0 => Waveform::Saw,
        1 => Waveform::SawTri,
        2 => Waveform::Triangle,
        3 => Waveform::Pulse,
        _ => Waveform::Saw,
    }
}

fn save_wav(state: &OscillatorViewState) {
    let path = format!(
        "osc_{}_{}hz_{}pt_{:.0}cyc.wav",
        match state.waveform {
            0 => "saw",
            1 => "sawtri",
            2 => "tri",
            3 => "pulse",
            _ => "mixed",
        },
        state.sample_rate as u32,
        match state.saw_method {
            SawMethod::PolyBlep => "poly",
            SawMethod::Blep => "blep",
        },
        state.cycles,
    );
    let sr = state.sample_rate as u32;
    let data: Vec<i16> = state
        .samples
        .iter()
        .map(|&sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect();

    let mut buf = Vec::with_capacity(44 + data.len() * 2);
    // RIFF header
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data.len() as u32 * 2).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    // fmt chunk
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sr.to_le_bytes());
    buf.extend_from_slice(&(sr * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    // data chunk
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(data.len() as u32 * 2).to_le_bytes());
    for sample in &data {
        buf.extend_from_slice(&sample.to_le_bytes());
    }

    std::fs::write(&path, &buf).ok();
    eprintln!(
        "WAV saved: {path} ({} samples, {:.1}ms)",
        data.len(),
        data.len() as f32 / sr as f32 * 1000.0
    );
}
