use std::f32::consts::TAU;

use eframe::egui;
use rustfft::{FftPlanner, num_complex::Complex32};
use synth_core::LadderFilter;
use wide::f32x4;

use super::spectrum::{self, SpectrumConfig};

const LABEL_W: f32 = 80.0;

pub struct FilterDesignState {
    pub cutoff: f32,
    pub resonance: f32,
    pub poles: u8,
    pub key_track: f32,
    pub env_amount: f32,
    pub audio_mod: f32,

    pub note: f32,
    pub filter_env: f32,
    pub osc1_audio: f32,

    pub response_db: Vec<f32>,
    pub fft_size: usize,
    pub window_type: usize,
    pub db_top: f32,
    pub db_floor: f32,
    pub log_scale: bool,
    pub sample_rate: f32,

    pub live_mode: bool,
    pub needs_render: bool,
    pub last_params_hash: u64,
    pub live_frame: u32,
}

impl Default for FilterDesignState {
    fn default() -> Self {
        Self {
            cutoff: 1.0,
            resonance: 0.0,
            poles: 4,
            key_track: 0.0,
            env_amount: 0.0,
            audio_mod: 0.0,
            note: 60.0,
            filter_env: 0.0,
            osc1_audio: 0.0,
            response_db: Vec::new(),
            fft_size: 4096,
            window_type: 0,
            db_top: 12.0,
            db_floor: -96.0,
            log_scale: true,
            sample_rate: 44100.0,
            live_mode: true,
            needs_render: true,
            last_params_hash: 0,
            live_frame: 0,
        }
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut FilterDesignState) {
    let old_hash = param_hash(state);

    // ---- Filter parameters ----
    ui.horizontal(|ui| {
        ui.label("Cutoff:");
        ui.add(egui::Slider::new(&mut state.cutoff, 0.0..=1.0).text(""));
        ui.label(format!("{:.0}Hz", cutoff_hz(state.cutoff)));
        ui.separator();
        ui.label("Resonance:");
        ui.add(egui::Slider::new(&mut state.resonance, 0.0..=1.0).text(""));
    });

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
        ui.separator();
        ui.label("Key Track:");
        ui.add(egui::Slider::new(&mut state.key_track, 0.0..=1.0).text(""));
        ui.separator();
        ui.label("Env Amt:");
        ui.add(egui::Slider::new(&mut state.env_amount, -1.0..=1.0).text(""));
        ui.separator();
        ui.label("Audio Mod:");
        ui.add(egui::Slider::new(&mut state.audio_mod, 0.0..=1.0).text(""));
    });

    // ---- Test signals ----
    ui.horizontal(|ui| {
        ui.add_sized([LABEL_W, 0.0], egui::Label::new("Note").selectable(false));
        ui.add(egui::Slider::new(&mut state.note, 0.0..=127.0).text(""));
        ui.label(format!(
            "{:.0} ({})",
            state.note,
            note_name(state.note as u8)
        ));
        ui.separator();
        ui.add_sized(
            [LABEL_W, 0.0],
            egui::Label::new("Env level").selectable(false),
        );
        ui.add(egui::Slider::new(&mut state.filter_env, 0.0..=1.0).text(""));
        ui.separator();
        ui.add_sized(
            [LABEL_W, 0.0],
            egui::Label::new("Mod Sig").selectable(false),
        );
        ui.add(egui::Slider::new(&mut state.osc1_audio, -1.0..=1.0).text(""));
    });

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

    // ---- Render scheduling ----
    let new_hash = param_hash(state);
    if new_hash != old_hash {
        state.last_params_hash = new_hash;
        if state.live_mode {
            state.needs_render = true;
        }
    }

    if state.response_db.is_empty() {
        state.needs_render = true;
    }

    if state.live_mode && state.needs_render {
        state.live_frame += 1;
        if state.live_frame % 6 == 0 {
            compute_response(state);
            state.needs_render = false;
        }
    } else if state.needs_render && !state.live_mode {
        compute_response(state);
        state.needs_render = false;
    }

    if state.live_mode {
        state.needs_render = new_hash != old_hash || state.response_db.is_empty();
    }

    ui.add_space(8.0);
    ui.separator();

    // ---- Graph controls ----
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
        if ui.selectable_label(state.log_scale, "Log").clicked() {
            state.log_scale = !state.log_scale;
        }
    });

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
        spectrum::render_spectrum(ui, &state.response_db, &config, 0.0);
    }
}

fn cutoff_hz(value: f32) -> f32 {
    20.0 * (1000.0f32).powf(value)
}

fn note_name(note: u8) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", names[(note % 12) as usize], (note as i32 / 12) - 1)
}

fn param_hash(state: &FilterDesignState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state.cutoff.to_bits().hash(&mut hasher);
    state.resonance.to_bits().hash(&mut hasher);
    state.poles.hash(&mut hasher);
    state.key_track.to_bits().hash(&mut hasher);
    state.env_amount.to_bits().hash(&mut hasher);
    state.audio_mod.to_bits().hash(&mut hasher);
    state.note.to_bits().hash(&mut hasher);
    state.filter_env.to_bits().hash(&mut hasher);
    state.osc1_audio.to_bits().hash(&mut hasher);
    state.sample_rate.to_bits().hash(&mut hasher);
    state.fft_size.hash(&mut hasher);
    state.window_type.hash(&mut hasher);
    hasher.finish()
}

fn compute_response(state: &mut FilterDesignState) {
    let fft_size = state.fft_size;
    let sr = state.sample_rate;

    let mut filter = LadderFilter::default();
    filter.set_cutoff(cutoff_hz(state.cutoff));
    filter.set_resonance(state.resonance);
    filter.set_poles(state.poles);
    filter.set_key_track(state.key_track);
    filter.set_env_amount(state.env_amount);
    filter.set_audio_mod(state.audio_mod);
    filter.reset();

    let note = f32x4::splat(state.note);
    let env = f32x4::splat(state.filter_env);
    let audio = f32x4::splat(state.osc1_audio);

    let mut impulse = vec![0.0f32; fft_size];
    for sample_index in 0..fft_size {
        let input = if sample_index == 0 {
            f32x4::splat(1.0)
        } else {
            f32x4::splat(0.0)
        };
        let output = filter.process(
            input,
            note,
            env,
            f32x4::splat(1.0),
            audio,
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            sr,
        );
        impulse[sample_index] = output.to_array()[0];
    }

    let windowed: Vec<f32> = impulse
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
    let mut db: Vec<f32> = (0..fft_size / 2)
        .map(|bin| {
            let re = complex[bin].re;
            let im = complex[bin].im;
            let mag = (re * re + im * im).sqrt() * scale;
            20.0 * (mag.max(1e-10)).log10()
        })
        .collect();

    let peak = db.iter().cloned().fold(-200.0f32, f32::max);
    if peak > -100.0 {
        for value in &mut db {
            *value -= peak;
        }
    }

    state.response_db = db;
}
