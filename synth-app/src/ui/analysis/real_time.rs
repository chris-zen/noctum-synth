use eframe::egui;
use eframe::egui::PointerButton;
use eframe::egui::epaint::PathShape;
use rustfft::{Fft, FftPlanner, num_complex::Complex32};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::Arc;

use crate::engine::{AudioBlock, MAX_AUDIO_BUF};
use crate::ui::analysis::spectrum::{self, SpectrumConfig};

const INPUT_LEFT_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 150, 45);
const INPUT_RIGHT_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 205, 80);
const OUTPUT_LEFT_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 205, 255);
const OUTPUT_RIGHT_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 125, 255);

pub struct RealTimeState {
    pub sample_rate: f32,
    pub osc: OscilloscopeState,
    pub fft: FftState,
}

impl Default for RealTimeState {
    fn default() -> Self {
        Self {
            sample_rate: 44100.0,
            osc: OscilloscopeState::default(),
            fft: FftState::default(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OscilloscopeDisplayModeConfig {
    Left,
    Right,
    Stereo,
}

impl Default for OscilloscopeDisplayModeConfig {
    fn default() -> Self {
        Self::Left
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpectrumChannel {
    Left,
    Right,
    Sum,
}

impl Default for SpectrumChannel {
    fn default() -> Self {
        Self::Left
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalSource {
    Input,
    Output,
    InputAndOutput,
}

impl Default for SignalSource {
    fn default() -> Self {
        Self::Output
    }
}

#[derive(Serialize, Deserialize)]
pub struct OscilloscopeViewConfig {
    pub timebase_ms: f32,
    pub trigger_level: f32,
    pub y_range: f32,
    pub display_mode: OscilloscopeDisplayModeConfig,
    #[serde(default)]
    pub source: SignalSource,
}

#[derive(Serialize, Deserialize)]
pub struct FftViewConfig {
    pub fft_size: usize,
    pub window_type: usize,
    pub db_floor: f32,
    pub db_top: f32,
    pub log_scale: bool,
    #[serde(default)]
    pub show_peak_hold: bool,
    #[serde(default)]
    pub channel: SpectrumChannel,
    #[serde(default)]
    pub source: SignalSource,
}

#[derive(Serialize, Deserialize)]
pub struct RealTimeViewConfig {
    pub oscilloscope: OscilloscopeViewConfig,
    pub fft: FftViewConfig,
}

impl Default for OscilloscopeViewConfig {
    fn default() -> Self {
        Self::from_state(&OscilloscopeState::default())
    }
}

impl OscilloscopeViewConfig {
    pub fn from_state(state: &OscilloscopeState) -> Self {
        Self {
            timebase_ms: state.timebase_ms,
            trigger_level: state.trigger_level,
            y_range: state.y_range,
            display_mode: display_mode_to_config(state.display_mode),
            source: state.source,
        }
    }

    pub fn apply_to(&self, state: &mut OscilloscopeState) {
        state.timebase_ms = self.timebase_ms;
        state.trigger_level = self.trigger_level;
        state.y_range = self.y_range;
        state.display_mode = display_mode_from_config(self.display_mode);
        state.source = self.source;
    }
}

impl Default for FftViewConfig {
    fn default() -> Self {
        Self::from_state(&FftState::default())
    }
}

impl FftViewConfig {
    pub fn from_state(state: &FftState) -> Self {
        Self {
            fft_size: state.fft_size,
            window_type: state.window_type,
            db_floor: state.db_floor,
            db_top: state.db_top,
            log_scale: state.log_scale,
            show_peak_hold: state.show_peak_hold,
            channel: state.channel,
            source: state.source,
        }
    }

    pub fn apply_to(&self, state: &mut FftState) {
        state.fft_size = self.fft_size;
        state.window_type = self.window_type;
        state.db_floor = self.db_floor;
        state.db_top = self.db_top;
        state.log_scale = self.log_scale;
        state.show_peak_hold = self.show_peak_hold;
        state.channel = self.channel;
        state.source = self.source;
        if state.complex_buf.len() != self.fft_size {
            state.complex_buf = vec![Complex32::new(0.0, 0.0); self.fft_size];
            state.fft = None;
        }
    }
}

impl Default for RealTimeViewConfig {
    fn default() -> Self {
        Self::from_state(&RealTimeState::default())
    }
}

impl RealTimeViewConfig {
    pub fn from_state(state: &RealTimeState) -> Self {
        Self {
            oscilloscope: OscilloscopeViewConfig::from_state(&state.osc),
            fft: FftViewConfig::from_state(&state.fft),
        }
    }

    pub fn apply_to(&self, state: &mut RealTimeState) {
        self.oscilloscope.apply_to(&mut state.osc);
        self.fft.apply_to(&mut state.fft);
    }
}

fn display_mode_to_config(mode: OscilloscopeDisplayMode) -> OscilloscopeDisplayModeConfig {
    match mode {
        OscilloscopeDisplayMode::Left => OscilloscopeDisplayModeConfig::Left,
        OscilloscopeDisplayMode::Right => OscilloscopeDisplayModeConfig::Right,
        OscilloscopeDisplayMode::Stereo => OscilloscopeDisplayModeConfig::Stereo,
    }
}

fn display_mode_from_config(mode: OscilloscopeDisplayModeConfig) -> OscilloscopeDisplayMode {
    match mode {
        OscilloscopeDisplayModeConfig::Left => OscilloscopeDisplayMode::Left,
        OscilloscopeDisplayModeConfig::Right => OscilloscopeDisplayMode::Right,
        OscilloscopeDisplayModeConfig::Stereo => OscilloscopeDisplayMode::Stereo,
    }
}

fn copy_channel(dest: &mut [f32], left: &[f32], right: &[f32], channel: SpectrumChannel) {
    match channel {
        SpectrumChannel::Left => dest.copy_from_slice(left),
        SpectrumChannel::Right => dest.copy_from_slice(right),
        SpectrumChannel::Sum => {
            for ((dest, left), right) in dest.iter_mut().zip(left).zip(right) {
                *dest = 0.5 * (*left + *right);
            }
        }
    }
}

pub fn show(ui: &mut egui::Ui, audio_blocks: VecDeque<AudioBlock>, state: &mut RealTimeState) {
    if state.osc.frozen {
        // Skip data intake so both the oscilloscope and spectrum stay frozen.
    } else {
        for block in audio_blocks {
            let block_len = (block.len as usize).min(MAX_AUDIO_BUF);
            let osc_copy_len = block_len.min(MAX_AUDIO_BUF);
            state.osc.input_buffer_l[..osc_copy_len]
                .copy_from_slice(&block.input_left[..osc_copy_len]);
            state.osc.input_buffer_r[..osc_copy_len]
                .copy_from_slice(&block.input_right[..osc_copy_len]);
            state.osc.output_buffer_l[..osc_copy_len]
                .copy_from_slice(&block.output_left[..osc_copy_len]);
            state.osc.output_buffer_r[..osc_copy_len]
                .copy_from_slice(&block.output_right[..osc_copy_len]);
            state.osc.buf_len = osc_copy_len;

            let fft_size = state.fft.fft_size;
            let copy_len = block_len.min(fft_size);
            let shift = fft_size - copy_len;
            state.fft.output_buffer.copy_within(copy_len..fft_size, 0);
            state.fft.input_buffer.copy_within(copy_len..fft_size, 0);
            copy_channel(
                &mut state.fft.output_buffer[shift..fft_size],
                &block.output_left[..copy_len],
                &block.output_right[..copy_len],
                state.fft.channel,
            );
            copy_channel(
                &mut state.fft.input_buffer[shift..fft_size],
                &block.input_left[..copy_len],
                &block.input_right[..copy_len],
                state.fft.channel,
            );
            state.fft.frame_count += 1;
        }
    }

    let available = ui.available_size();
    let gap = 12.0;
    let osc_h = (available.y * 0.4).max(120.0);
    let fft_h = (available.y - osc_h - gap).max(120.0);

    ui.allocate_ui(egui::vec2(available.x, osc_h), |ui| {
        ui.strong("Oscilloscope");
        ui.add_space(6.0);
        draw_oscilloscope(ui, &mut state.osc, state.sample_rate);
    });
    ui.add_space(gap);
    ui.allocate_ui(egui::vec2(available.x, fft_h), |ui| {
        ui.strong("Spectrum Analyzer");
        ui.add_space(6.0);
        draw_fft(ui, &mut state.fft, state.osc.frozen, state.sample_rate);
    });
}

pub struct OscilloscopeState {
    input_buffer_l: [f32; MAX_AUDIO_BUF],
    input_buffer_r: [f32; MAX_AUDIO_BUF],
    output_buffer_l: [f32; MAX_AUDIO_BUF],
    output_buffer_r: [f32; MAX_AUDIO_BUF],
    buf_len: usize,
    timebase_ms: f32,
    trigger_level: f32,
    y_range: f32,
    display_mode: OscilloscopeDisplayMode,
    source: SignalSource,
    frozen: bool,
    frozen_input_l: [f32; MAX_AUDIO_BUF],
    frozen_input_r: [f32; MAX_AUDIO_BUF],
    frozen_output_l: [f32; MAX_AUDIO_BUF],
    frozen_output_r: [f32; MAX_AUDIO_BUF],
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
            input_buffer_l: [0.0; MAX_AUDIO_BUF],
            input_buffer_r: [0.0; MAX_AUDIO_BUF],
            output_buffer_l: [0.0; MAX_AUDIO_BUF],
            output_buffer_r: [0.0; MAX_AUDIO_BUF],
            buf_len: 0,
            timebase_ms: 5.0,
            trigger_level: 0.0,
            y_range: 1.0,
            display_mode: OscilloscopeDisplayMode::Left,
            source: SignalSource::Output,
            frozen: false,
            frozen_input_l: [0.0; MAX_AUDIO_BUF],
            frozen_input_r: [0.0; MAX_AUDIO_BUF],
            frozen_output_l: [0.0; MAX_AUDIO_BUF],
            frozen_output_r: [0.0; MAX_AUDIO_BUF],
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

fn find_combined_trigger(first: &[f32], second: &[f32], len: usize, level: f32) -> f32 {
    for index in 0..len.saturating_sub(1) {
        let current = (first[index] + second[index]).clamp(-1.0, 1.0);
        let next = (first[index + 1] + second[index + 1]).clamp(-1.0, 1.0);
        if current < level && next >= level {
            let fraction = (level - current) / (next - current);
            return index as f32 + fraction;
        }
    }
    0.0
}

fn freeze_scope(state: &mut OscilloscopeState) {
    state.frozen_input_l = state.input_buffer_l;
    state.frozen_input_r = state.input_buffer_r;
    state.frozen_output_l = state.output_buffer_l;
    state.frozen_output_r = state.output_buffer_r;
    state.frozen_len = state.buf_len;
}

fn source_selector(ui: &mut egui::Ui, source: &mut SignalSource) -> bool {
    let before = *source;
    ui.label("Signal:");
    for (value, label, color) in [
        (SignalSource::Input, "I", Some(INPUT_LEFT_COLOR)),
        (SignalSource::Output, "O", Some(OUTPUT_LEFT_COLOR)),
        (SignalSource::InputAndOutput, "I+O", None),
    ] {
        let text = color.map_or_else(
            || egui::RichText::new(label),
            |color| egui::RichText::new(label).color(color),
        );
        if ui.selectable_label(*source == value, text).clicked() {
            *source = value;
        }
    }
    before != *source
}

fn draw_oscilloscope(ui: &mut egui::Ui, state: &mut OscilloscopeState, sample_rate: f32) {
    ui.horizontal_wrapped(|ui| {
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
        source_selector(ui, &mut state.source);
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
                freeze_scope(state);
            }
        }
    });
    ui.add_space(6.0);

    let available = ui.available_size();
    let y_label_w = 28.0;
    let x_label_h = 14.0;
    let top_pad = 8.0;
    let plot_h = available.y.max(40.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available.x, plot_h),
        egui::Sense::click_and_drag(),
    );
    let plot_left = rect.left() + y_label_w;
    let plot_rect = egui::Rect::from_min_max(
        egui::pos2(plot_left, rect.top() + top_pad),
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
            egui::Stroke::new(1.0_f32, grid_color),
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
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 70, 80)),
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
            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(255, 160, 40)),
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
            freeze_scope(state);
        }
    }

    let (input_l, input_r, output_l, output_r) = if state.frozen {
        (
            &state.frozen_input_l[..],
            &state.frozen_input_r[..],
            &state.frozen_output_l[..],
            &state.frozen_output_r[..],
        )
    } else {
        (
            &state.input_buffer_l[..],
            &state.input_buffer_r[..],
            &state.output_buffer_l[..],
            &state.output_buffer_r[..],
        )
    };
    let len = if state.frozen {
        state.frozen_len
    } else {
        state.buf_len
    };

    if len > 1 {
        let (input_trigger, output_trigger) = match state.display_mode {
            OscilloscopeDisplayMode::Left | OscilloscopeDisplayMode::Stereo => (input_l, output_l),
            OscilloscopeDisplayMode::Right => (input_r, output_r),
        };
        let trig_f32 = match state.source {
            SignalSource::Input => find_trigger(input_trigger, len, state.trigger_level),
            SignalSource::Output => find_trigger(output_trigger, len, state.trigger_level),
            SignalSource::InputAndOutput => {
                find_combined_trigger(input_trigger, output_trigger, len, state.trigger_level)
            }
        };
        let trig_idx = trig_f32 as usize;
        let samples_to_show = (state.timebase_ms / 1000.0 * sample_rate) as usize;
        let samples_to_show = samples_to_show.min(len.saturating_sub(trig_idx)).max(2);
        let start = trig_idx;
        let end = (start + samples_to_show).min(len);

        let overlay = state.source == SignalSource::InputAndOutput;
        if matches!(
            state.source,
            SignalSource::Output | SignalSource::InputAndOutput
        ) {
            draw_scope_source(
                &painter,
                plot_rect,
                center_y,
                display_yscale,
                output_l,
                output_r,
                state.display_mode,
                start,
                end,
                trig_f32,
                samples_to_show,
                OUTPUT_LEFT_COLOR,
                OUTPUT_RIGHT_COLOR,
                overlay,
            );
        }
        if matches!(
            state.source,
            SignalSource::Input | SignalSource::InputAndOutput
        ) {
            draw_scope_source(
                &painter,
                plot_rect,
                center_y,
                display_yscale,
                input_l,
                input_r,
                state.display_mode,
                start,
                end,
                trig_f32,
                samples_to_show,
                INPUT_LEFT_COLOR,
                INPUT_RIGHT_COLOR,
                overlay,
            );
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
                    egui::Stroke::new(1.0_f32, grid_color),
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
}

#[allow(clippy::too_many_arguments)]
fn draw_scope_source(
    painter: &egui::Painter,
    plot_rect: egui::Rect,
    center_y: f32,
    display_yscale: f32,
    left: &[f32],
    right: &[f32],
    mode: OscilloscopeDisplayMode,
    start: usize,
    end: usize,
    trigger: f32,
    samples_to_show: usize,
    left_color: egui::Color32,
    right_color: egui::Color32,
    translucent: bool,
) {
    let color = |color: egui::Color32| {
        if translucent {
            egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 190)
        } else {
            color
        }
    };
    if matches!(
        mode,
        OscilloscopeDisplayMode::Left | OscilloscopeDisplayMode::Stereo
    ) {
        draw_oscilloscope_trace(
            painter,
            plot_rect,
            center_y,
            display_yscale,
            left,
            start,
            end,
            trigger,
            samples_to_show,
            color(left_color),
        );
    }
    if matches!(
        mode,
        OscilloscopeDisplayMode::Right | OscilloscopeDisplayMode::Stereo
    ) {
        draw_oscilloscope_trace(
            painter,
            plot_rect,
            center_y,
            display_yscale,
            right,
            start,
            end,
            trigger,
            samples_to_show,
            color(right_color),
        );
    }
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
        painter.add(PathShape::line(pts, egui::Stroke::new(1.2_f32, color)));
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
    input_buffer: [f32; 4096],
    output_buffer: [f32; 4096],
    input_latest_db: [f32; 2048],
    output_latest_db: [f32; 2048],
    input_peak_hold: [f32; 2048],
    output_peak_hold: [f32; 2048],
    peak_decay: f32,
    frame_count: u32,
    pub fft_size: usize,
    complex_buf: Vec<Complex32>,
    fft: Option<Arc<dyn Fft<f32>>>,
    pub window_type: usize,
    pub db_floor: f32,
    pub db_top: f32,
    pub log_scale: bool,
    pub show_peak_hold: bool,
    pub channel: SpectrumChannel,
    pub source: SignalSource,
}

impl Default for FftState {
    fn default() -> Self {
        let fft_size = 4096;
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        Self {
            input_buffer: [0.0; 4096],
            output_buffer: [0.0; 4096],
            input_latest_db: [-120.0; 2048],
            output_latest_db: [-120.0; 2048],
            input_peak_hold: [-120.0; 2048],
            output_peak_hold: [-120.0; 2048],
            peak_decay: 0.5,
            frame_count: 0,
            fft_size,
            complex_buf: vec![Complex32::new(0.0, 0.0); fft_size],
            fft: Some(fft),
            window_type: 0,
            db_floor: -96.0,
            db_top: 0.0,
            log_scale: true,
            show_peak_hold: false,
            channel: SpectrumChannel::Left,
            source: SignalSource::Output,
        }
    }
}

impl FftState {
    fn clear_history(&mut self) {
        self.input_buffer.fill(0.0);
        self.output_buffer.fill(0.0);
        self.input_latest_db.fill(self.db_floor);
        self.output_latest_db.fill(self.db_floor);
        self.input_peak_hold.fill(self.db_floor);
        self.output_peak_hold.fill(self.db_floor);
    }

    fn clear_peaks(&mut self) {
        self.input_peak_hold.fill(self.db_floor);
        self.output_peak_hold.fill(self.db_floor);
    }
}

#[allow(clippy::too_many_arguments)]
fn process_fft_trace(
    samples: &[f32],
    latest_db: &mut [f32],
    peak_hold: &mut [f32],
    complex_buf: &mut [Complex32],
    fft: &Arc<dyn Fft<f32>>,
    fft_size: usize,
    window_type: usize,
    peak_decay: f32,
) {
    for index in 0..fft_size {
        let phase = TAU * index as f32 / (fft_size - 1) as f32;
        let window = match window_type {
            0 => 0.5 * (1.0 - phase.cos()),
            1 => 0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos(),
            2 => {
                0.21557895 - 0.41663158 * phase.cos() + 0.277263158 * (2.0 * phase).cos()
                    - 0.083578947 * (3.0 * phase).cos()
                    + 0.006947368 * (4.0 * phase).cos()
            }
            _ => 1.0,
        };
        complex_buf[index] = Complex32::new(samples[index] * window, 0.0);
    }
    fft.process(&mut complex_buf[..fft_size]);
    let scale = 1.0 / fft_size as f32;
    for bin in 0..fft_size / 2 {
        let value = complex_buf[bin];
        let mag = (value.re * value.re + value.im * value.im).sqrt() * scale;
        let db = 20.0 * mag.max(1e-10).log10().max(-150.0);
        latest_db[bin] = db;
        if db > peak_hold[bin] {
            peak_hold[bin] = db;
        } else {
            peak_hold[bin] += (db - peak_hold[bin]) * peak_decay * 0.05;
        }
    }
}

fn draw_fft(ui: &mut egui::Ui, state: &mut FftState, frozen: bool, sample_rate: f32) {
    let fft_size = state.fft_size;
    if !frozen && state.frame_count % 4 == 0 {
        if state.complex_buf.len() != fft_size || state.fft.is_none() {
            state.complex_buf = vec![Complex32::new(0.0, 0.0); fft_size];
            let mut planner = FftPlanner::new();
            state.fft = Some(planner.plan_fft_forward(fft_size));
        }
        let fft = state.fft.as_ref().expect("FFT plan initialized").clone();
        if matches!(
            state.source,
            SignalSource::Input | SignalSource::InputAndOutput
        ) {
            process_fft_trace(
                &state.input_buffer,
                &mut state.input_latest_db,
                &mut state.input_peak_hold,
                &mut state.complex_buf,
                &fft,
                fft_size,
                state.window_type,
                state.peak_decay,
            );
        }
        if matches!(
            state.source,
            SignalSource::Output | SignalSource::InputAndOutput
        ) {
            process_fft_trace(
                &state.output_buffer,
                &mut state.output_latest_db,
                &mut state.output_peak_hold,
                &mut state.complex_buf,
                &fft,
                fft_size,
                state.window_type,
                state.peak_decay,
            );
        }
    }

    // Controls
    ui.horizontal_wrapped(|ui| {
        ui.label("Mode:");
        if ui
            .selectable_label(!state.show_peak_hold, "Instant")
            .on_hover_text("Show the current FFT frame without peak hold.")
            .clicked()
        {
            state.show_peak_hold = false;
        }
        if ui
            .selectable_label(state.show_peak_hold, "Hold")
            .on_hover_text("Show peak hold with slow decay.")
            .clicked()
        {
            state.show_peak_hold = true;
        }
        if ui
            .button("Reset")
            .on_hover_text("Clear held spectrum peaks.")
            .clicked()
        {
            state.clear_peaks();
        }
        ui.separator();
        if source_selector(ui, &mut state.source) {
            state.clear_history();
        }
        ui.separator();
        ui.label("Chan:");
        for (chan, label) in [
            (SpectrumChannel::Left, "L"),
            (SpectrumChannel::Right, "R"),
            (SpectrumChannel::Sum, "L+R"),
        ] {
            if ui.selectable_label(state.channel == chan, label).clicked() && state.channel != chan
            {
                state.channel = chan;
                state.clear_history();
            }
        }
        ui.separator();
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
                state.clear_history();
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
    });
    ui.add_space(6.0);

    let config = SpectrumConfig {
        fft_size: state.fft_size,
        sample_rate,
        db_floor: state.db_floor,
        db_top: state.db_top,
        log_scale: state.log_scale,
        min_freq: 20.0,
    };
    let num_bins = state.fft_size / 2;
    let input_db = if state.show_peak_hold {
        &state.input_peak_hold[..num_bins]
    } else {
        &state.input_latest_db[..num_bins]
    };
    let output_db = if state.show_peak_hold {
        &state.output_peak_hold[..num_bins]
    } else {
        &state.output_latest_db[..num_bins]
    };
    const HOVER_READOUT_H: f32 = 24.0;
    let input_trace = spectrum::SpectrumTrace {
        db_values: input_db,
        color: INPUT_LEFT_COLOR,
    };
    let output_trace = spectrum::SpectrumTrace {
        db_values: output_db,
        color: OUTPUT_LEFT_COLOR,
    };
    let plot_rect = match state.source {
        SignalSource::Input => {
            spectrum::render_spectra(ui, &[input_trace], &config, HOVER_READOUT_H)
        }
        SignalSource::Output => {
            spectrum::render_spectra(ui, &[output_trace], &config, HOVER_READOUT_H)
        }
        SignalSource::InputAndOutput => {
            spectrum::render_spectra(ui, &[output_trace, input_trace], &config, HOVER_READOUT_H)
        }
    };

    let hover_info = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|pos| plot_rect.contains(*pos))
        .map(|pos| {
            let max_freq = sample_rate * 0.5;
            let x_frac = ((pos.x - plot_rect.left()) / plot_rect.width()).clamp(0.0, 1.0);
            let freq = if state.log_scale {
                20.0_f32 * (max_freq / 20.0_f32).powf(x_frac)
            } else {
                max_freq * x_frac
            };
            let bin_hz = sample_rate / state.fft_size as f32;
            let bin = ((freq / bin_hz).floor() as usize).clamp(0, num_bins.saturating_sub(1));
            let input_level = input_db[bin];
            let output_level = output_db[bin];
            let db = match state.source {
                SignalSource::Input => input_level,
                SignalSource::Output => output_level,
                SignalSource::InputAndOutput => input_level.max(output_level),
            };
            let x = plot_rect.left() + x_frac * plot_rect.width();
            let db_range = (state.db_top - state.db_floor).max(1.0);
            let y_frac = ((db - state.db_floor) / db_range).clamp(0.0, 1.0);
            let y = plot_rect.bottom() - y_frac * plot_rect.height();
            (freq, input_level, output_level, x, y)
        });

    if let Some((_freq, _input_db, _output_db, x, y)) = hover_info {
        let painter = ui.painter_at(plot_rect);
        painter.line_segment(
            [
                egui::pos2(x, plot_rect.top()),
                egui::pos2(x, plot_rect.bottom()),
            ],
            egui::Stroke::new(
                1.0_f32,
                egui::Color32::from_rgba_premultiplied(180, 235, 255, 80),
            ),
        );
        painter.line_segment(
            [egui::pos2(x, y), egui::pos2(x + 6.0, y)],
            egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(230, 250, 255)),
        );
        painter.circle_filled(
            egui::pos2(x, y),
            3.0,
            egui::Color32::from_rgb(230, 250, 255),
        );
    }

    ui.horizontal(|ui| {
        if let Some((freq, input_db, output_db, _, _)) = hover_info {
            let levels = match state.source {
                SignalSource::Input => format!("I: {input_db:+.1} dB"),
                SignalSource::Output => format!("O: {output_db:+.1} dB"),
                SignalSource::InputAndOutput => {
                    format!("I: {input_db:+.1} dB   O: {output_db:+.1} dB")
                }
            };
            ui.label(format!(
                "Freq: {}   {}   Note: {}",
                format_hz(freq),
                levels,
                format_midi_note(freq)
            ));
        } else {
            ui.label("Freq: -   Level: -   Note: -");
        }
    });
}

fn format_hz(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else {
        format!("{:.0} Hz", hz)
    }
}

/// Convert a frequency to the nearest MIDI note, returning both the numeric
/// note number and its letter notation (e.g. "63 (D#4)").
fn format_midi_note(hz: f32) -> String {
    if hz <= 0.0 {
        return "-".to_string();
    }
    const NOTE_NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round() as i32;
    if !(0..=127).contains(&midi) {
        return "-".to_string();
    }
    let name = NOTE_NAMES[(midi % 12) as usize];
    let octave = midi / 12 - 1;
    format!("{} ({}{})", midi, name, octave)
}

#[cfg(test)]
mod tests {
    use super::{SignalSource, SpectrumChannel, copy_channel, find_combined_trigger};

    #[test]
    fn source_defaults_to_internal_output() {
        assert!(matches!(SignalSource::default(), SignalSource::Output));
    }

    #[test]
    fn channel_sum_averages_stereo_samples() {
        let left = [1.0, -0.5];
        let right = [-0.5, 0.25];
        let mut dest = [0.0; 2];

        copy_channel(&mut dest, &left, &right, SpectrumChannel::Sum);

        assert_eq!(dest, [0.25, -0.125]);
    }

    #[test]
    fn combined_trigger_uses_both_sources() {
        let input = [-0.4, -0.2, 0.1, 0.2];
        let output = [-0.4, -0.1, 0.2, 0.3];

        let trigger = find_combined_trigger(&input, &output, input.len(), 0.0);

        assert!((trigger - 1.5).abs() < f32::EPSILON);
    }
}
