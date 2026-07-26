use eframe::egui;
use rustfft::{Fft, FftPlanner, num_complex::Complex32};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::Arc;

use crate::engine::AudioBlock;
use crate::ui::analysis::spectrum::{self, SpectrumConfig};

use super::oscilloscope::{self, OscilloscopeState, OscilloscopeViewConfig};

const INPUT_LEFT_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 150, 45);
const OUTPUT_LEFT_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 205, 255);

// ---------------------------------------------------------------------------
// Shared enums
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// FFT State
// ---------------------------------------------------------------------------

pub(crate) struct FftState {
    pub(crate) input_buffer: [f32; 4096],
    pub(crate) output_buffer: [f32; 4096],
    pub(crate) input_latest_db: [f32; 2048],
    pub(crate) output_latest_db: [f32; 2048],
    pub(crate) input_peak_hold: [f32; 2048],
    pub(crate) output_peak_hold: [f32; 2048],
    pub(crate) peak_decay: f32,
    pub(crate) frame_count: u32,
    pub fft_size: usize,
    pub(crate) complex_buf: Vec<Complex32>,
    pub(crate) fft: Option<Arc<dyn Fft<f32>>>,
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

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub(crate) struct FftViewConfig {
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

// ---------------------------------------------------------------------------
// RealTime root state
// ---------------------------------------------------------------------------

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

#[derive(Serialize, Deserialize)]
pub struct RealTimeViewConfig {
    pub oscilloscope: OscilloscopeViewConfig,
    pub fft: FftViewConfig,
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

// ---------------------------------------------------------------------------
// Shared helpers (used by oscilloscope module)
// ---------------------------------------------------------------------------

pub(crate) fn copy_channel(
    dest: &mut [f32],
    left: &[f32],
    right: &[f32],
    channel: SpectrumChannel,
) {
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

pub(crate) fn fill_fft_from_captured(
    fft_state: &mut FftState,
    captured_in_l: &[f32],
    captured_in_r: &[f32],
    captured_out_l: &[f32],
    captured_out_r: &[f32],
    captured_len: usize,
    view_offset: f32,
    timebase_ms: f32,
    sample_rate: f32,
    fft_window_start: Option<f32>,
) {
    let fft_size = fft_state.fft_size;
    let end = if let Some(w) = fft_window_start {
        let start = (w as usize).min(captured_len.saturating_sub(fft_size));
        (start + fft_size).min(captured_len)
    } else {
        let visible = ((timebase_ms / 1000.0 * sample_rate) as usize).max(2);
        let offset = view_offset as usize;
        (offset + visible).min(captured_len)
    };
    if end < fft_size {
        return;
    }
    let start = end.saturating_sub(fft_size);
    let slice_len = (end - start).min(fft_size);
    fft_state.output_buffer.fill(0.0);
    fft_state.input_buffer.fill(0.0);
    let shift = fft_size - slice_len;
    copy_channel(
        &mut fft_state.output_buffer[shift..fft_size],
        &captured_out_l[start..end],
        &captured_out_r[start..end],
        fft_state.channel,
    );
    copy_channel(
        &mut fft_state.input_buffer[shift..fft_size],
        &captured_in_l[start..end],
        &captured_in_r[start..end],
        fft_state.channel,
    );
}

pub(crate) fn fill_fft_from_live(
    fft_state: &mut FftState,
    live_in_l: &[f32],
    live_in_r: &[f32],
    live_out_l: &[f32],
    live_out_r: &[f32],
    live_len: usize,
) {
    let fft_size = fft_state.fft_size;
    if live_len < fft_size {
        return;
    }
    let start = live_len.saturating_sub(fft_size);
    let end = live_len;
    let slice_len = end - start;
    fft_state.output_buffer.fill(0.0);
    fft_state.input_buffer.fill(0.0);
    let shift = fft_size - slice_len;
    copy_channel(
        &mut fft_state.output_buffer[shift..fft_size],
        &live_out_l[start..end],
        &live_out_r[start..end],
        fft_state.channel,
    );
    copy_channel(
        &mut fft_state.input_buffer[shift..fft_size],
        &live_in_l[start..end],
        &live_in_r[start..end],
        fft_state.channel,
    );
}

pub(crate) fn process_fft_trace(
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

// ---------------------------------------------------------------------------
// Main show
// ---------------------------------------------------------------------------

pub fn show(ui: &mut egui::Ui, audio_blocks: VecDeque<AudioBlock>, state: &mut RealTimeState) {
    oscilloscope::feed_audio(
        &mut state.osc,
        &mut state.fft,
        audio_blocks,
        state.sample_rate,
    );

    let available = ui.available_size();
    let gap = 12.0;
    let osc_h = (available.y * 0.4).max(120.0);
    let fft_h = (available.y - osc_h - gap).max(120.0);

    ui.allocate_ui(egui::vec2(available.x, osc_h), |ui| {
        ui.strong("Oscilloscope");
        ui.add_space(6.0);
        oscilloscope::draw_oscilloscope(ui, &mut state.osc, state.sample_rate, state.fft.fft_size);
    });
    ui.add_space(gap);
    ui.allocate_ui(egui::vec2(available.x, fft_h), |ui| {
        ui.strong("Spectrum Analyzer");
        ui.add_space(6.0);
        draw_fft(
            ui,
            &mut state.fft,
            &state.osc,
            state.sample_rate,
        );
    });
}

// ---------------------------------------------------------------------------
// FFT drawing
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

fn draw_fft(
    ui: &mut egui::Ui,
    state: &mut FftState,
    osc: &OscilloscopeState,
    sample_rate: f32,
) {
    let fft_size = state.fft_size;
    if osc.captured {
        if state.complex_buf.len() != fft_size || state.fft.is_none() {
            state.complex_buf = vec![Complex32::new(0.0, 0.0); fft_size];
            let mut planner = FftPlanner::new();
            state.fft = Some(planner.plan_fft_forward(fft_size));
        }
        fill_fft_from_captured(
            state,
            &osc.captured_input_l,
            &osc.captured_input_r,
            &osc.captured_output_l,
            &osc.captured_output_r,
            osc.captured_len,
            osc.captured_view_offset,
            osc.timebase_ms,
            sample_rate,
            osc.fft_window_start,
        );
        let fft = state.fft.as_ref().cloned();
        if let Some(fft) = fft {
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
    } else if state.frame_count % 4 == 0 {
        if state.complex_buf.len() != fft_size || state.fft.is_none() {
            state.complex_buf = vec![Complex32::new(0.0, 0.0); fft_size];
            let mut planner = FftPlanner::new();
            state.fft = Some(planner.plan_fft_forward(fft_size));
        }
        fill_fft_from_live(
            state,
            &osc.input_buffer_l,
            &osc.input_buffer_r,
            &osc.output_buffer_l,
            &osc.output_buffer_r,
            osc.buf_len,
        );
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
            if ui
                .selectable_label(state.channel == chan, label)
                .clicked()
                && state.channel != chan
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
            spectrum::render_spectra(
                ui,
                &[output_trace, input_trace],
                &config,
                HOVER_READOUT_H,
            )
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
    use super::{SignalSource, SpectrumChannel, copy_channel};
    use super::super::oscilloscope::{TriggerSlope, find_combined_trigger};

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

        let trigger = find_combined_trigger(
            &input,
            &output,
            input.len(),
            0.0,
            TriggerSlope::Rising,
        );

        assert!(trigger.is_some());
        assert!((trigger.unwrap() - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn falling_edge_trigger() {
        let buf = [0.3, 0.2, -0.1, -0.2];
        let trigger = super::super::oscilloscope::find_trigger(
            &buf,
            buf.len(),
            0.0,
            TriggerSlope::Falling,
        );
        assert!(trigger.is_some());
        assert!((trigger.unwrap() - (1.0 + 2.0 / 3.0)).abs() < 0.001);
    }
}
