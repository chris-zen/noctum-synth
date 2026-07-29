use eframe::egui;
use eframe::egui::PointerButton;
use eframe::egui::epaint::PathShape;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::engine::{AudioBlock, MAX_AUDIO_BUF};
use crate::ui::analysis::real_time::{HoverStatus, SignalSource};
use crate::ui::analysis::spectrum_analyzer::{FftState, fill_fft_from_captured, process_fft_trace};

pub(crate) const MAX_SCOPE_SAMPLES: usize = 65536;
const DEFAULT_CLICK_SENSITIVITY: f32 = 0.5;
const PRE_TRIGGER_FRAC: f32 = 0.2;
const VIEW_PRE_TRIGGER_SAMPLES: f32 = 8.0;
const CLICK_MAD_EPS: f32 = 1e-4;
const CLICK_MAD_SCALE: f32 = 1.4826;
const CLICK_MIN_PERIOD: usize = 8;
const CLICK_MAX_PERIOD: usize = 512;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerMode {
    Free,
    Auto,
    Normal,
    Single,
}

impl Default for TriggerMode {
    fn default() -> Self {
        Self::Auto
    }
}

impl TriggerMode {
    fn is_triggered(self) -> bool {
        matches!(self, Self::Auto | Self::Normal | Self::Single)
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
    #[serde(default = "default_click_sensitivity_val", alias = "jump_threshold")]
    pub click_sensitivity: f32,
    #[serde(default = "default_capture_duration")]
    pub capture_duration_ms: f32,
}

fn default_capture_duration() -> f32 {
    500.0
}

fn default_click_sensitivity_val() -> f32 {
    DEFAULT_CLICK_SENSITIVITY
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
            click_sensitivity: state.click_sensitivity,
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
        state.click_sensitivity = self.click_sensitivity;
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
    pub(crate) click_sensitivity: f32,
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
    capture_circ_filled: usize,
    capture_post_needed: usize,
    capture_trig_pos: f32,
    acq_wait_samples: usize,
    trigger_prev_sample: Option<f32>,
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
            click_sensitivity: DEFAULT_CLICK_SENSITIVITY,
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
            capture_circ_filled: 0,
            capture_post_needed: 0,
            capture_trig_pos: 0.0,
            acq_wait_samples: 0,
            trigger_prev_sample: None,
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

#[cfg(test)]
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
                    let fraction = (level - buf[index - 1]) / (buf[index] - buf[index - 1]);
                    return Some((index - 1) as f32 + fraction);
                }
            }
        }
        TriggerSlope::Falling => {
            for index in (1..len).rev() {
                if buf[index - 1] > level && buf[index] <= level {
                    let fraction = (buf[index - 1] - level) / (buf[index - 1] - buf[index]);
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

#[cfg(test)]
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

pub(crate) fn find_trigger_offset_with_prev(
    prev: Option<f32>,
    buf: &[f32],
    level: f32,
    slope: TriggerSlope,
) -> Option<usize> {
    if buf.is_empty() {
        return None;
    }
    let crosses = |a: f32, b: f32| -> bool {
        match slope {
            TriggerSlope::Rising => a < level && b >= level,
            TriggerSlope::Falling => a > level && b <= level,
            TriggerSlope::Both => (a < level && b >= level) || (a > level && b <= level),
        }
    };
    if let Some(p) = prev {
        if crosses(p, buf[0]) {
            return Some(0);
        }
    }
    for i in 1..buf.len() {
        if crosses(buf[i - 1], buf[i]) {
            return Some(i);
        }
    }
    None
}

fn pre_trigger_samples(tgt: usize) -> usize {
    ((tgt as f32 * PRE_TRIGGER_FRAC) as usize).min(tgt.saturating_sub(1))
}

fn auto_timeout_samples(osc: &OscilloscopeState, sample_rate: f32) -> usize {
    let ms = (2.0 * osc.capture_duration_ms).max(100.0);
    ((ms / 1000.0) * sample_rate) as usize
}

fn trigger_channel_from_block<'a>(
    block: &'a AudioBlock,
    block_len: usize,
    source: SignalSource,
    display_mode: OscilloscopeDisplayMode,
) -> &'a [f32] {
    let use_left = display_mode != OscilloscopeDisplayMode::Right;
    match source {
        SignalSource::Input => {
            if use_left {
                &block.input_left[..block_len]
            } else {
                &block.input_right[..block_len]
            }
        }
        SignalSource::Output | SignalSource::InputAndOutput => {
            if use_left {
                &block.output_left[..block_len]
            } else {
                &block.output_right[..block_len]
            }
        }
    }
}

fn write_circ_samples(osc: &mut OscilloscopeState, block: &AudioBlock, write_len: usize) {
    if write_len == 0 {
        return;
    }
    let tgt = osc.capture_circ_target;
    let mut write = osc.capture_circ_write;
    let first = write_len.min(tgt - write);
    osc.capture_circ_il[write..write + first].copy_from_slice(&block.input_left[..first]);
    osc.capture_circ_ir[write..write + first].copy_from_slice(&block.input_right[..first]);
    osc.capture_circ_ol[write..write + first].copy_from_slice(&block.output_left[..first]);
    osc.capture_circ_or[write..write + first].copy_from_slice(&block.output_right[..first]);
    write = (write + first) % tgt;
    if write_len > first {
        let second = write_len - first;
        osc.capture_circ_il[0..second].copy_from_slice(&block.input_left[first..write_len]);
        osc.capture_circ_ir[0..second].copy_from_slice(&block.input_right[first..write_len]);
        osc.capture_circ_ol[0..second].copy_from_slice(&block.output_left[first..write_len]);
        osc.capture_circ_or[0..second].copy_from_slice(&block.output_right[first..write_len]);
        write = second % tgt;
    }
    osc.capture_circ_write = write;
    osc.capture_circ_filled = (osc.capture_circ_filled + write_len).min(tgt);
}

fn unwrap_capture_to_display(state: &mut OscilloscopeState) {
    let start = state.capture_circ_start;
    let n = state.capture_circ_target;
    if n == 0 || state.capture_circ_il.len() != n {
        return;
    }
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
}

fn rearm_trigger(osc: &mut OscilloscopeState) {
    osc.capture_trigger_found = false;
    osc.capture_circ_count = 0;
    osc.capture_post_needed = 0;
    osc.acq_wait_samples = 0;
    osc.trigger_prev_sample = None;
}

fn finalize_acquisition(
    osc: &mut OscilloscopeState,
    fft: &mut FftState,
    sample_rate: f32,
    stop: bool,
) {
    let trig = osc.capture_trig_pos;
    unwrap_capture_to_display(osc);
    osc.captured_view_offset = (trig - VIEW_PRE_TRIGGER_SAMPLES).max(0.0);
    compute_fft_on_capture(fft, osc, sample_rate);
    if stop {
        osc.captured = true;
        osc.capture_armed = false;
        osc.capture_trigger_found = false;
    } else {
        rearm_trigger(osc);
    }
}

fn force_auto_sweep(osc: &mut OscilloscopeState) {
    let tgt = osc.capture_circ_target;
    if tgt == 0 || osc.capture_circ_filled < tgt {
        return;
    }
    osc.capture_circ_start = osc.capture_circ_write % tgt;
    osc.capture_trig_pos = 0.0;
}

fn arm_acquisition(osc: &mut OscilloscopeState, sample_rate: f32) {
    let tgt = ((osc.capture_duration_ms / 1000.0 * sample_rate) as usize)
        .max(64)
        .min(MAX_SCOPE_SAMPLES);
    if osc.capture_circ_target != tgt || osc.capture_circ_il.len() != tgt {
        osc.capture_circ_il = vec![0.0f32; tgt];
        osc.capture_circ_ir = vec![0.0f32; tgt];
        osc.capture_circ_ol = vec![0.0f32; tgt];
        osc.capture_circ_or = vec![0.0f32; tgt];
        osc.capture_circ_target = tgt;
        osc.capture_circ_write = 0;
        osc.capture_circ_start = 0;
        osc.capture_circ_filled = 0;
    }
    osc.capture_armed = true;
    rearm_trigger(osc);
}

fn start_run(osc: &mut OscilloscopeState, sample_rate: f32) {
    osc.captured = false;
    osc.fft_window_start = None;
    if osc.trigger_mode.is_triggered() {
        arm_acquisition(osc, sample_rate);
    } else {
        osc.capture_armed = false;
        osc.capture_trigger_found = false;
        osc.capture_circ_target = 0;
        osc.trigger_prev_sample = None;
    }
}

fn freeze_from_live(osc: &mut OscilloscopeState, fft: &mut FftState, sample_rate: f32) {
    if osc.captured_len > 1 && osc.trigger_mode.is_triggered() {
        osc.captured = true;
        osc.capture_armed = false;
        osc.capture_trigger_found = false;
        compute_fft_on_capture(fft, osc, sample_rate);
        return;
    }
    let n = osc.buf_len;
    if n <= 1 {
        return;
    }
    let samples_to_show = ((osc.timebase_ms / 1000.0 * sample_rate) as usize).max(2);
    let view_offset = n.saturating_sub(samples_to_show) as f32;
    osc.captured_input_l = osc.input_buffer_l[..n].to_vec();
    osc.captured_input_r = osc.input_buffer_r[..n].to_vec();
    osc.captured_output_l = osc.output_buffer_l[..n].to_vec();
    osc.captured_output_r = osc.output_buffer_r[..n].to_vec();
    osc.captured_len = n;
    osc.captured_view_offset = view_offset;
    osc.capture_armed = false;
    osc.capture_trigger_found = false;
    osc.captured = true;
    compute_fft_on_capture(fft, osc, sample_rate);
}

fn stop_acquisition(osc: &mut OscilloscopeState, fft: &mut FftState, sample_rate: f32) {
    if osc.captured {
        return;
    }
    freeze_from_live(osc, fft, sample_rate);
}

fn mark_trigger(osc: &mut OscilloscopeState, trig_circ_idx: usize) {
    let tgt = osc.capture_circ_target;
    let pre = pre_trigger_samples(tgt);
    osc.capture_trigger_found = true;
    osc.capture_circ_start = (trig_circ_idx + tgt - pre) % tgt;
    osc.capture_trig_pos = pre as f32;
    osc.capture_post_needed = tgt.saturating_sub(pre + 1);
    osc.capture_circ_count = 0;
}

fn process_triggered_block(
    osc: &mut OscilloscopeState,
    fft: &mut FftState,
    block: &AudioBlock,
    block_len: usize,
    sample_rate: f32,
) -> bool {
    let tgt = osc.capture_circ_target;
    if tgt == 0 || !osc.capture_armed {
        return false;
    }

    let trig_chan = trigger_channel_from_block(block, block_len, osc.source, osc.display_mode);
    let write_before = osc.capture_circ_write;
    let mut completed = false;

    if !osc.capture_trigger_found {
        osc.acq_wait_samples = osc.acq_wait_samples.saturating_add(block_len);
        if let Some(off) = find_trigger_offset_with_prev(
            osc.trigger_prev_sample,
            trig_chan,
            osc.trigger_level,
            osc.trigger_slope,
        ) {
            let trig_circ_idx = (write_before + off) % tgt;
            let pre = pre_trigger_samples(tgt);
            let post_needed = tgt.saturating_sub(pre + 1);
            let samples_after = block_len.saturating_sub(off + 1);
            let write_after = samples_after.min(post_needed);
            let write_len = off + 1 + write_after;
            write_circ_samples(osc, block, write_len);
            mark_trigger(osc, trig_circ_idx);
            osc.capture_circ_count = write_after;
        } else if osc.trigger_mode == TriggerMode::Auto
            && osc.capture_circ_filled >= tgt
            && osc.acq_wait_samples >= auto_timeout_samples(osc, sample_rate)
        {
            write_circ_samples(osc, block, block_len);
            force_auto_sweep(osc);
            finalize_acquisition(osc, fft, sample_rate, false);
            completed = true;
        } else {
            write_circ_samples(osc, block, block_len);
        }
    } else {
        let need = osc
            .capture_post_needed
            .saturating_sub(osc.capture_circ_count);
        let write_len = block_len.min(need);
        write_circ_samples(osc, block, write_len);
        osc.capture_circ_count += write_len;
    }

    if let Some(last) = trig_chan.last().copied() {
        osc.trigger_prev_sample = Some(last);
    }

    if osc.capture_trigger_found && osc.capture_circ_count >= osc.capture_post_needed && !completed
    {
        let stop = osc.trigger_mode == TriggerMode::Single;
        finalize_acquisition(osc, fft, sample_rate, stop);
        completed = true;
    }

    completed
}

fn compute_fft_on_capture(fft_state: &mut FftState, osc: &OscilloscopeState, sample_rate: f32) {
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

/// Feeds audio blocks into live buffers and triggered acquisition.
/// Returns true if an acquisition just completed.
pub(crate) fn feed_audio(
    osc: &mut OscilloscopeState,
    fft: &mut FftState,
    audio_blocks: VecDeque<AudioBlock>,
    sample_rate: f32,
) -> bool {
    let mut completed = false;
    if osc.captured {
        return false;
    }

    if osc.trigger_mode.is_triggered() && !osc.capture_armed {
        arm_acquisition(osc, sample_rate);
    } else if osc.trigger_mode == TriggerMode::Free && osc.capture_armed {
        osc.capture_armed = false;
        osc.capture_trigger_found = false;
        osc.capture_circ_target = 0;
        osc.trigger_prev_sample = None;
    }

    if osc.trigger_mode.is_triggered() && osc.capture_armed {
        let tgt = ((osc.capture_duration_ms / 1000.0 * sample_rate) as usize)
            .max(64)
            .min(MAX_SCOPE_SAMPLES);
        if tgt != osc.capture_circ_target {
            arm_acquisition(osc, sample_rate);
        }
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

        if osc.trigger_mode.is_triggered() {
            if process_triggered_block(osc, fft, &block, block_len, sample_rate) {
                completed = true;
            }
        }

        fft.frame_count += 1;
    }

    completed
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
// Click / discontinuity detection
// ---------------------------------------------------------------------------

fn second_diff_residual(x0: f32, x1: f32, x2: f32) -> f32 {
    (x2 - 2.0 * x1 + x0).abs()
}

fn median_sorted(sorted: &mut [f32]) -> f32 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        0.5 * (sorted[n / 2 - 1] + sorted[n / 2])
    }
}

fn mad(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = median_sorted(&mut sorted);
    for v in &mut sorted {
        *v = (*v - med).abs();
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    median_sorted(&mut sorted)
}

fn click_sensitivity_to_z(sensitivity: f32) -> f32 {
    let s = sensitivity.clamp(0.0, 1.0);
    5.0 + (1.0 - s) * 10.0
}

fn click_abs_floor_frac(sensitivity: f32) -> f32 {
    let s = sensitivity.clamp(0.0, 1.0);
    0.04 + (1.0 - s) * 0.08
}

fn percentile_range(values: &[f32], lo_pct: usize, hi_pct: usize) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let lo = sorted[(n * lo_pct / 100).min(n - 1)];
    let hi = sorted[(n * hi_pct / 100).min(n - 1)];
    (hi - lo).max(1e-6)
}

fn mean_first_diff(buffer: &[f32], start: usize, end: usize) -> f32 {
    let end = end.min(buffer.len().saturating_sub(1));
    if end <= start {
        return 0.0;
    }
    let mut sum = 0.0_f32;
    let mut count = 0_u32;
    for i in start..end {
        sum += buffer[i + 1] - buffer[i];
        count += 1;
    }
    if count == 0 { 0.0 } else { sum / count as f32 }
}

fn is_waveform_wrap(buffer: &[f32], n: usize) -> bool {
    const PRE: usize = 4;
    const POST: usize = 4;
    const SETTLE: usize = 2;
    if n < PRE + 2 || n + POST + SETTLE + 1 >= buffer.len() {
        return false;
    }
    let mut best_j = n;
    let mut best_jump = 0.0_f32;
    for j in (n.saturating_sub(2))..=n {
        if j == 0 || j >= buffer.len() {
            continue;
        }
        let jump = buffer[j] - buffer[j - 1];
        if jump.abs() > best_jump.abs() {
            best_j = j;
            best_jump = jump;
        }
    }
    if best_jump.abs() < 1e-4 {
        return false;
    }
    let j = best_j;
    if j < PRE + 1 || j + SETTLE + POST >= buffer.len() {
        return false;
    }
    let slope_before = mean_first_diff(buffer, j - 1 - PRE, j - 1);
    let slope_after = mean_first_diff(buffer, j + SETTLE, j + SETTLE + POST);
    if slope_before.abs() < 1e-5 && slope_after.abs() < 1e-5 {
        return false;
    }
    if !(slope_before * slope_after > 0.0 && slope_before * best_jump < 0.0) {
        return false;
    }
    let step = slope_before.abs().max(slope_after.abs()).max(1e-6);
    if best_jump.abs() <= 4.0 * step {
        return false;
    }
    let slope_lo = slope_before.abs().min(slope_after.abs()).max(1e-6);
    let slope_hi = slope_before.abs().max(slope_after.abs());
    slope_hi <= 6.0 * slope_lo
}

fn similar_residual(a: f32, b: f32) -> bool {
    if a < 1e-8 || b < 1e-8 {
        return false;
    }
    let ratio = if a > b { a / b } else { b / a };
    ratio <= 2.0
}

fn is_periodic_residual(residuals: &[f32], index: usize, residual: f32) -> bool {
    let n = residuals.len();
    let max_period = CLICK_MAX_PERIOD.min(n / 2).max(CLICK_MIN_PERIOD + 1);
    for period in CLICK_MIN_PERIOD..max_period {
        let mut matches = 0_u32;
        let candidates = [
            index.checked_sub(period),
            Some(index + period),
            index.checked_sub(period.saturating_mul(2)),
            Some(index + period.saturating_mul(2)),
        ];
        for j in candidates.into_iter().flatten() {
            if j >= 2 && j < n && similar_residual(residuals[j], residual) {
                matches += 1;
            }
        }
        if matches >= 2 {
            return true;
        }
    }
    false
}

fn find_click_indices(buffer: &[f32], start: usize, end: usize, sensitivity: f32) -> Vec<usize> {
    if sensitivity <= 0.0 {
        return Vec::new();
    }
    let end = end.min(buffer.len());
    if end <= start + 6 {
        return Vec::new();
    }
    let z_thresh = click_sensitivity_to_z(sensitivity);
    let amp = percentile_range(&buffer[start..end], 5, 95);
    let abs_floor = click_abs_floor_frac(sensitivity) * amp;

    let mut residuals = vec![0.0_f32; end];
    for n in (start + 2)..end {
        residuals[n] = second_diff_residual(buffer[n - 2], buffer[n - 1], buffer[n]);
    }
    let scale = CLICK_MAD_EPS + CLICK_MAD_SCALE * mad(&residuals[start + 2..end]);

    let mut flags = Vec::new();
    let lo = (start + 4).max(4);
    let hi = end.saturating_sub(2);
    for n in lo..hi {
        let r = residuals[n];
        if r < abs_floor || r / scale < z_thresh {
            continue;
        }
        if r < residuals[n - 1] || r <= residuals[n + 1] {
            continue;
        }
        if is_waveform_wrap(buffer, n) {
            continue;
        }
        if is_periodic_residual(&residuals, n, r) {
            continue;
        }
        flags.push(n);
    }
    flags
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
    sensitivity: f32,
    color: egui::Color32,
) {
    let stroke = egui::Stroke::new(1.0_f32, color);
    for idx in find_click_indices(buffer, start, end, sensitivity) {
        let x =
            plot_rect.left() + plot_rect.width() * (idx as f32 - trig_f32) / samples_to_show as f32;
        if x >= plot_rect.left() && x <= plot_rect.right() {
            painter.add(PathShape::line(
                vec![
                    egui::pos2(x, plot_rect.top()),
                    egui::pos2(x, plot_rect.bottom()),
                ],
                stroke,
            ));
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
                + plot_rect.width() * (sample_index as f32 - trig_f32) / samples_to_show as f32;
            let point_y = center_y - buffer[sample_index] * display_yscale;
            egui::pos2(point_x, point_y.clamp(plot_rect.top(), plot_rect.bottom()))
        })
        .collect();

    if pts.len() >= 2 {
        painter.add(PathShape::line(pts, egui::Stroke::new(1.2_f32, color)));
    }
}

// ---------------------------------------------------------------------------
// Main render
// ---------------------------------------------------------------------------

pub(crate) fn draw_oscilloscope(
    ui: &mut egui::Ui,
    state: &mut OscilloscopeState,
    fft: &mut FftState,
    sample_rate: f32,
    hover: &mut Option<HoverStatus>,
) {
    let fft_size = fft.fft_size;
    let scroll_input = ui.ctx().input(|i| i.smooth_scroll_delta);
    let zoom_input = ui.ctx().input(|i| i.zoom_delta());
    let cmd_held = ui.ctx().input(|i| i.modifiers.command);
    let alt_held = ui.ctx().input(|i| i.modifiers.alt);
    let cursor_pos = ui.ctx().input(|i| i.pointer.hover_pos());

    // -- Bar 1: Run / Mode / Trigger --
    ui.horizontal_wrapped(|ui| {
        let key_toggle = ui.input(|i| i.key_pressed(egui::Key::Space));
        if state.captured {
            if ui
                .button("▶ Run")
                .on_hover_text("Resume acquisition (Space)")
                .clicked()
                || key_toggle
            {
                start_run(state, sample_rate);
            }
        } else if ui
            .button("⏹ Stop")
            .on_hover_text("Stop acquisition and hold the current view (Space)")
            .clicked()
            || key_toggle
        {
            stop_acquisition(state, fft, sample_rate);
        }

        ui.separator();
        ui.label("Mode:");
        let prev_mode = state.trigger_mode;
        for (mode, label, tip) in [
            (
                TriggerMode::Free,
                "Free",
                "Free-run: waveform always scrolls",
            ),
            (
                TriggerMode::Auto,
                "Auto",
                "Triggered sweeps; free-run if no trigger within timeout",
            ),
            (
                TriggerMode::Normal,
                "Normal",
                "Triggered sweeps only; hold last acquisition while waiting",
            ),
            (
                TriggerMode::Single,
                "Single",
                "One triggered acquisition, then stop",
            ),
        ] {
            if ui
                .selectable_label(state.trigger_mode == mode, label)
                .on_hover_text(tip)
                .clicked()
            {
                state.trigger_mode = mode;
            }
        }
        if state.trigger_mode != prev_mode && !state.captured {
            start_run(state, sample_rate);
        }

        ui.separator();
        ui.label("Level:");
        ui.add(
            egui::Slider::new(&mut state.trigger_level, -1.0..=1.0)
                .text("")
                .trailing_fill(true),
        )
        .on_hover_text("Trigger threshold");
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
        )
        .on_hover_text("Acquisition buffer length");
        if state.captured {
            ui.separator();
            ui.label("Click:");
            ui.add(egui::Slider::new(&mut state.click_sensitivity, 0.0..=1.0).text(""))
                .on_hover_text("Click sensitivity: 0 = off, higher marks more candidates");
        }
    });

    // -- Bar 2: View parameters --
    ui.horizontal_wrapped(|ui| {
        ui.label("X:");
        let buf_duration_ms = if state.captured || uses_display_hold(state) {
            state.captured_len as f32 / sample_rate * 1000.0
        } else if state.capture_armed {
            state.capture_duration_ms
        } else {
            state.buf_len as f32 / sample_rate * 1000.0
        };

        let clamp_ms = timebase_max_ms(state, sample_rate);
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
            egui::Slider::new(&mut state.y_range, 0.01..=1.0)
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
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(available.x, plot_h), egui::Sense::click());
    let plot_left = rect.left() + y_label_w;
    let plot_rect = egui::Rect::from_min_max(
        egui::pos2(plot_left, rect.top() + top_pad),
        egui::pos2(rect.right(), rect.bottom() - x_label_h),
    );
    let painter = ui.painter_at(rect);

    painter.rect_filled(plot_rect, 0.0, egui::Color32::from_rgb(20, 20, 24));

    let y_scale = plot_rect.height() * 0.48;
    let center_y = plot_rect.center().y;
    let display_yscale = y_scale / state.y_range;
    let font_id = egui::FontId::monospace(8.0);
    let label_color = egui::Color32::from_rgb(120, 120, 130);
    let grid_color = egui::Color32::from_rgb(50, 50, 58);

    for step in -4..=4 {
        let val = step as f32 * 0.25 * state.y_range;
        let grid_y = center_y - val * display_yscale;
        if !plot_rect.y_range().contains(grid_y) {
            continue;
        }
        painter.line_segment(
            [
                egui::pos2(plot_rect.left(), grid_y),
                egui::pos2(plot_rect.right(), grid_y),
            ],
            egui::Stroke::new(1.0_f32, grid_color),
        );
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
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(200, 120, 30)),
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
        if has_inspectable_buffer(state) {
            lock_scope_view(state, fft, sample_rate);
            if state.trigger_mode.is_triggered() {
                state.captured_view_offset =
                    (state.capture_trig_pos - VIEW_PRE_TRIGGER_SAMPLES).max(0.0);
            } else {
                state.captured_view_offset = 0.0;
            }
            let visible = (state.timebase_ms / 1000.0 * sample_rate).max(2.0);
            let max_offset = (state.captured_len as f32 - visible).max(0.0);
            state.captured_view_offset = state.captured_view_offset.clamp(0.0, max_offset);
        }
    }

    // -- Gesture handling --
    let cursor_over_plot = cursor_pos.map_or(false, |p| plot_rect.contains(p));
    if cursor_over_plot {
        let scroll = scroll_input;
        let zoom = zoom_input;
        let cmd = cmd_held;
        let alt = alt_held;
        let has_zoom = (zoom - 1.0).abs() > 0.001;
        let inspectable = has_inspectable_buffer(state);

        let x_frac = cursor_pos
            .map(|c| ((c.x - plot_rect.left()) / plot_rect.width()).clamp(0.0, 1.0))
            .unwrap_or(0.5);

        if cmd && has_zoom {
            let old_ms = state.timebase_ms;
            let max_ms = timebase_max_ms(state, sample_rate);
            state.timebase_ms = (state.timebase_ms * zoom).clamp(1.0, max_ms);
            if inspectable {
                lock_scope_view(state, fft, sample_rate);
                let old_samples = old_ms / 1000.0 * sample_rate;
                let new_samples = state.timebase_ms / 1000.0 * sample_rate;
                let anchor = state.captured_view_offset + x_frac * old_samples;
                state.captured_view_offset = (anchor - x_frac * new_samples).round();
                let max_offset = (state.captured_len as f32 - new_samples).max(0.0);
                state.captured_view_offset = state.captured_view_offset.clamp(0.0, max_offset);
            }
        }

        if cmd && scroll.y != 0.0 && !has_zoom {
            let old_ms = state.timebase_ms;
            let max_ms = timebase_max_ms(state, sample_rate);
            state.timebase_ms = (state.timebase_ms * (1.0 - scroll.y * 0.005)).clamp(1.0, max_ms);
            if inspectable {
                lock_scope_view(state, fft, sample_rate);
                let old_samples = old_ms / 1000.0 * sample_rate;
                let new_samples = state.timebase_ms / 1000.0 * sample_rate;
                let anchor = state.captured_view_offset + x_frac * old_samples;
                state.captured_view_offset = (anchor - x_frac * new_samples).round();
                let max_offset = (state.captured_len as f32 - new_samples).max(0.0);
                state.captured_view_offset = state.captured_view_offset.clamp(0.0, max_offset);
            }
        }

        if alt && !cmd {
            // Read raw wheel events (not smooth_scroll_delta). With Opt held, egui's
            // default vertical_scroll_modifier remaps axes into smooth_scroll_delta;
            // event deltas keep the platform sign: positive y = finger/content down.
            let mut wheel_y = 0.0_f32;
            ui.input(|input| {
                for event in &input.events {
                    if let egui::Event::MouseWheel {
                        delta, modifiers, ..
                    } = event
                    {
                        if modifiers.alt && !modifiers.command {
                            wheel_y += delta.y;
                        }
                    }
                }
            });
            if wheel_y != 0.0 {
                // Finger/content down (positive y) increases Y range.
                let steps = (wheel_y / 8.0).abs().max(1.0).round();
                let delta = wheel_y.signum() * steps * 0.01;
                state.y_range = quantize_y_range(state.y_range + delta);
            } else if has_zoom {
                // Pinch with Opt: zoom>1 (spread) decreases Y range.
                state.y_range = quantize_y_range(state.y_range / zoom);
            }
        }

        if cmd && inspectable {
            lock_scope_view(state, fft, sample_rate);
            let visible_samples = state.timebase_ms / 1000.0 * sample_rate;
            let cursor_sample = state.captured_view_offset + x_frac * visible_samples;
            let half = fft_size as f32 * 0.5;
            let max_start = (state.captured_len as f32 - fft_size as f32).max(0.0);
            state.fft_window_start = Some((cursor_sample - half).clamp(0.0, max_start));
        }

        if inspectable && scroll.x != 0.0 {
            lock_scope_view(state, fft, sample_rate);
            let visible_samples = state.timebase_ms / 1000.0 * sample_rate;
            let samples_per_px = visible_samples / plot_rect.width();
            let shift = scroll.x * samples_per_px * 2.0;
            state.captured_view_offset = (state.captured_view_offset - shift).round();
            if state.captured_len > 1 {
                let max_offset = (state.captured_len as f32 - visible_samples).max(0.0);
                state.captured_view_offset = state.captured_view_offset.clamp(0.0, max_offset);
            }
        }
    }

    // -- Acquire data --
    let use_hold = uses_display_hold(state);
    let (input_l, input_r, output_l, output_r, len, trig_idx_opt) = if state.captured || use_hold {
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
    } else if state.trigger_mode == TriggerMode::Free {
        let il = state.input_buffer_l.as_slice();
        let ir = state.input_buffer_r.as_slice();
        let ol = state.output_buffer_l.as_slice();
        let or = state.output_buffer_r.as_slice();
        let elen = state.buf_len;
        if elen <= 1 {
            return;
        }
        let samples_to_show = ((state.timebase_ms / 1000.0 * sample_rate) as usize).max(2);
        let start = elen.saturating_sub(samples_to_show) as f32;
        (il, ir, ol, or, elen, Some(start))
    } else {
        return;
    };

    if len <= 1 {
        return;
    }

    let trig_f32 = trig_idx_opt.unwrap_or(0.0);

    let samples_to_show = ((state.timebase_ms / 1000.0 * sample_rate) as usize).max(2);

    let start = trig_f32 as usize;
    let end = (start + samples_to_show).min(len);

    if state.captured {
        if let Some(w) = state.fft_window_start {
            let win_end = (w + fft_size as f32).min(state.captured_len as f32);
            let x0 = plot_rect.left() + plot_rect.width() * (w - trig_f32) / samples_to_show as f32;
            let x1 = plot_rect.left()
                + plot_rect.width() * (win_end - trig_f32) / samples_to_show as f32;
            let color = egui::Color32::from_rgba_premultiplied(255, 255, 255, 50);
            for x in [x0, x1] {
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

    if state.captured && state.click_sensitivity > 0.0 {
        let disc_color = egui::Color32::from_rgba_unmultiplied(220, 40, 40, 90);
        draw_discontinuities(
            &painter,
            plot_rect,
            center_y,
            output_l,
            start,
            end,
            trig_f32,
            samples_to_show,
            state.click_sensitivity,
            disc_color,
        );
        draw_discontinuities(
            &painter,
            plot_rect,
            center_y,
            output_r,
            start,
            end,
            trig_f32,
            samples_to_show,
            state.click_sensitivity,
            disc_color,
        );
        draw_discontinuities(
            &painter,
            plot_rect,
            center_y,
            input_l,
            start,
            end,
            trig_f32,
            samples_to_show,
            state.click_sensitivity,
            disc_color,
        );
        draw_discontinuities(
            &painter,
            plot_rect,
            center_y,
            input_r,
            start,
            end,
            trig_f32,
            samples_to_show,
            state.click_sensitivity,
            disc_color,
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

            let x_frac = ((cursor.x - plot_rect.left()) / plot_rect.width()).clamp(0.0, 1.0);
            let offset_ms = if state.captured || use_hold {
                state.captured_view_offset / sample_rate * 1000.0
            } else {
                0.0
            };
            let time_ms = offset_ms + x_frac * state.timebase_ms;
            let sample_idx = ((start as f32 + x_frac * samples_to_show as f32).round() as usize)
                .clamp(start, end.saturating_sub(1).max(start));
            let levels = format_scope_levels(
                state.source,
                state.display_mode,
                sample_at(input_l, sample_idx),
                sample_at(input_r, sample_idx),
                sample_at(output_l, sample_idx),
                sample_at(output_r, sample_idx),
            );
            *hover = Some(HoverStatus::Scope { time_ms, levels });
        }
    }

    // X-axis time labels
    let visible_ms = state.timebase_ms;
    let offset_ms = if state.captured || use_hold {
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

fn sample_at(buf: &[f32], index: usize) -> f32 {
    buf.get(index).copied().unwrap_or(0.0)
}

fn uses_display_hold(state: &OscilloscopeState) -> bool {
    !state.captured && state.trigger_mode.is_triggered() && state.captured_len > 1
}

fn has_inspectable_buffer(state: &OscilloscopeState) -> bool {
    state.captured_len > 1 && (state.captured || uses_display_hold(state))
}

fn lock_scope_view(osc: &mut OscilloscopeState, fft: &mut FftState, sample_rate: f32) {
    if !osc.captured {
        stop_acquisition(osc, fft, sample_rate);
    }
}

fn timebase_max_ms(state: &OscilloscopeState, sample_rate: f32) -> f32 {
    if state.captured || uses_display_hold(state) {
        (state.captured_len as f32 / sample_rate * 1000.0).max(1.0)
    } else {
        state.capture_duration_ms.max(1.0)
    }
}

fn quantize_y_range(value: f32) -> f32 {
    ((value * 100.0).round() / 100.0).clamp(0.01, 1.0)
}

fn format_scope_levels(
    source: SignalSource,
    mode: OscilloscopeDisplayMode,
    input_l: f32,
    input_r: f32,
    output_l: f32,
    output_r: f32,
) -> String {
    let (left, right) = match source {
        SignalSource::Input => (input_l, input_r),
        SignalSource::Output => (output_l, output_r),
        SignalSource::InputAndOutput => (0.0, 0.0),
    };
    match (source, mode) {
        (SignalSource::InputAndOutput, OscilloscopeDisplayMode::Stereo) => {
            format!(
                "I L: {input_l:+.3}   I R: {input_r:+.3}   O L: {output_l:+.3}   O R: {output_r:+.3}"
            )
        }
        (SignalSource::InputAndOutput, OscilloscopeDisplayMode::Left) => {
            format!("I: {input_l:+.3}   O: {output_l:+.3}")
        }
        (SignalSource::InputAndOutput, OscilloscopeDisplayMode::Right) => {
            format!("I: {input_r:+.3}   O: {output_r:+.3}")
        }
        (_, OscilloscopeDisplayMode::Stereo) => {
            format!("L: {left:+.3}   R: {right:+.3}")
        }
        (_, OscilloscopeDisplayMode::Left) => format!("Level: {left:+.3}"),
        (_, OscilloscopeDisplayMode::Right) => format!("Level: {right:+.3}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        OscilloscopeDisplayMode, OscilloscopeState, TriggerMode, TriggerSlope,
        click_sensitivity_to_z, find_click_indices, find_trigger_offset_with_prev,
        format_scope_levels, mark_trigger, pre_trigger_samples, second_diff_residual,
        unwrap_capture_to_display,
    };
    use crate::ui::analysis::real_time::SignalSource;
    use synth_core::dsp::{AnalogOscillator, Waveform};
    use synth_core::math::WideF32;

    fn flags_near(flags: &[usize], target: usize, tol: usize) -> bool {
        flags.iter().any(|&i| i.abs_diff(target) <= tol)
    }

    fn render_oscillator(waveform: Waveform, freq_hz: f32, n: usize) -> Vec<f32> {
        let sample_rate = 48_000.0;
        let mut osc = AnalogOscillator::new(sample_rate);
        osc.set_waveform(waveform);
        osc.set_frequency(WideF32::splat(freq_hz));
        let mut ctx = synth_core::create_render_context!();
        let mut buf = Vec::with_capacity(n);
        for _ in 0..n {
            buf.push(osc.next(&mut ctx).output.to_array()[0]);
        }
        buf
    }

    fn inject_sample_drop(buf: &mut [f32], index: usize) {
        buf[index] = 0.0;
    }

    fn inject_impulse(buf: &mut [f32], index: usize, value: f32) {
        buf[index] = value;
    }

    fn inject_dc_step(buf: &mut [f32], index: usize, delta: f32) {
        for sample in &mut buf[index..] {
            *sample += delta;
        }
    }

    fn inject_phase_splice(buf: &mut [f32], index: usize, shift: usize) {
        let tail = buf[index..].to_vec();
        let n = tail.len();
        for (i, sample) in buf[index..].iter_mut().enumerate() {
            *sample = tail[(i + shift) % n];
        }
    }

    #[test]
    fn scope_levels_format_by_source_and_mode() {
        assert_eq!(
            format_scope_levels(
                SignalSource::Output,
                OscilloscopeDisplayMode::Left,
                0.1,
                0.2,
                0.3,
                0.4
            ),
            "Level: +0.300"
        );
        assert_eq!(
            format_scope_levels(
                SignalSource::Input,
                OscilloscopeDisplayMode::Stereo,
                0.1,
                -0.2,
                0.3,
                0.4
            ),
            "L: +0.100   R: -0.200"
        );
        assert_eq!(
            format_scope_levels(
                SignalSource::InputAndOutput,
                OscilloscopeDisplayMode::Right,
                0.1,
                0.2,
                0.3,
                0.4
            ),
            "I: +0.200   O: +0.400"
        );
    }

    #[test]
    fn trigger_offset_detects_block_boundary_crossing() {
        let buf = [0.2, 0.4, 0.5];
        let hit = find_trigger_offset_with_prev(Some(-0.1), &buf, 0.0, TriggerSlope::Rising);
        assert_eq!(hit, Some(0));
    }

    #[test]
    fn trigger_offset_finds_rising_edge_inside_block() {
        let buf = [-0.2, -0.1, 0.1, 0.2];
        let hit = find_trigger_offset_with_prev(None, &buf, 0.0, TriggerSlope::Rising);
        assert_eq!(hit, Some(2));
    }

    #[test]
    fn pre_trigger_places_trigger_near_twenty_percent() {
        let mut osc = OscilloscopeState::default();
        let tgt = 100;
        osc.capture_circ_il = (0..tgt).map(|i| i as f32).collect();
        osc.capture_circ_ir = vec![0.0; tgt];
        osc.capture_circ_ol = vec![0.0; tgt];
        osc.capture_circ_or = vec![0.0; tgt];
        osc.capture_circ_target = tgt;
        let trig_idx = 50;
        mark_trigger(&mut osc, trig_idx);
        let pre = pre_trigger_samples(tgt);
        assert_eq!(osc.capture_trig_pos, pre as f32);
        assert_eq!(osc.capture_circ_start, (trig_idx + tgt - pre) % tgt);
        unwrap_capture_to_display(&mut osc);
        assert_eq!(osc.captured_len, tgt);
        assert!((osc.captured_input_l[pre] - trig_idx as f32).abs() < f32::EPSILON);
    }

    #[test]
    fn trigger_mode_default_is_auto() {
        assert_eq!(TriggerMode::default(), TriggerMode::Auto);
        assert!(TriggerMode::Auto.is_triggered());
        assert!(TriggerMode::Normal.is_triggered());
        assert!(TriggerMode::Single.is_triggered());
        assert!(!TriggerMode::Free.is_triggered());
    }

    #[test]
    fn second_diff_is_zero_on_linear_ramp() {
        assert!((second_diff_residual(0.0, 0.1, 0.2)).abs() < 1e-6);
    }

    #[test]
    fn smooth_sine_has_no_click_flags_at_default_sensitivity() {
        let n = 2048;
        let buf: Vec<f32> = (0..n)
            .map(|i| (i as f32 * std::f32::consts::TAU / 64.0).sin())
            .collect();
        let flags = find_click_indices(&buf, 0, n, 0.5);
        assert!(
            flags.is_empty(),
            "expected no flags on smooth sine, got {flags:?}"
        );
    }

    #[test]
    fn hard_step_is_flagged_at_default_sensitivity() {
        let mut buf = vec![0.0_f32; 512];
        for sample in buf.iter_mut().take(512).skip(256) {
            *sample = 0.8;
        }
        let flags = find_click_indices(&buf, 0, buf.len(), 0.5);
        assert!(
            flags_near(&flags, 256, 3),
            "expected a flag near the step, got {flags:?}"
        );
    }

    #[test]
    fn consistent_bright_saw_has_no_click_flags() {
        let period = 32;
        let cycles = 16;
        let n = period * cycles;
        let mut buf = Vec::with_capacity(n);
        for _ in 0..cycles {
            for i in 0..period {
                let t = i as f32 / period as f32;
                buf.push(2.0 * t - 1.0);
            }
        }
        let flags = find_click_indices(&buf, 0, n, 0.5);
        assert!(
            flags.is_empty(),
            "saw wraps should not be marked as clicks; got {flags:?}"
        );
    }

    #[test]
    fn click_sensitivity_zero_disables_detection() {
        let mut buf = vec![0.0_f32; 512];
        for sample in buf.iter_mut().take(512).skip(256) {
            *sample = 0.8;
        }
        assert!(find_click_indices(&buf, 0, buf.len(), 0.0).is_empty());
    }

    #[test]
    fn click_sensitivity_maps_higher_to_lower_z() {
        assert!(click_sensitivity_to_z(1.0) < click_sensitivity_to_z(0.0));
        assert!((click_sensitivity_to_z(1.0) - 5.0).abs() < 1e-5);
        assert!((click_sensitivity_to_z(0.0) - 15.0).abs() < 1e-5);
    }

    #[test]
    fn clean_analog_saw_has_no_click_flags() {
        let buf = render_oscillator(Waveform::Saw, 220.0, 4096);
        let flags = find_click_indices(&buf, 0, buf.len(), 0.5);
        assert!(
            flags.is_empty(),
            "clean AnalogOscillator saw should not flag; got {flags:?}"
        );
    }

    #[test]
    fn clean_analog_pulse_has_no_click_flags() {
        let buf = render_oscillator(Waveform::Pulse, 220.0, 4096);
        let flags = find_click_indices(&buf, 0, buf.len(), 0.5);
        assert!(
            flags.is_empty(),
            "clean AnalogOscillator pulse should not flag; got {flags:?}"
        );
    }

    #[test]
    fn clean_analog_triangle_has_no_click_flags() {
        let buf = render_oscillator(Waveform::Triangle, 220.0, 4096);
        let flags = find_click_indices(&buf, 0, buf.len(), 0.5);
        assert!(
            flags.is_empty(),
            "clean AnalogOscillator triangle should not flag; got {flags:?}"
        );
    }

    #[test]
    fn sample_drop_click_on_analog_saw_is_detected() {
        let mut buf = render_oscillator(Waveform::Saw, 220.0, 4096);
        let click_at = 2000;
        inject_sample_drop(&mut buf, click_at);
        let flags = find_click_indices(&buf, 0, buf.len(), 0.5);
        assert!(
            flags_near(&flags, click_at, 3),
            "expected drop click near {click_at}, got {flags:?}"
        );
    }

    #[test]
    fn impulse_click_on_analog_saw_is_detected() {
        let mut buf = render_oscillator(Waveform::Saw, 220.0, 4096);
        let click_at = 2000;
        inject_impulse(&mut buf, click_at, 1.0);
        let flags = find_click_indices(&buf, 0, buf.len(), 0.5);
        assert!(
            flags_near(&flags, click_at, 3),
            "expected impulse click near {click_at}, got {flags:?}"
        );
    }

    #[test]
    fn dc_step_click_on_analog_saw_is_detected() {
        let mut buf = render_oscillator(Waveform::Saw, 220.0, 4096);
        let click_at = 2000;
        inject_dc_step(&mut buf, click_at, 0.5);
        let flags = find_click_indices(&buf, 0, buf.len(), 0.5);
        assert!(
            flags_near(&flags, click_at, 3),
            "expected DC-step click near {click_at}, got {flags:?}"
        );
    }

    #[test]
    fn phase_splice_click_on_analog_saw_is_detected() {
        let mut buf = render_oscillator(Waveform::Saw, 220.0, 4096);
        let click_at = 2000;
        let period_samples = (48_000.0 / 220.0) as usize;
        inject_phase_splice(&mut buf, click_at, period_samples / 2);
        let flags = find_click_indices(&buf, 0, buf.len(), 0.5);
        assert!(
            flags_near(&flags, click_at, 3),
            "expected phase-splice click near {click_at}, got {flags:?}"
        );
    }

    #[test]
    fn sample_drop_click_on_sine_is_detected() {
        let n = 4096;
        let mut buf: Vec<f32> = (0..n)
            .map(|i| (i as f32 * std::f32::consts::TAU / 64.0).sin())
            .collect();
        let click_at = 2000;
        inject_sample_drop(&mut buf, click_at);
        let flags = find_click_indices(&buf, 0, n, 0.5);
        assert!(
            flags_near(&flags, click_at, 3),
            "expected sine drop click near {click_at}, got {flags:?}"
        );
    }

    #[test]
    fn two_real_clicks_are_both_detected() {
        let n = 4096;
        let mut buf: Vec<f32> = (0..n)
            .map(|i| (i as f32 * std::f32::consts::TAU / 64.0).sin())
            .collect();
        inject_sample_drop(&mut buf, 1000);
        inject_sample_drop(&mut buf, 3000);
        let flags = find_click_indices(&buf, 0, n, 0.5);
        assert!(
            flags_near(&flags, 1000, 3),
            "missing first click; got {flags:?}"
        );
        assert!(
            flags_near(&flags, 3000, 3),
            "missing second click; got {flags:?}"
        );
    }
}
