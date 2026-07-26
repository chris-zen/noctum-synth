use eframe::egui;
use eframe::egui::PointerButton;
use eframe::egui::epaint::PathShape;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::engine::{AudioBlock, MAX_AUDIO_BUF};

use super::real_time::{
    fill_fft_from_captured, process_fft_trace, FftState, SignalSource,
};

pub(crate) const MAX_SCOPE_SAMPLES: usize = 65536;
const DEFAULT_JUMP_THRESHOLD: f32 = 0.5;

const INPUT_LEFT_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 150, 45);
const INPUT_RIGHT_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 205, 80);
const OUTPUT_LEFT_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 205, 255);
const OUTPUT_RIGHT_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 125, 255);

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

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
pub enum TriggerSlope {
    Rising,
    Falling,
    Both,
}

impl Default for TriggerSlope {
    fn default() -> Self {
        Self::Rising
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerMode {
    Auto,
    Normal,
}

impl Default for TriggerMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OscilloscopeDisplayMode {
    Left,
    Right,
    Stereo,
}

// ---------------------------------------------------------------------------
// Config
// Config
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct OscilloscopeViewConfig {
    pub timebase_ms: f32,
    pub trigger_level: f32,
    pub y_range: f32,
    pub display_mode: OscilloscopeDisplayModeConfig,
    #[serde(default)]
    pub source: SignalSource,
    #[serde(default)]
    pub trigger_slope: TriggerSlope,
    #[serde(default)]
    pub trigger_mode: TriggerMode,
    #[serde(default = "default_jump_threshold_val")]
    pub jump_threshold: f32,
    #[serde(default = "default_capture_duration")]
    pub capture_duration_ms: f32,
}

fn default_capture_duration() -> f32 {
    500.0
}

fn default_jump_threshold_val() -> f32 {
    DEFAULT_JUMP_THRESHOLD
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
            trigger_slope: state.trigger_slope,
            trigger_mode: state.trigger_mode,
            jump_threshold: state.jump_threshold,
            capture_duration_ms: state.capture_duration_ms,
        }
    }

    pub fn apply_to(&self, state: &mut OscilloscopeState) {
        state.timebase_ms = self.timebase_ms;
        state.trigger_level = self.trigger_level;
        state.y_range = self.y_range;
        state.display_mode = display_mode_from_config(self.display_mode);
        state.source = self.source;
        state.trigger_slope = self.trigger_slope;
        state.trigger_mode = self.trigger_mode;
        state.jump_threshold = self.jump_threshold;
        state.capture_duration_ms = self.capture_duration_ms;
    }
}

pub(crate) fn display_mode_to_config(
    mode: OscilloscopeDisplayMode,
) -> OscilloscopeDisplayModeConfig {
    match mode {
        OscilloscopeDisplayMode::Left => OscilloscopeDisplayModeConfig::Left,
        OscilloscopeDisplayMode::Right => OscilloscopeDisplayModeConfig::Right,
        OscilloscopeDisplayMode::Stereo => OscilloscopeDisplayModeConfig::Stereo,
    }
}

pub(crate) fn display_mode_from_config(
    mode: OscilloscopeDisplayModeConfig,
) -> OscilloscopeDisplayMode {
    match mode {
        OscilloscopeDisplayModeConfig::Left => OscilloscopeDisplayMode::Left,
        OscilloscopeDisplayModeConfig::Right => OscilloscopeDisplayMode::Right,
        OscilloscopeDisplayModeConfig::Stereo => OscilloscopeDisplayMode::Stereo,
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct OscilloscopeState {
    pub(crate) input_buffer_l: Vec<f32>,
    pub(crate) input_buffer_r: Vec<f32>,
    pub(crate) output_buffer_l: Vec<f32>,
    pub(crate) output_buffer_r: Vec<f32>,
    pub(crate) buf_len: usize,
    pub(crate) timebase_ms: f32,
    pub(crate) trigger_level: f32,
    pub(crate) y_range: f32,
    pub(crate) display_mode: OscilloscopeDisplayMode,
    pub(crate) source: SignalSource,
    pub(crate) trigger_slope: TriggerSlope,
    pub(crate) trigger_mode: TriggerMode,
    pub(crate) jump_threshold: f32,
    pub(crate) capture_duration_ms: f32,

    pub(crate) captured: bool,
    capture_armed: bool,
    capture_trigger_found: bool,
    capture_circ_il: Vec<f32>,
    capture_circ_ir: Vec<f32>,
    capture_circ_ol: Vec<f32>,
    capture_circ_or: Vec<f32>,
    capture_circ_target: usize,
    capture_circ_write: usize,
    capture_circ_start: usize,
    capture_circ_count: usize,
    capture_trig_pos: f32,
    pub(crate) captured_input_l: Vec<f32>,
    pub(crate) captured_input_r: Vec<f32>,
    pub(crate) captured_output_l: Vec<f32>,
    pub(crate) captured_output_r: Vec<f32>,
    pub(crate) captured_len: usize,
    pub(crate) captured_view_offset: f32,
    pub(crate) fft_window_start: Option<f32>,
}

impl Default for OscilloscopeState {
    fn default() -> Self {
        Self {
            input_buffer_l: Vec::with_capacity(MAX_SCOPE_SAMPLES),
            input_buffer_r: Vec::with_capacity(MAX_SCOPE_SAMPLES),
            output_buffer_l: Vec::with_capacity(MAX_SCOPE_SAMPLES),
            output_buffer_r: Vec::with_capacity(MAX_SCOPE_SAMPLES),
            buf_len: 0,
            timebase_ms: 5.0,
            trigger_level: 0.0,
            y_range: 1.0,
            display_mode: OscilloscopeDisplayMode::Left,
            source: SignalSource::Output,
            trigger_slope: TriggerSlope::Rising,
            trigger_mode: TriggerMode::Auto,
            jump_threshold: DEFAULT_JUMP_THRESHOLD,
            capture_duration_ms: 500.0,
            captured: false,
            capture_armed: false,
            capture_trigger_found: false,
            capture_circ_il: Vec::new(),
            capture_circ_ir: Vec::new(),
            capture_circ_ol: Vec::new(),
            capture_circ_or: Vec::new(),
            capture_circ_target: 0,
            capture_circ_write: 0,
            capture_circ_start: 0,
            capture_circ_count: 0,
            capture_trig_pos: 0.0,
            captured_input_l: Vec::new(),
            captured_input_r: Vec::new(),
            captured_output_l: Vec::new(),
            captured_output_r: Vec::new(),
            captured_len: 0,
            captured_view_offset: 0.0,
            fft_window_start: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Trigger functions
// ---------------------------------------------------------------------------

pub(crate) fn find_trigger(
    buf: &[f32],
    len: usize,
    level: f32,
    slope: TriggerSlope,
) -> Option<f32> {
    match slope {
        TriggerSlope::Rising => {
            for index in (1..len).rev() {
                if buf[index - 1] < level && buf[index] >= level {
                    let fraction =
                        (level - buf[index - 1]) / (buf[index] - buf[index - 1]);
                    return Some((index - 1) as f32 + fraction);
                }
            }
        }
        TriggerSlope::Falling => {
            for index in (1..len).rev() {
                if buf[index - 1] > level && buf[index] <= level {
                    let fraction =
                        (buf[index - 1] - level) / (buf[index - 1] - buf[index]);
                    return Some((index - 1) as f32 + fraction);
                }
            }
        }
        TriggerSlope::Both => {
            for index in (1..len).rev() {
                let prev = buf[index - 1];
                let curr = buf[index];
                if (prev < level && curr >= level) || (prev > level && curr <= level) {
                    let fraction = if prev < level {
                        (level - prev) / (curr - prev)
                    } else {
                        (prev - level) / (prev - curr)
                    };
                    return Some((index - 1) as f32 + fraction);
                }
            }
        }
    }
    None
}

pub(crate) fn find_combined_trigger(
    first: &[f32],
    second: &[f32],
    len: usize,
    level: f32,
    slope: TriggerSlope,
) -> Option<f32> {
    match slope {
        TriggerSlope::Rising => {
            for index in (1..len).rev() {
                let prev = (first[index - 1] + second[index - 1]).clamp(-1.0, 1.0);
                let curr = (first[index] + second[index]).clamp(-1.0, 1.0);
                if prev < level && curr >= level {
                    let fraction = (level - prev) / (curr - prev);
                    return Some((index - 1) as f32 + fraction);
                }
            }
        }
        TriggerSlope::Falling => {
            for index in (1..len).rev() {
                let prev = (first[index - 1] + second[index - 1]).clamp(-1.0, 1.0);
                let curr = (first[index] + second[index]).clamp(-1.0, 1.0);
                if prev > level && curr <= level {
                    let fraction = (prev - level) / (prev - curr);
                    return Some((index - 1) as f32 + fraction);
                }
            }
        }
        TriggerSlope::Both => {
            for index in (1..len).rev() {
                let prev = (first[index - 1] + second[index - 1]).clamp(-1.0, 1.0);
                let curr = (first[index] + second[index]).clamp(-1.0, 1.0);
                if (prev < level && curr >= level) || (prev > level && curr <= level) {
                    let fraction = if prev < level {
                        (level - prev) / (curr - prev)
                    } else {
                        (prev - level) / (prev - curr)
                    };
                    return Some((index - 1) as f32 + fraction);
                }
            }
        }
    }
    None
}

fn find_trigger_offset(buf: &[f32], level: f32, slope: TriggerSlope) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    match slope {
        TriggerSlope::Rising => {
            for i in 1..buf.len() {
                if buf[i - 1] < level && buf[i] >= level {
                    return Some(i - 1);
                }
            }
        }
        TriggerSlope::Falling => {
            for i in 1..buf.len() {
                if buf[i - 1] > level && buf[i] <= level {
                    return Some(i - 1);
                }
            }
        }
        TriggerSlope::Both => {
            for i in 1..buf.len() {
                let p = buf[i - 1];
                let c = buf[i];
                if (p < level && c >= level) || (p > level && c <= level) {
                    return Some(i - 1);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

fn capture_scope(state: &mut OscilloscopeState) {
    let start = state.capture_circ_start;
    let n = state.capture_circ_target;
    let il = &state.capture_circ_il;
    let ir = &state.capture_circ_ir;
    let ol = &state.capture_circ_ol;
    let or = &state.capture_circ_or;
    let first = n - start;
    let mut out_il = Vec::with_capacity(n);
    let mut out_ir = Vec::with_capacity(n);
    let mut out_ol = Vec::with_capacity(n);
    let mut out_or = Vec::with_capacity(n);
    out_il.extend_from_slice(&il[start..start + first]);
    out_ir.extend_from_slice(&ir[start..start + first]);
    out_ol.extend_from_slice(&ol[start..start + first]);
    out_or.extend_from_slice(&or[start..start + first]);
    let second = n - first;
    if second > 0 {
        out_il.extend_from_slice(&il[..second]);
        out_ir.extend_from_slice(&ir[..second]);
        out_ol.extend_from_slice(&ol[..second]);
        out_or.extend_from_slice(&or[..second]);
    }
    state.captured_input_l = out_il;
    state.captured_input_r = out_ir;
    state.captured_output_l = out_ol;
    state.captured_output_r = out_or;
    state.captured_len = n;
    state.captured_view_offset = 0.0;
    state.captured = true;
}

pub(crate) fn finalize_capture(
    osc: &mut OscilloscopeState,
    fft: &mut FftState,
    sample_rate: f32,
) {
    osc.capture_armed = false;
    osc.capture_trigger_found = false;
    let trig = osc.capture_trig_pos;
    capture_scope(osc);
    let visible = (osc.timebase_ms / 1000.0 * sample_rate).max(2.0);
    osc.captured_view_offset = (trig - 0.2 * visible).max(0.0);
    compute_fft_on_capture(fft, osc, sample_rate);
}

fn compute_fft_on_capture(
    fft_state: &mut FftState,
    osc: &OscilloscopeState,
    sample_rate: f32,
) {
    let fft_size = fft_state.fft_size;
    if fft_state.complex_buf.len() != fft_size || fft_state.fft.is_none() {
        fft_state.complex_buf = vec![rustfft::num_complex::Complex32::new(0.0, 0.0); fft_size];
        let mut planner = rustfft::FftPlanner::new();
        fft_state.fft = Some(planner.plan_fft_forward(fft_size));
    }
    fill_fft_from_captured(
        fft_state,
        &osc.captured_input_l,
        &osc.captured_input_r,
        &osc.captured_output_l,
        &osc.captured_output_r,
        osc.captured_len,
        osc.captured_view_offset,
        osc.timebase_ms,
        sample_rate,
        None,
    );
    let fft = fft_state.fft.as_ref().cloned();
    if let Some(fft) = fft {
        if matches!(
            fft_state.source,
            SignalSource::Input | SignalSource::InputAndOutput
        ) {
            process_fft_trace(
                &fft_state.input_buffer,
                &mut fft_state.input_latest_db,
                &mut fft_state.input_peak_hold,
                &mut fft_state.complex_buf,
                &fft,
                fft_size,
                fft_state.window_type,
                fft_state.peak_decay,
            );
        }
        if matches!(
            fft_state.source,
            SignalSource::Output | SignalSource::InputAndOutput
        ) {
            process_fft_trace(
                &fft_state.output_buffer,
                &mut fft_state.output_latest_db,
                &mut fft_state.output_peak_hold,
                &mut fft_state.complex_buf,
                &fft,
                fft_size,
                fft_state.window_type,
                fft_state.peak_decay,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Audio feed
// ---------------------------------------------------------------------------

/// Feeds audio blocks into live buffers and circular capture buffer.
/// Returns true if capture just finalized (caller should compute FFT).
pub(crate) fn feed_audio(
    osc: &mut OscilloscopeState,
    fft: &mut FftState,
    audio_blocks: VecDeque<AudioBlock>,
    sample_rate: f32,
) -> bool {
    let mut captured = false;
    if osc.captured {
        return false;
    }
    for block in audio_blocks {
        let block_len = (block.len as usize).min(MAX_AUDIO_BUF);

        osc.input_buffer_l
            .extend_from_slice(&block.input_left[..block_len]);
        osc.input_buffer_r
            .extend_from_slice(&block.input_right[..block_len]);
        osc.output_buffer_l
            .extend_from_slice(&block.output_left[..block_len]);
        osc.output_buffer_r
            .extend_from_slice(&block.output_right[..block_len]);

        let excess = osc
            .input_buffer_l
            .len()
            .saturating_sub((osc.capture_duration_ms / 1000.0 * sample_rate) as usize);
        if excess > 0 {
            osc.input_buffer_l.drain(..excess);
            osc.input_buffer_r.drain(..excess);
            osc.output_buffer_l.drain(..excess);
            osc.output_buffer_r.drain(..excess);
        }
        osc.buf_len = osc.input_buffer_l.len();

        // Circular capture buffer
        let tgt = osc.capture_circ_target;
        if osc.capture_armed && tgt > 0 {
            let write_before = osc.capture_circ_write;

            let mut trig_in_block = false;
            let mut trig_offset = 0;
            if !osc.capture_trigger_found {
                let use_left = osc.display_mode
                    != OscilloscopeDisplayMode::Right;
                let trig_chan = match osc.source {
                    SignalSource::Input => if use_left {
                        &block.input_left[..block_len]
                    } else {
                        &block.input_right[..block_len]
                    },
                    SignalSource::Output | SignalSource::InputAndOutput => if use_left {
                        &block.output_left[..block_len]
                    } else {
                        &block.output_right[..block_len]
                    },
                };
                if let Some(off) = find_trigger_offset(
                    trig_chan,
                    osc.trigger_level,
                    osc.trigger_slope,
                ) {
                    trig_in_block = true;
                    trig_offset = off;
                    osc.capture_trigger_found = true;
                    osc.capture_circ_start =
                        (write_before + off) % tgt;
                    osc.capture_trig_pos = 0.0;
                }
            }

            let write_len = if trig_in_block {
                block_len.min(tgt)
            } else if osc.capture_trigger_found {
                let need = tgt
                    .saturating_sub(1)
                    .saturating_sub(osc.capture_circ_count);
                block_len.min(need)
            } else {
                block_len
            };

            if trig_in_block {
                osc.capture_circ_count =
                    write_len.saturating_sub(trig_offset + 1);
            } else if osc.capture_trigger_found {
                osc.capture_circ_count += write_len;
            }

            if write_len > 0 {
                let mut write = write_before;
                let first = write_len.min(tgt - write);
                osc.capture_circ_il[write..write + first]
                    .copy_from_slice(&block.input_left[..first]);
                osc.capture_circ_ir[write..write + first]
                    .copy_from_slice(&block.input_right[..first]);
                osc.capture_circ_ol[write..write + first]
                    .copy_from_slice(&block.output_left[..first]);
                osc.capture_circ_or[write..write + first]
                    .copy_from_slice(&block.output_right[..first]);
                write = (write + first) % tgt;
                if write_len > first {
                    let second = write_len - first;
                    osc.capture_circ_il[0..second]
                        .copy_from_slice(&block.input_left[first..write_len]);
                    osc.capture_circ_ir[0..second]
                        .copy_from_slice(&block.input_right[first..write_len]);
                    osc.capture_circ_ol[0..second]
                        .copy_from_slice(&block.output_left[first..write_len]);
                    osc.capture_circ_or[0..second]
                        .copy_from_slice(&block.output_right[first..write_len]);
                    write = second % tgt;
                }
                osc.capture_circ_write = write;
            }
        }

        fft.frame_count += 1;
    }

    if osc.capture_armed
        && osc.capture_trigger_found
        && osc.capture_circ_count
            >= osc.capture_circ_target.saturating_sub(1)
    {
        finalize_capture(osc, fft, sample_rate);
        captured = true;
    }

    captured
}

// ---------------------------------------------------------------------------
// UI helpers
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn draw_discontinuities(
    painter: &egui::Painter,
    plot_rect: egui::Rect,
    _center_y: f32,
    buffer: &[f32],
    start: usize,
    end: usize,
    trig_f32: f32,
    samples_to_show: usize,
    threshold: f32,
    color: egui::Color32,
) {
    if end <= start + 1 {
        return;
    }
    for idx in (start + 1)..end {
        let diff = (buffer[idx] - buffer[idx - 1]).abs();
        if diff > threshold {
            let x = plot_rect.left()
                + plot_rect.width() * (idx as f32 - trig_f32) / samples_to_show as f32;
            if x >= plot_rect.left() && x <= plot_rect.right() {
                painter.line_segment(
                    [
                        egui::pos2(x, plot_rect.top()),
                        egui::pos2(x, plot_rect.bottom()),
                    ],
                    egui::Stroke::new(1.0_f32, color),
                );
            }
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
    let pts: Vec<egui::Pos2> = (start..end.min(buffer.len()))
        .map(|sample_index| {
            let point_x = plot_rect.left()
                + plot_rect.width() * (sample_index as f32 - trig_f32)
                    / samples_to_show as f32;
            let point_y = center_y - buffer[sample_index] * display_yscale;
            egui::pos2(point_x, point_y.clamp(plot_rect.top(), plot_rect.bottom()))
        })
        .collect();

    if pts.len() >= 2 {
        painter.add(PathShape::line(
            pts,
            egui::Stroke::new(1.2_f32, color),
        ));
    }
}

// ---------------------------------------------------------------------------
// Main render
// ---------------------------------------------------------------------------

pub(crate) fn draw_oscilloscope(
    ui: &mut egui::Ui,
    state: &mut OscilloscopeState,
    sample_rate: f32,
    fft_size: usize,
) {
    let scroll_input = ui.ctx().input(|i| i.smooth_scroll_delta);
    let zoom_input = ui.ctx().input(|i| i.zoom_delta());
    let cmd_held = ui.ctx().input(|i| i.modifiers.command);
    let cursor_pos = ui.ctx().input(|i| i.pointer.hover_pos());

    // -- Bar 1: Capture / Trigger --
    ui.horizontal_wrapped(|ui| {
        if state.captured {
            if ui
                .button("▶ Live")
                .on_hover_text("Return to live streaming")
                .clicked()
            {
                state.captured = false;
                state.fft_window_start = None;
            }
        } else if state.capture_armed {
            if state.capture_trigger_found {
                let pct = if state.capture_circ_target > 1 {
                    let c = state.capture_circ_count;
                    ((c as f32 / (state.capture_circ_target - 1) as f32) * 100.0)
                        .min(99.0) as u32
                } else {
                    0
                };
                ui.label(
                    egui::RichText::new(format!("Capturing {pct}%"))
                        .color(egui::Color32::from_rgb(100, 200, 100)),
                );
            } else {
                ui.label(
                    egui::RichText::new("Waiting...")
                        .color(egui::Color32::from_rgb(180, 180, 100)),
                );
            }
            if ui
                .button("Cancel")
                .on_hover_text("Cancel capture and return to live")
                .clicked()
            {
                state.capture_armed = false;
                state.capture_trigger_found = false;
                state.capture_circ_il.clear();
                state.capture_circ_ir.clear();
                state.capture_circ_ol.clear();
                state.capture_circ_or.clear();
                state.capture_circ_target = 0;
            }
        } else if ui
            .button("Capture")
            .on_hover_text("Arm capture; freezes on next trigger")
            .clicked()
        {
            state.input_buffer_l.clear();
            state.input_buffer_r.clear();
            state.output_buffer_l.clear();
            state.output_buffer_r.clear();
            state.buf_len = 0;
            let tgt =
                ((state.capture_duration_ms / 1000.0 * sample_rate) as usize)
                    .max(64);
            state.capture_circ_il = vec![0.0f32; tgt];
            state.capture_circ_ir = vec![0.0f32; tgt];
            state.capture_circ_ol = vec![0.0f32; tgt];
            state.capture_circ_or = vec![0.0f32; tgt];
            state.capture_circ_target = tgt;
            state.capture_circ_write = 0;
            state.capture_circ_start = 0;
            state.capture_circ_count = 0;
            state.capture_trig_pos = 0.0;
            state.capture_armed = true;
            state.capture_trigger_found = false;
        }
        ui.separator();
        ui.label("Level:");
        ui.add(
            egui::Slider::new(&mut state.trigger_level, -1.0..=1.0)
                .text("")
                .trailing_fill(true),
        )
        .on_hover_text("Trigger threshold: higher = only louder signals capture");
        ui.separator();
        ui.label("Slope:");
        if ui
            .selectable_label(
                state.trigger_slope == TriggerSlope::Rising,
                egui::RichText::new("↗"),
            )
            .on_hover_text("Rising edge trigger")
            .clicked()
        {
            state.trigger_slope = TriggerSlope::Rising;
        }
        if ui
            .selectable_label(
                state.trigger_slope == TriggerSlope::Falling,
                egui::RichText::new("↘"),
            )
            .on_hover_text("Falling edge trigger")
            .clicked()
        {
            state.trigger_slope = TriggerSlope::Falling;
        }
        if ui
            .selectable_label(
                state.trigger_slope == TriggerSlope::Both,
                egui::RichText::new("↕"),
            )
            .on_hover_text("Either edge trigger")
            .clicked()
        {
            state.trigger_slope = TriggerSlope::Both;
        }
        ui.separator();
        ui.label("Length:");
        ui.add(
            egui::Slider::new(&mut state.capture_duration_ms, 100.0..=5000.0)
                .logarithmic(true)
                .text("ms"),
        );
        if state.captured {
            ui.separator();
            ui.label("Jmp:");
            ui.add(
                egui::Slider::new(&mut state.jump_threshold, 0.0..=1.0)
                    .text(""),
            )
            .on_hover_text("Show sample-to-sample jumps exceeding this threshold");
        }
    });

    // -- Bar 2: View parameters --
    ui.horizontal_wrapped(|ui| {
        ui.label("X (ms):");
        let buf_duration_ms = if state.captured {
            state.captured_len as f32 / sample_rate * 1000.0
        } else if state.capture_armed {
            state.capture_duration_ms
        } else {
            state.buf_len as f32 / sample_rate * 1000.0
        };

        let clamp_ms = if state.captured {
            state.captured_len as f32 / sample_rate * 1000.0
        } else {
            state.capture_duration_ms
        };
        state.timebase_ms = state.timebase_ms.clamp(1.0, clamp_ms);

        let slider_max = buf_duration_ms.max(state.timebase_ms).max(1.0);
        ui.add(
            egui::Slider::new(&mut state.timebase_ms, 1.0..=slider_max)
                .logarithmic(true)
                .text("ms"),
        );
        ui.separator();
        ui.label("Y:");
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
    let display_yscale = y_scale / state.y_range;
    let font_id = egui::FontId::monospace(8.0);
    let label_color = egui::Color32::from_rgb(120, 120, 130);
    let grid_color = egui::Color32::from_rgb(50, 50, 58);

    for row in 0..=8 {
        let grid_y = plot_rect.top() + plot_rect.height() * (row as f32 / 8.0);
        painter.line_segment(
            [
                egui::pos2(plot_rect.left(), grid_y),
                egui::pos2(plot_rect.right(), grid_y),
            ],
            egui::Stroke::new(1.0_f32, grid_color),
        );
        let val = (1.0 - row as f32 * 0.25) * state.y_range;
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

    // Trigger level line
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
        state.fft_window_start = None;
        if state.captured {
            state.captured_view_offset = 0.0;
        }
    }

    // -- Gesture handling --
    let cursor_over_plot = cursor_pos.map_or(false, |p| plot_rect.contains(p));
    if cursor_over_plot {
        let scroll = scroll_input;
        let zoom = zoom_input;
        let cmd = cmd_held;
        let has_zoom = (zoom - 1.0).abs() > 0.001;

        let x_frac = cursor_pos
            .map(|c| ((c.x - plot_rect.left()) / plot_rect.width()).clamp(0.0, 1.0))
            .unwrap_or(0.5);

        if cmd && has_zoom {
            let old_ms = state.timebase_ms;
            state.timebase_ms = (state.timebase_ms * zoom).clamp(1.0, 500.0);
            if state.captured && state.captured_len > 1 {
                let old_samples = old_ms / 1000.0 * sample_rate;
                let new_samples = state.timebase_ms / 1000.0 * sample_rate;
                let anchor = state.captured_view_offset + x_frac * old_samples;
                state.captured_view_offset = (anchor - x_frac * new_samples).round();
                let max_offset = (state.captured_len as f32 - new_samples).max(0.0);
                state.captured_view_offset =
                    state.captured_view_offset.clamp(0.0, max_offset);
            }
        } else if !cmd && has_zoom {
            state.y_range = (state.y_range * zoom).clamp(0.001, 1.0);
        }

        if cmd && scroll.y != 0.0 && !has_zoom {
            let old_ms = state.timebase_ms;
            state.timebase_ms =
                (state.timebase_ms * (1.0 - scroll.y * 0.005)).clamp(1.0, 500.0);
            if state.captured && state.captured_len > 1 {
                let old_samples = old_ms / 1000.0 * sample_rate;
                let new_samples = state.timebase_ms / 1000.0 * sample_rate;
                let anchor = state.captured_view_offset + x_frac * old_samples;
                state.captured_view_offset = (anchor - x_frac * new_samples).round();
                let max_offset = (state.captured_len as f32 - new_samples).max(0.0);
                state.captured_view_offset =
                    state.captured_view_offset.clamp(0.0, max_offset);
            }
        } else if !cmd && scroll.y != 0.0 && !has_zoom {
            state.y_range =
                (state.y_range * (1.0 - scroll.y * 0.005)).clamp(0.001, 1.0);
        }

        if cmd && state.captured {
            let visible_samples = state.timebase_ms / 1000.0 * sample_rate;
            let cursor_sample = state.captured_view_offset + x_frac * visible_samples;
            let half = fft_size as f32 * 0.5;
            let max_start = (state.captured_len as f32 - fft_size as f32).max(0.0);
            state.fft_window_start =
                Some((cursor_sample - half).clamp(0.0, max_start));
        }

        if state.captured && scroll.x != 0.0 {
            let visible_samples = state.timebase_ms / 1000.0 * sample_rate;
            let samples_per_px = visible_samples / plot_rect.width();
            let shift = scroll.x * samples_per_px * 2.0;
            state.captured_view_offset =
                (state.captured_view_offset - shift).round();
            if state.captured_len > 1 {
                let max_offset =
                    (state.captured_len as f32 - visible_samples).max(0.0);
                state.captured_view_offset =
                    state.captured_view_offset.clamp(0.0, max_offset);
            }
        }
    }

    if state.captured && response.dragged_by(PointerButton::Primary) {
        let delta = response.drag_delta();
        let visible_samples = state.timebase_ms / 1000.0 * sample_rate;
        let samples_per_px = visible_samples / plot_rect.width();
        state.captured_view_offset =
            (state.captured_view_offset + delta.x * samples_per_px).round();
        if state.captured_len > 1 {
            let max_offset =
                (state.captured_len as f32 - visible_samples).max(0.0);
            state.captured_view_offset =
                state.captured_view_offset.clamp(0.0, max_offset);
        }
    }

    // -- Acquire data --
    let (input_l, input_r, output_l, output_r, len, trig_idx_opt) = if state.captured {
        let il = &state.captured_input_l;
        let ir = &state.captured_input_r;
        let ol = &state.captured_output_l;
        let or = &state.captured_output_r;
        let clen = state.captured_len;
        let view_offset = state.captured_view_offset as usize;
        (
            il.as_slice(),
            ir.as_slice(),
            ol.as_slice(),
            or.as_slice(),
            clen,
            Some(view_offset as f32),
        )
    } else {
        let il = state.input_buffer_l.as_slice();
        let ir = state.input_buffer_r.as_slice();
        let ol = state.output_buffer_l.as_slice();
        let or = state.output_buffer_r.as_slice();
        let elen = state.buf_len;

        if elen <= 1 {
            return;
        }

        let trigger_buf_l = match state.source {
            SignalSource::Input => il,
            SignalSource::Output => ol,
            SignalSource::InputAndOutput => ol,
        };
        let trigger_buf_r = match state.source {
            SignalSource::Input => ir,
            SignalSource::Output => or,
            SignalSource::InputAndOutput => or,
        };
        let trigger_buf_l_final = match state.display_mode {
            OscilloscopeDisplayMode::Left | OscilloscopeDisplayMode::Stereo => {
                trigger_buf_l
            }
            OscilloscopeDisplayMode::Right => trigger_buf_r,
        };
        let trigger_buf_r_for_combined = match state.display_mode {
            OscilloscopeDisplayMode::Left | OscilloscopeDisplayMode::Stereo => {
                let trigger_r = match state.source {
                    SignalSource::Input => ir,
                    SignalSource::Output => or,
                    SignalSource::InputAndOutput => or,
                };
                trigger_r
            }
            OscilloscopeDisplayMode::Right => trigger_buf_r,
        };

        let trig = match state.source {
            SignalSource::InputAndOutput => find_combined_trigger(
                trigger_buf_l_final,
                trigger_buf_r_for_combined,
                elen,
                state.trigger_level,
                state.trigger_slope,
            ),
            _ => find_trigger(
                trigger_buf_l_final,
                elen,
                state.trigger_level,
                state.trigger_slope,
            ),
        };

        let trig_f32 = match trig {
            None => {
                let samples_to_show =
                    ((state.timebase_ms / 1000.0 * sample_rate) as usize).max(2);
                elen.saturating_sub(samples_to_show) as f32
            }
            Some(t) => t,
        };

        let samples_to_show =
            ((state.timebase_ms / 1000.0 * sample_rate) as usize).max(2);
        let mut trig_idx = trig_f32 as usize;
        if trig_idx + samples_to_show > elen {
            trig_idx = elen.saturating_sub(samples_to_show);
        }

        (il, ir, ol, or, elen, Some(trig_idx as f32))
    };

    if len <= 1 {
        return;
    }

    let trig_f32 = trig_idx_opt.unwrap_or(0.0);

    let samples_to_show =
        ((state.timebase_ms / 1000.0 * sample_rate) as usize).max(2);

    let start = trig_f32 as usize;
    let end = (start + samples_to_show).min(len);

    if state.captured {
        if let Some(w) = state.fft_window_start {
            let win_end = (w + fft_size as f32).min(state.captured_len as f32);
            let x0 = plot_rect.left()
                + plot_rect.width() * (w - trig_f32) / samples_to_show as f32;
            let x1 = plot_rect.left()
                + plot_rect.width() * (win_end - trig_f32) / samples_to_show as f32;
            let color = egui::Color32::from_rgba_premultiplied(255, 255, 255, 50);
            for x in [x0, x1] {
                if x >= plot_rect.left() && x <= plot_rect.right() {
                    painter.line_segment(
                        [egui::pos2(x, plot_rect.top()), egui::pos2(x, plot_rect.bottom())],
                        egui::Stroke::new(1.0_f32, color),
                    );
                }
            }
        }
        let disc_color =
            egui::Color32::from_rgba_premultiplied(220, 40, 40, 120);
        draw_discontinuities(
            &painter, plot_rect, center_y, output_l, start, end, trig_f32,
            samples_to_show, state.jump_threshold, disc_color,
        );
        draw_discontinuities(
            &painter, plot_rect, center_y, output_r, start, end, trig_f32,
            samples_to_show, state.jump_threshold, disc_color,
        );
        draw_discontinuities(
            &painter, plot_rect, center_y, input_l, start, end, trig_f32,
            samples_to_show, state.jump_threshold, disc_color,
        );
        draw_discontinuities(
            &painter, plot_rect, center_y, input_r, start, end, trig_f32,
            samples_to_show, state.jump_threshold, disc_color,
        );
    }

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

    // -- Cursor crosshair --
    if let Some(cursor) = cursor_pos {
        if plot_rect.contains(cursor) {
            painter.line_segment(
                [
                    egui::pos2(cursor.x, plot_rect.top()),
                    egui::pos2(cursor.x, plot_rect.bottom()),
                ],
                egui::Stroke::new(1.0_f32, grid_color),
            );
            painter.line_segment(
                [
                    egui::pos2(plot_rect.left(), cursor.y),
                    egui::pos2(plot_rect.right(), cursor.y),
                ],
                egui::Stroke::new(1.0_f32, grid_color),
            );
        }
    }

    // X-axis time labels
    let visible_ms = state.timebase_ms;
    let offset_ms = if state.captured {
        state.captured_view_offset / sample_rate * 1000.0
    } else {
        0.0
    };
    let tick_interval = nice_tick_interval(visible_ms, 5.0);
    let first_tick = (offset_ms / tick_interval).ceil() * tick_interval;
    let mut tick_ms = first_tick;
    while tick_ms <= offset_ms + visible_ms {
        let frac = (tick_ms - offset_ms) / visible_ms;
        let tick_x = plot_rect.left() + plot_rect.width() * frac;
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
