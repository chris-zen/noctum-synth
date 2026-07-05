use eframe::egui;
use eframe::egui::PointerButton;
use eframe::egui::epaint::PathShape;
use rustfft::{Fft, FftPlanner, num_complex::Complex32};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::Arc;

use super::spectrum::{self, SpectrumConfig};
use crate::engine::{AudioBlock, MAX_AUDIO_BUF};

pub struct RealTimeState {
    pub osc: OscilloscopeState,
    pub fft: FftState,
}

impl Default for RealTimeState {
    fn default() -> Self {
        Self {
            osc: OscilloscopeState::default(),
            fft: FftState::default(),
        }
    }
}

pub fn show(ui: &mut egui::Ui, audio_blocks: VecDeque<AudioBlock>, state: &mut RealTimeState) {
    for block in audio_blocks {
        let block_len = (block.len as usize).min(MAX_AUDIO_BUF);
        let osc_copy_len = block_len.min(MAX_AUDIO_BUF);
        state.osc.buffer_l[..osc_copy_len].copy_from_slice(&block.left[..osc_copy_len]);
        state.osc.buffer_r[..osc_copy_len].copy_from_slice(&block.right[..osc_copy_len]);
        state.osc.buf_len = osc_copy_len;

        let fft_size = state.fft.fft_size;
        let copy_len = block_len.min(fft_size);
        let shift = fft_size - copy_len;
        state.fft.buffer.copy_within(copy_len..fft_size, 0);
        state.fft.buffer[shift..fft_size].copy_from_slice(&block.left[..copy_len]);
        state.fft.frame_count += 1;
    }

    let available = ui.available_size();
    let gap = 4.0;
    let osc_h = (available.y * 0.4).max(120.0);
    let fft_h = (available.y - osc_h - gap).max(120.0);

    ui.allocate_ui(egui::vec2(available.x, osc_h), |ui| {
        ui.strong("Oscilloscope");
        draw_oscilloscope(ui, &mut state.osc);
    });
    ui.add_space(gap);
    ui.allocate_ui(egui::vec2(available.x, fft_h), |ui| {
        ui.strong("Spectrum Analyzer");
        draw_fft(ui, &mut state.fft);
    });
}

pub struct OscilloscopeState {
    buffer_l: [f32; MAX_AUDIO_BUF],
    buffer_r: [f32; MAX_AUDIO_BUF],
    buf_len: usize,
    timebase_ms: f32,
    trigger_level: f32,
    y_range: f32,
    display_mode: OscilloscopeDisplayMode,
    frozen: bool,
    frozen_buffer_l: [f32; MAX_AUDIO_BUF],
    frozen_buffer_r: [f32; MAX_AUDIO_BUF],
    frozen_len: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OscilloscopeDisplayMode {
    Left,
    Right,
    Stereo,
}

impl Default for OscilloscopeState {
    fn default() -> Self {
        Self {
            buffer_l: [0.0; MAX_AUDIO_BUF],
            buffer_r: [0.0; MAX_AUDIO_BUF],
            buf_len: 0,
            timebase_ms: 5.0,
            trigger_level: 0.0,
            y_range: 1.0,
            display_mode: OscilloscopeDisplayMode::Left,
            frozen: false,
            frozen_buffer_l: [0.0; MAX_AUDIO_BUF],
            frozen_buffer_r: [0.0; MAX_AUDIO_BUF],
            frozen_len: 0,
        }
    }
}

fn find_trigger(buf: &[f32], len: usize, level: f32) -> f32 {
    for index in 0..len.saturating_sub(1) {
        if buf[index] < level && buf[index + 1] >= level {
            let fraction = (level - buf[index]) / (buf[index + 1] - buf[index]);
            return index as f32 + fraction;
        }
    }
    0.0
}

fn draw_oscilloscope(ui: &mut egui::Ui, state: &mut OscilloscopeState) {
    let available = ui.available_size();
    let controls_h = 20.0;
    let y_label_w = 28.0;
    let x_label_h = 14.0;
    let plot_h = (available.y - controls_h).max(40.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available.x, plot_h),
        egui::Sense::click_and_drag(),
    );
    let plot_left = rect.left() + y_label_w;
    let plot_rect = egui::Rect::from_min_max(
        egui::pos2(plot_left, rect.top()),
        egui::pos2(rect.right(), rect.bottom() - x_label_h),
    );
    let painter = ui.painter_at(rect);

    painter.rect_filled(plot_rect, 0.0, egui::Color32::from_rgb(20, 20, 24));

    let y_scale = plot_rect.height() * 0.4;
    let center_y = plot_rect.center().y;
    let range = state.y_range;
    let display_yscale = y_scale / range;
    let font_id = egui::FontId::monospace(8.0);
    let label_color = egui::Color32::from_rgb(120, 120, 130);
    let grid_color = egui::Color32::from_rgb(50, 50, 58);

    // Y-axis labels + horizontal grid lines
    for row in 0..=8 {
        let grid_y = plot_rect.top() + plot_rect.height() * (row as f32 / 8.0);
        painter.line_segment(
            [
                egui::pos2(plot_rect.left(), grid_y),
                egui::pos2(plot_rect.right(), grid_y),
            ],
            egui::Stroke::new(1.0, grid_color),
        );
        let val = (1.0 - row as f32 * 0.25) * range;
        painter.text(
            egui::pos2(plot_rect.left() - 4.0, grid_y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.2}", val),
            font_id.clone(),
            label_color,
        );
    }
    painter.line_segment(
        [
            egui::pos2(plot_rect.left(), center_y),
            egui::pos2(plot_rect.right(), center_y),
        ],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 70, 80)),
    );

    // Trigger level — orange dotted line
    let trig_y = center_y - state.trigger_level * display_yscale;
    let trig_y = trig_y.clamp(plot_rect.top(), plot_rect.bottom());
    let dot_len = 4.0;
    let gap = 4.0;
    let mut dot_x = plot_rect.left();
    while dot_x < plot_rect.right() {
        let end = (dot_x + dot_len).min(plot_rect.right());
        painter.line_segment(
            [egui::pos2(dot_x, trig_y), egui::pos2(end, trig_y)],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 160, 40)),
        );
        dot_x += dot_len + gap;
    }

    if response.clicked_by(PointerButton::Secondary) {
        state.timebase_ms = 5.0;
    }

    // Freeze on middle-click or toggle
    if response.clicked_by(PointerButton::Middle) {
        state.frozen = !state.frozen;
        if state.frozen {
            state.frozen_buffer_l = state.buffer_l;
            state.frozen_buffer_r = state.buffer_r;
            state.frozen_len = state.buf_len;
        }
    }

    let buf_l = if state.frozen {
        &state.frozen_buffer_l[..]
    } else {
        &state.buffer_l[..]
    };
    let buf_r = if state.frozen {
        &state.frozen_buffer_r[..]
    } else {
        &state.buffer_r[..]
    };
    let len = if state.frozen {
        state.frozen_len
    } else {
        state.buf_len
    };

    if len > 1 {
        let trigger_buf = match state.display_mode {
            OscilloscopeDisplayMode::Left | OscilloscopeDisplayMode::Stereo => buf_l,
            OscilloscopeDisplayMode::Right => buf_r,
        };
        let trig_f32 = find_trigger(trigger_buf, len, state.trigger_level);
        let trig_idx = trig_f32 as usize;
        let samples_to_show = (state.timebase_ms / 1000.0 * 44100.0) as usize;
        let samples_to_show = samples_to_show.min(len.saturating_sub(trig_idx)).max(2);
        let start = trig_idx;
        let end = (start + samples_to_show).min(len);

        match state.display_mode {
            OscilloscopeDisplayMode::Left => draw_oscilloscope_trace(
                &painter,
                plot_rect,
                center_y,
                display_yscale,
                buf_l,
                start,
                end,
                trig_f32,
                samples_to_show,
                egui::Color32::GREEN,
            ),
            OscilloscopeDisplayMode::Right => draw_oscilloscope_trace(
                &painter,
                plot_rect,
                center_y,
                display_yscale,
                buf_r,
                start,
                end,
                trig_f32,
                samples_to_show,
                egui::Color32::from_rgb(80, 160, 255),
            ),
            OscilloscopeDisplayMode::Stereo => {
                draw_oscilloscope_trace(
                    &painter,
                    plot_rect,
                    center_y,
                    display_yscale,
                    buf_l,
                    start,
                    end,
                    trig_f32,
                    samples_to_show,
                    egui::Color32::GREEN,
                );
                draw_oscilloscope_trace(
                    &painter,
                    plot_rect,
                    center_y,
                    display_yscale,
                    buf_r,
                    start,
                    end,
                    trig_f32,
                    samples_to_show,
                    egui::Color32::from_rgb(80, 160, 255),
                );
            }
        }

        // X-axis time labels
        let visible_ms = state.timebase_ms;
        let tick_interval = nice_tick_interval(visible_ms, 5.0);
        let mut tick_ms = 0.0;
        while tick_ms <= visible_ms {
            let tick_x = plot_rect.left() + plot_rect.width() * (tick_ms / visible_ms);
            if tick_x <= plot_rect.right() {
                painter.line_segment(
                    [
                        egui::pos2(tick_x, plot_rect.bottom()),
                        egui::pos2(tick_x, plot_rect.bottom() + 4.0),
                    ],
                    egui::Stroke::new(1.0, grid_color),
                );
                let label = if tick_ms < 1.0 {
                    format!("{:.1}ms", tick_ms)
                } else {
                    format!("{:.0}ms", tick_ms)
                };
                painter.text(
                    egui::pos2(tick_x, plot_rect.bottom() + 4.0),
                    egui::Align2::CENTER_TOP,
                    label,
                    font_id.clone(),
                    label_color,
                );
            }
            tick_ms += tick_interval;
        }
    }

    ui.horizontal(|ui| {
        ui.label("Timebase:");
        ui.add(
            egui::Slider::new(&mut state.timebase_ms, 1.0..=50.0)
                .logarithmic(true)
                .text("ms"),
        );
        ui.separator();
        ui.label("Trigger:");
        ui.add(egui::Slider::new(&mut state.trigger_level, -1.0..=1.0).text(""));
        ui.separator();
        ui.label("Y Range:");
        ui.add(
            egui::Slider::new(&mut state.y_range, 0.001..=1.0)
                .logarithmic(true)
                .text(""),
        );
        ui.separator();
        ui.label("Trace:");
        for (mode, label) in [
            (OscilloscopeDisplayMode::Left, "L"),
            (OscilloscopeDisplayMode::Right, "R"),
            (OscilloscopeDisplayMode::Stereo, "L+R"),
        ] {
            if ui
                .selectable_label(state.display_mode == mode, label)
                .clicked()
            {
                state.display_mode = mode;
            }
        }
        ui.separator();
        if ui
            .button(if state.frozen { "Unfreeze" } else { "Freeze" })
            .clicked()
        {
            state.frozen = !state.frozen;
            if state.frozen {
                state.frozen_buffer_l = state.buffer_l;
                state.frozen_buffer_r = state.buffer_r;
                state.frozen_len = state.buf_len;
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_oscilloscope_trace(
    painter: &egui::Painter,
    plot_rect: egui::Rect,
    center_y: f32,
    display_yscale: f32,
    buffer: &[f32],
    start: usize,
    end: usize,
    trig_f32: f32,
    samples_to_show: usize,
    color: egui::Color32,
) {
    let pts: Vec<egui::Pos2> = (start..end)
        .map(|sample_index| {
            let point_x = plot_rect.left()
                + plot_rect.width() * (sample_index as f32 - trig_f32) / samples_to_show as f32;
            let point_y = center_y - buffer[sample_index] * display_yscale;
            egui::pos2(point_x, point_y.clamp(plot_rect.top(), plot_rect.bottom()))
        })
        .collect();

    if pts.len() >= 2 {
        painter.add(PathShape::line(pts, egui::Stroke::new(1.2, color)));
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

pub struct FftState {
    buffer: [f32; 4096],
    peak_hold: [f32; 2048],
    peak_decay: f32,
    frame_count: u32,
    pub fft_size: usize,
    complex_buf: Vec<Complex32>,
    fft: Option<Arc<dyn Fft<f32>>>,
    pub window_type: usize,
    pub db_floor: f32,
    pub db_top: f32,
    pub log_scale: bool,
}

impl Default for FftState {
    fn default() -> Self {
        let fft_size = 4096;
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        Self {
            buffer: [0.0; 4096],
            peak_hold: [-120.0; 2048],
            peak_decay: 0.5,
            frame_count: 0,
            fft_size,
            complex_buf: vec![Complex32::new(0.0, 0.0); fft_size],
            fft: Some(fft),
            window_type: 0,
            db_floor: -96.0,
            db_top: 0.0,
            log_scale: true,
        }
    }
}

fn draw_fft(ui: &mut egui::Ui, state: &mut FftState) {
    let fft_size = state.fft_size;
    if state.frame_count % 4 == 0 {
        if state.complex_buf.len() != fft_size || state.fft.is_none() {
            state.complex_buf = vec![Complex32::new(0.0, 0.0); fft_size];
            let mut planner = FftPlanner::new();
            state.fft = Some(planner.plan_fft_forward(fft_size));
        }
        for index in 0..fft_size {
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
            state.complex_buf[index] = Complex32::new(state.buffer[index] * window, 0.0);
        }
        if let Some(ref fft) = state.fft {
            fft.process(&mut state.complex_buf);
        }
        let scale = 1.0 / fft_size as f32;
        for bin in 0..fft_size / 2 {
            let re = state.complex_buf[bin].re;
            let im = state.complex_buf[bin].im;
            let mag = (re * re + im * im).sqrt() * scale;
            let db = 20.0 * (mag.max(1e-10)).log10().max(-150.0);
            if db > state.peak_hold[bin] {
                state.peak_hold[bin] = db;
            } else {
                state.peak_hold[bin] += (db - state.peak_hold[bin]) * state.peak_decay * 0.05;
            }
        }
    }

    let config = SpectrumConfig {
        fft_size,
        sample_rate: 44100.0,
        db_floor: state.db_floor,
        db_top: state.db_top,
        log_scale: state.log_scale,
        min_freq: 20.0,
    };
    let _plot_rect = spectrum::render_spectrum(ui, &state.peak_hold[..fft_size / 2], &config, 24.0);

    // Controls
    ui.horizontal(|ui| {
        ui.label("Win:");
        for (index, name) in ["Hann", "Blackman", "FlatTop", "None"].iter().enumerate() {
            if ui
                .selectable_label(state.window_type == index, *name)
                .clicked()
            {
                state.window_type = index;
            }
        }
        ui.separator();
        ui.label("FFT:");
        for &size in &[1024, 2048, 4096] {
            if ui
                .selectable_label(state.fft_size == size, &size.to_string())
                .clicked()
            {
                state.fft_size = size;
                state.fft = None;
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
        if ui.selectable_label(state.log_scale, "Log").clicked() {
            state.log_scale = !state.log_scale;
        }
    });
}
