use std::path::PathBuf;

use eframe::egui;

use crate::engine::{AudioMetrics, SynthEngineControl};
use crate::ui::widgets::{
    KNOB_SIZE, master_volume, param_knob_bipolar, param_knob_f32, param_knob_log_hz,
    param_toggle,
};
use synth_core::{
    DEFAULT_ATTACK_SECONDS, DEFAULT_DECAY_SECONDS, DEFAULT_RELEASE_SECONDS, DEFAULT_SUSTAIN_LEVEL,
    LfoDestination, MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ, OscillatorPatch, ParamId, Patch,
};

const WIDE_LAYOUT_MIN_WIDTH: f32 = 900.0;
const OSC_GRID_WIDTH: f32 = 700.0;
const LFO_PANEL_WIDTH: f32 = 560.0;
const FILTER_GRID_WIDTH: f32 = 540.0;
const AMP_GRID_WIDTH: f32 = 320.0;
const AUX_GRID_WIDTH: f32 = 380.0;
const CONTROL_CELL_W: f32 = 46.0;
const CONTROL_CELL_H: f32 = 64.0;
const DEST_CELL_W: f32 = 112.0;
const TOGGLE_CELL_W: f32 = 82.0;
const TOGGLE_CELL_H: f32 = 64.0;
const WAVE_CELL_W: f32 = 112.0;

#[derive(Clone)]
pub struct UiState {
    pub osc1_enabled: bool,
    pub osc2_enabled: bool,
    pub osc1_waveform: usize,
    pub osc2_waveform: usize,
    pub osc1_freq: f32,
    pub osc2_freq: f32,
    pub osc1_fine: f32,
    pub osc2_fine: f32,
    pub osc1_shape: f32,
    pub osc2_shape: f32,
    pub osc_mix: f32,
    pub sync: bool,
    pub osc_slop: f32,
    pub osc1_note_reset: bool,
    pub osc2_note_reset: bool,
    pub sub_level: f32,
    pub noise_level: f32,
    pub filter_cutoff: f32,
    pub filter_resonance: f32,
    pub filter_poles: usize,
    pub filter_key_track: f32,
    pub filter_env_amount: f32,
    pub filter_velocity: f32,
    pub filter_audio_mod: f32,
    pub filter_delay: f32,
    pub filter_attack: f32,
    pub filter_decay: f32,
    pub filter_sustain: f32,
    pub filter_release: f32,
    pub amp_pan_spread: f32,
    pub amp_env_amount: f32,
    pub amp_velocity: f32,
    pub amp_delay: f32,
    pub amp_attack: f32,
    pub amp_decay: f32,
    pub amp_sustain: f32,
    pub amp_release: f32,
    pub aux_destination: usize,
    pub aux_env_amount: f32,
    pub aux_velocity: f32,
    pub aux_delay: f32,
    pub aux_attack: f32,
    pub aux_decay: f32,
    pub aux_sustain: f32,
    pub aux_release: f32,
    pub aux_repeat: bool,
    pub selected_lfo: usize,
    pub lfo_rates: [f32; 4],
    pub lfo_depths: [f32; 4],
    pub lfo_waveforms: [usize; 4],
    pub lfo_destinations: [usize; 4],
    pub lfo_clock_sync: [bool; 4],
    pub lfo_key_sync: [bool; 4],
    pub master_volume: f32,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            osc1_enabled: true,
            osc2_enabled: false,
            osc1_waveform: 0,
            osc2_waveform: 0,
            osc1_freq: 60.0,
            osc2_freq: 60.0,
            osc1_fine: 0.0,
            osc2_fine: 0.0,
            osc1_shape: 0.0,
            osc2_shape: 0.0,
            osc_mix: 0.0,
            sync: false,
            osc_slop: 0.0,
            osc1_note_reset: true,
            osc2_note_reset: true,
            sub_level: 0.0,
            noise_level: 0.0,
            filter_cutoff: 20_000.0,
            filter_resonance: 0.0,
            filter_poles: 1,
            filter_key_track: 0.0,
            filter_env_amount: 0.0,
            filter_velocity: 0.0,
            filter_audio_mod: 0.0,
            filter_delay: 0.0,
            filter_attack: DEFAULT_ATTACK_SECONDS,
            filter_decay: DEFAULT_DECAY_SECONDS,
            filter_sustain: DEFAULT_SUSTAIN_LEVEL,
            filter_release: DEFAULT_RELEASE_SECONDS,
            amp_pan_spread: 0.0,
            amp_env_amount: 1.0,
            amp_velocity: 1.0,
            amp_delay: 0.0,
            amp_attack: DEFAULT_ATTACK_SECONDS,
            amp_decay: DEFAULT_DECAY_SECONDS,
            amp_sustain: DEFAULT_SUSTAIN_LEVEL,
            amp_release: DEFAULT_RELEASE_SECONDS,
            aux_destination: 0,
            aux_env_amount: 0.0,
            aux_velocity: 0.0,
            aux_delay: 0.0,
            aux_attack: DEFAULT_ATTACK_SECONDS,
            aux_decay: DEFAULT_DECAY_SECONDS,
            aux_sustain: DEFAULT_SUSTAIN_LEVEL,
            aux_release: DEFAULT_RELEASE_SECONDS,
            aux_repeat: false,
            selected_lfo: 0,
            lfo_rates: [1.0; 4],
            lfo_depths: [0.0; 4],
            lfo_waveforms: [0; 4],
            lfo_destinations: [0; 4],
            lfo_clock_sync: [false; 4],
            lfo_key_sync: [true; 4],
            master_volume: 1.0,
        }
    }
}

impl UiState {
    pub fn apply_from_patch(&mut self, patch: &Patch) {
        self.osc1_enabled = patch.osc1.enabled;
        self.osc2_enabled = patch.osc2.enabled;
        self.osc1_waveform = patch.osc1.waveform as usize;
        self.osc2_waveform = patch.osc2.waveform as usize;
        self.osc1_freq = patch.osc1.frequency;
        self.osc2_freq = patch.osc2.frequency;
        self.osc1_fine = patch.osc1.fine_tune;
        self.osc2_fine = patch.osc2.fine_tune;
        self.osc1_shape = patch.osc1.shape;
        self.osc2_shape = patch.osc2.shape;
        self.osc_mix = patch.osc_mix;
        self.sync = patch.hard_sync;
        self.osc_slop = patch.osc_slop;
        self.osc1_note_reset = patch.osc1.note_reset;
        self.osc2_note_reset = patch.osc2.note_reset;
        self.sub_level = patch.sub_osc_level;
        self.noise_level = patch.noise_level;
        self.filter_cutoff = patch.filter.cutoff;
        self.filter_resonance = patch.filter.resonance;
        self.filter_poles = if patch.filter.poles <= 2 { 0 } else { 1 };
        self.filter_key_track = patch.filter.key_track;
        self.filter_env_amount = patch.filter.env_amount;
        self.filter_velocity = patch.filter.velocity;
        self.filter_audio_mod = patch.filter.audio_mod;
        self.filter_delay = patch.filter.eg_delay;
        self.filter_attack = patch.filter.eg_attack;
        self.filter_decay = patch.filter.eg_decay;
        self.filter_sustain = patch.filter.eg_sustain;
        self.filter_release = patch.filter.eg_release;
        self.amp_pan_spread = patch.amplifier.pan_spread;
        self.amp_env_amount = patch.amplifier.env_amount;
        self.amp_velocity = patch.amplifier.velocity;
        self.amp_delay = patch.amplifier.eg_delay;
        self.amp_attack = patch.amplifier.eg_attack;
        self.amp_decay = patch.amplifier.eg_decay;
        self.amp_sustain = patch.amplifier.eg_sustain;
        self.amp_release = patch.amplifier.eg_release;
        self.aux_destination = patch.aux_envelope.destination.index();
        self.aux_env_amount = patch.aux_envelope.amount;
        self.aux_velocity = patch.aux_envelope.velocity;
        self.aux_delay = patch.aux_envelope.delay;
        self.aux_attack = patch.aux_envelope.attack;
        self.aux_decay = patch.aux_envelope.decay;
        self.aux_sustain = patch.aux_envelope.sustain;
        self.aux_release = patch.aux_envelope.release;
        self.aux_repeat = patch.aux_envelope.repeat;
        for i in 0..4 {
            let lfo = &patch.lfos[i];
            self.lfo_rates[i] = lfo.rate_hz;
            self.lfo_depths[i] = lfo.depth;
            self.lfo_waveforms[i] = lfo_waveform_usize(lfo.waveform);
            self.lfo_destinations[i] = lfo.destination.index();
            self.lfo_clock_sync[i] = lfo.clock_sync;
            self.lfo_key_sync[i] = lfo.key_sync;
        }
    }
}

fn lfo_waveform_usize(w: synth_core::LfoWaveform) -> usize {
    match w {
        synth_core::LfoWaveform::Triangle => 0,
        synth_core::LfoWaveform::Saw => 1,
        synth_core::LfoWaveform::ReverseSaw => 2,
        synth_core::LfoWaveform::Square => 3,
        synth_core::LfoWaveform::SampleAndHold => 4,
    }
}

impl From<&UiState> for Patch {
    fn from(state: &UiState) -> Self {
        use synth_core::LfoWaveform;
        let lfo_wf = |idx: usize| -> LfoWaveform {
            match state.lfo_waveforms[idx] {
                0 => LfoWaveform::Triangle,
                1 => LfoWaveform::Saw,
                2 => LfoWaveform::ReverseSaw,
                3 => LfoWaveform::Square,
                _ => LfoWaveform::SampleAndHold,
            }
        };
        Patch {
            osc1: OscillatorPatch {
                waveform: state.osc1_waveform as u8,
                enabled: state.osc1_enabled,
                frequency: state.osc1_freq,
                fine_tune: state.osc1_fine,
                shape: state.osc1_shape,
                level: 1.0,
                note_reset: state.osc1_note_reset,
                keyboard_on: true,
                glide: false,
            },
            osc2: OscillatorPatch {
                waveform: state.osc2_waveform as u8,
                enabled: state.osc2_enabled,
                frequency: state.osc2_freq,
                fine_tune: state.osc2_fine,
                shape: state.osc2_shape,
                level: 1.0,
                note_reset: state.osc2_note_reset,
                keyboard_on: true,
                glide: false,
            },
            osc_mix: state.osc_mix,
            sub_osc_level: state.sub_level,
            noise_level: state.noise_level,
            hard_sync: state.sync,
            osc_slop: state.osc_slop,
            glide_time: 0.0,
            filter: synth_core::FilterParams {
                cutoff: state.filter_cutoff,
                resonance: state.filter_resonance,
                poles: if state.filter_poles == 0 { 2 } else { 4 },
                key_track: state.filter_key_track,
                env_amount: state.filter_env_amount,
                velocity: state.filter_velocity,
                audio_mod: state.filter_audio_mod,
                eg_delay: state.filter_delay,
                eg_attack: state.filter_attack,
                eg_decay: state.filter_decay,
                eg_sustain: state.filter_sustain,
                eg_release: state.filter_release,
            },
            amplifier: synth_core::AmplifierParams {
                pan_spread: state.amp_pan_spread,
                env_amount: state.amp_env_amount,
                velocity: state.amp_velocity,
                eg_delay: state.amp_delay,
                eg_attack: state.amp_attack,
                eg_decay: state.amp_decay,
                eg_sustain: state.amp_sustain,
                eg_release: state.amp_release,
            },
            aux_envelope: synth_core::AuxEnvelopeParams {
                destination: LfoDestination::from_index(state.aux_destination),
                amount: state.aux_env_amount,
                velocity: state.aux_velocity,
                delay: state.aux_delay,
                attack: state.aux_attack,
                decay: state.aux_decay,
                sustain: state.aux_sustain,
                release: state.aux_release,
                repeat: state.aux_repeat,
            },
            lfos: [
                synth_core::LfoParams {
                    rate_hz: state.lfo_rates[0],
                    depth: state.lfo_depths[0],
                    waveform: lfo_wf(0),
                    destination: LfoDestination::from_index(state.lfo_destinations[0]),
                    clock_sync: state.lfo_clock_sync[0],
                    key_sync: state.lfo_key_sync[0],
                },
                synth_core::LfoParams {
                    rate_hz: state.lfo_rates[1],
                    depth: state.lfo_depths[1],
                    waveform: lfo_wf(1),
                    destination: LfoDestination::from_index(state.lfo_destinations[1]),
                    clock_sync: state.lfo_clock_sync[1],
                    key_sync: state.lfo_key_sync[1],
                },
                synth_core::LfoParams {
                    rate_hz: state.lfo_rates[2],
                    depth: state.lfo_depths[2],
                    waveform: lfo_wf(2),
                    destination: LfoDestination::from_index(state.lfo_destinations[2]),
                    clock_sync: state.lfo_clock_sync[2],
                    key_sync: state.lfo_key_sync[2],
                },
                synth_core::LfoParams {
                    rate_hz: state.lfo_rates[3],
                    depth: state.lfo_depths[3],
                    waveform: lfo_wf(3),
                    destination: LfoDestination::from_index(state.lfo_destinations[3]),
                    clock_sync: state.lfo_clock_sync[3],
                    key_sync: state.lfo_key_sync[3],
                },
            ],
            master_volume: 1.0,
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut UiState,
    control: &SynthEngineControl,
    voice_active: usize,
    voice_total: usize,
    metrics: Option<AudioMetrics>,
    analysis_open: &mut bool,
    patch_mgr: &mut PatchManager,
) {
    command_row(
        ui,
        control,
        voice_active,
        voice_total,
        metrics,
        analysis_open,
        state,
        patch_mgr,
    );

    ui.add_space(6.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        module_panel(ui, "Oscillators", |ui| {
            oscillators_module(ui, state, control);
        });

        ui.add_space(8.0);

        module_panel(ui, "Low Frequency Oscillators", |ui| {
            lfo_module(ui, state, control);
        });

        ui.add_space(8.0);

        if ui.available_width() >= WIDE_LAYOUT_MIN_WIDTH {
            ui.columns(2, |columns| {
                module_panel(&mut columns[0], "Low-Pass Filter", |ui| {
                    filter_module(ui, state, control);
                });
                module_panel(&mut columns[1], "Amplifier", |ui| {
                    amplifier_module(ui, state, control);
                });
            });
            ui.add_space(8.0);
            module_panel(ui, "Auxiliary Envelope", |ui| {
                auxiliary_envelope_module(ui, state, control);
            });
        } else {
            module_panel(ui, "Low-Pass Filter", |ui| {
                filter_module(ui, state, control);
            });
            ui.add_space(8.0);
            module_panel(ui, "Amplifier", |ui| {
                amplifier_module(ui, state, control);
            });
            ui.add_space(8.0);
            module_panel(ui, "Auxiliary Envelope", |ui| {
                auxiliary_envelope_module(ui, state, control);
            });
        }
    });

    ui.add_space(4.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(format!("Voices: {voice_active}/{voice_total}"));
        if let Some(metrics) = metrics {
            ui.separator();
            ui.label(metrics_text(&metrics));
        }
    });
}

fn command_row(
    ui: &mut egui::Ui,
    control: &SynthEngineControl,
    _voice_active: usize,
    _voice_total: usize,
    _metrics: Option<AudioMetrics>,
    analysis_open: &mut bool,
    state: &mut UiState,
    patch_mgr: &mut PatchManager,
) {
    ui.horizontal(|ui| {
        if ui.button("Play C4").clicked() {
            control.note_on(60, 0.8);
        }
        if ui.button("Play A3").clicked() {
            control.note_on(57, 0.8);
        }
        if ui.button("Stop all").clicked() {
            control.all_notes_off();
        }
        ui.separator();
        if ui.button("Analysis").clicked() {
            *analysis_open = !*analysis_open;
        }
        let input_enabled = control.input_enabled();
        if ui
            .selectable_label(input_enabled, "Audio In")
            .on_hover_text("Toggle mixing the audio input into the output")
            .clicked()
        {
            control.set_input_enabled(!input_enabled);
        }

        ui.separator();

        ui.label("Patch:");
        ui.add_sized(
            [160.0, 20.0],
            egui::TextEdit::singleline(&mut patch_mgr.save_name),
        );
        let load_clicked = egui::ComboBox::from_id_salt("patch_load")
            .selected_text("Load")
            .width(56.0)
            .show_ui(ui, |ui| {
                patch_mgr.refresh();
                if patch_mgr.patch_names.is_empty() {
                    ui.label("No saved patches yet.");
                } else {
                    for name in &patch_mgr.patch_names {
                        if ui.button(name.as_str()).clicked() {
                            ui.close();
                            if let Some(patch) = patch_mgr.load_patch(name) {
                                control.load_patch(&patch);
                                state.apply_from_patch(&patch);
                                patch_mgr.save_name.clone_from(name);
                            }
                        }
                    }
                }
            });
        if ui.button("Save").clicked() {
            let name = patch_mgr.save_name.trim().to_string();
            if !name.is_empty() {
                patch_mgr.save_patch(&name, &Patch::from(&*state));
                patch_mgr.refresh();
            }
        }
        let _ = load_clicked;

        ui.separator();
        master_volume(ui, &mut state.master_volume, control);
    });
}

fn metrics_text(metrics: &AudioMetrics) -> String {
    format!(
        "cb {:.2}/{:.2}ms  render {:.2}/{:.2}ms  deadline {:.2}ms  over {}/{} of {}",
        metrics.callback_avg_ms,
        metrics.callback_max_ms,
        metrics.render_avg_ms,
        metrics.render_max_ms,
        metrics.deadline_ms,
        metrics.overruns,
        metrics.render_overruns,
        metrics.callbacks,
    )
}

fn module_panel(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let horizontal_margin = 10;
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin {
            left: horizontal_margin,
            right: horizontal_margin,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(title).strong()));
            ui.add_space(4.0);
            ui.add(
                egui::Separator::default()
                    .horizontal()
                    .grow(horizontal_margin as f32),
            );
            ui.add_space(12.0);
            add_contents(ui);
        });
}

fn oscillators_module(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    fixed_panel_scroll(ui, "oscillators_grid_scroll", OSC_GRID_WIDTH, |ui| {
        egui::Grid::new("oscillators_grid")
            .num_columns(9)
            .spacing(egui::vec2(14.0, 10.0))
            .show(ui, |ui| {
                strong_label(ui, "OSC 1");
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Freq",
                        &mut state.osc1_freq,
                        0.0..=120.0,
                        60.0,
                        ParamId::Osc1Frequency,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Fine",
                        &mut state.osc1_fine,
                        -50.0..=50.0,
                        0.0,
                        ParamId::Osc1FineTune,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Shape",
                        &mut state.osc1_shape,
                        0.0..=1.0,
                        0.0,
                        ParamId::Osc1Shape,
                        control,
                    );
                });
                ui.allocate_ui_with_layout(
                    egui::vec2(32.0, CONTROL_CELL_H),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.add_space(18.0);
                        param_toggle(
                            ui,
                            "On",
                            &mut state.osc1_enabled,
                            ParamId::Osc1Enabled,
                            control,
                        );
                    },
                );
                wave_selector_cell(
                    ui,
                    &mut state.osc1_waveform,
                    &mut state.osc1_enabled,
                    ParamId::Osc1Waveform,
                    ParamId::Osc1Enabled,
                    control,
                );
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Sub",
                        &mut state.sub_level,
                        0.0..=1.0,
                        0.0,
                        ParamId::SubOscLevel,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Noise",
                        &mut state.noise_level,
                        0.0..=1.0,
                        0.0,
                        ParamId::NoiseLevel,
                        control,
                    );
                });
                ui.allocate_ui_with_layout(
                    egui::vec2(TOGGLE_CELL_W, TOGGLE_CELL_H),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            param_toggle(
                                ui,
                                "Osc1 Reset",
                                &mut state.osc1_note_reset,
                                ParamId::Osc1NoteReset,
                                control,
                            );
                            ui.add_space(3.0);
                            param_toggle(
                                ui,
                                "Osc2 Reset",
                                &mut state.osc2_note_reset,
                                ParamId::Osc2NoteReset,
                                control,
                            );
                            ui.add_space(3.0);
                            param_toggle(ui, "Sync", &mut state.sync, ParamId::HardSync, control);
                        });
                    },
                );
                ui.end_row();

                strong_label(ui, "OSC 2");
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Freq",
                        &mut state.osc2_freq,
                        0.0..=120.0,
                        60.0,
                        ParamId::Osc2Frequency,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Fine",
                        &mut state.osc2_fine,
                        -50.0..=50.0,
                        0.0,
                        ParamId::Osc2FineTune,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Shape",
                        &mut state.osc2_shape,
                        0.0..=1.0,
                        0.0,
                        ParamId::Osc2Shape,
                        control,
                    );
                });
                ui.allocate_ui_with_layout(
                    egui::vec2(32.0, CONTROL_CELL_H),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.add_space(18.0);
                        param_toggle(
                            ui,
                            "On",
                            &mut state.osc2_enabled,
                            ParamId::Osc2Enabled,
                            control,
                        );
                    },
                );
                wave_selector_cell(
                    ui,
                    &mut state.osc2_waveform,
                    &mut state.osc2_enabled,
                    ParamId::Osc2Waveform,
                    ParamId::Osc2Enabled,
                    control,
                );
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Mix",
                        &mut state.osc_mix,
                        0.0..=1.0,
                        0.0,
                        ParamId::OscMix,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Slop",
                        &mut state.osc_slop,
                        0.0..=1.0,
                        0.0,
                        ParamId::OscSlop,
                        control,
                    );
                });
                ui.end_row();
            });
    });
}

fn lfo_module(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    fixed_panel_scroll(ui, "lfo_panel_scroll", LFO_PANEL_WIDTH, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.add_space(4.0);
                for index in 0..4 {
                    let selected = state.selected_lfo == index;
                    if ui
                        .add_sized(
                            [34.0, 28.0],
                            egui::Button::selectable(selected, format!("{}", index + 1)),
                        )
                        .clicked()
                    {
                        state.selected_lfo = index;
                    }
                    ui.add_space(8.0);
                }
            });

            ui.add_space(16.0);

            ui.vertical(|ui| {
                lfo_shape_selector(ui, state, control);
            });

            ui.add_space(16.0);

            let index = state.selected_lfo;
            ui.vertical(|ui| {
                control_cell(ui, |ui| {
                    param_knob_log_hz(
                        ui,
                        "Freq",
                        &mut state.lfo_rates[index],
                        MIN_LFO_RATE_HZ,
                        MAX_LFO_RATE_HZ,
                        1.0,
                        lfo_rate_param(index),
                        control,
                    );
                });
                ui.add_space(10.0);
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Amount",
                        &mut state.lfo_depths[index],
                        0.0..=1.0,
                        0.0,
                        lfo_depth_param(index),
                        control,
                    );
                });
            });

            ui.add_space(16.0);

            ui.vertical(|ui| {
                lfo_destination_selector(ui, state, control);
                ui.add_space(8.0);
                param_toggle(
                    ui,
                    "Clk Sync",
                    &mut state.lfo_clock_sync[index],
                    lfo_clock_sync_param(index),
                    control,
                );
                param_toggle(
                    ui,
                    "Key Sync",
                    &mut state.lfo_key_sync[index],
                    lfo_key_sync_param(index),
                    control,
                );
            });
        });
    });
}

fn lfo_shape_selector(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    let index = state.selected_lfo;
    ui.label(egui::RichText::new("Shape").strong());
    ui.add_space(4.0);
    ui.vertical(|ui| {
        for (waveform, name) in ["Triangle", "Sawtooth", "Rev Saw", "Square", "Random"]
            .iter()
            .enumerate()
        {
            if ui
                .selectable_label(state.lfo_waveforms[index] == waveform, *name)
                .clicked()
            {
                state.lfo_waveforms[index] = waveform;
                control.set_param(lfo_waveform_param(index), waveform as f32);
            }
        }
    });
}

fn lfo_destination_selector(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    let index = state.selected_lfo;
    ui.label(egui::RichText::new("Destination").strong());
    let current = LfoDestination::from_index(state.lfo_destinations[index]);
    egui::ComboBox::from_id_salt(("lfo_destination", index))
        .width(150.0)
        .selected_text(current.name())
        .show_ui(ui, |ui| {
            for destination in LfoDestination::ALL {
                let destination_index = destination.index();
                if ui
                    .selectable_label(
                        state.lfo_destinations[index] == destination_index,
                        destination.name(),
                    )
                    .clicked()
                {
                    state.lfo_destinations[index] = destination_index;
                    control.set_param(lfo_destination_param(index), destination_index as f32);
                    ui.close();
                }
            }
        });
}

fn filter_module(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    fixed_panel_scroll(ui, "filter_grid_scroll", FILTER_GRID_WIDTH, |ui| {
        egui::Grid::new("filter_grid")
            .num_columns(6)
            .spacing(egui::vec2(12.0, 12.0))
            .show(ui, |ui| {
                control_cell(ui, |ui| {
                    param_knob_log_hz(
                        ui,
                        "Cutoff",
                        &mut state.filter_cutoff,
                        20.0,
                        20_000.0,
                        20_000.0,
                        ParamId::FilterCutoff,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Res",
                        &mut state.filter_resonance,
                        0.0..=1.0,
                        0.0,
                        ParamId::FilterResonance,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_bipolar(
                        ui,
                        "Env Amt",
                        &mut state.filter_env_amount,
                        0.0,
                        ParamId::FilterEnvAmount,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Velocity",
                        &mut state.filter_velocity,
                        0.0..=1.0,
                        0.0,
                        ParamId::FilterVelocity,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Key Amt",
                        &mut state.filter_key_track,
                        0.0..=1.0,
                        0.0,
                        ParamId::FilterKeyTrack,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Osc Mod",
                        &mut state.filter_audio_mod,
                        0.0..=1.0,
                        0.0,
                        ParamId::FilterAudioMod,
                        control,
                    );
                });
                ui.end_row();

                ui.allocate_ui_with_layout(
                    egui::vec2(CONTROL_CELL_W, CONTROL_CELL_H),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.add_space(18.0);
                        pole_toggle(ui, &mut state.filter_poles, control);
                    },
                );

                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Delay",
                        &mut state.filter_delay,
                        0.0..=5.0,
                        0.0,
                        ParamId::FilterEgDelay,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Attack",
                        &mut state.filter_attack,
                        0.0005..=5.0,
                        DEFAULT_ATTACK_SECONDS,
                        ParamId::FilterEgAttack,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Decay",
                        &mut state.filter_decay,
                        0.0005..=5.0,
                        DEFAULT_DECAY_SECONDS,
                        ParamId::FilterEgDecay,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Sustain",
                        &mut state.filter_sustain,
                        0.0..=1.0,
                        DEFAULT_SUSTAIN_LEVEL,
                        ParamId::FilterEgSustain,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Release",
                        &mut state.filter_release,
                        0.0005..=10.0,
                        DEFAULT_RELEASE_SECONDS,
                        ParamId::FilterEgRelease,
                        control,
                    );
                });
                ui.end_row();
            });
    });
}

fn amplifier_module(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    fixed_panel_scroll(ui, "amp_grid_scroll", AMP_GRID_WIDTH, |ui| {
        egui::Grid::new("amp_grid")
            .num_columns(4)
            .spacing(egui::vec2(12.0, 12.0))
            .show(ui, |ui| {
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Pan Sprd",
                        &mut state.amp_pan_spread,
                        0.0..=1.0,
                        0.0,
                        ParamId::PanSpread,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Env Amt",
                        &mut state.amp_env_amount,
                        0.0..=1.0,
                        1.0,
                        ParamId::AmpEnvAmount,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Velocity",
                        &mut state.amp_velocity,
                        0.0..=1.0,
                        1.0,
                        ParamId::AmpVelocity,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Delay",
                        &mut state.amp_delay,
                        0.0..=5.0,
                        0.0,
                        ParamId::AmpEgDelay,
                        control,
                    );
                });
                ui.end_row();

                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Attack",
                        &mut state.amp_attack,
                        0.0005..=5.0,
                        DEFAULT_ATTACK_SECONDS,
                        ParamId::AmpEgAttack,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Decay",
                        &mut state.amp_decay,
                        0.0005..=5.0,
                        DEFAULT_DECAY_SECONDS,
                        ParamId::AmpEgDecay,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Sustain",
                        &mut state.amp_sustain,
                        0.0..=1.0,
                        DEFAULT_SUSTAIN_LEVEL,
                        ParamId::AmpEgSustain,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Release",
                        &mut state.amp_release,
                        0.0005..=10.0,
                        DEFAULT_RELEASE_SECONDS,
                        ParamId::AmpEgRelease,
                        control,
                    );
                });
                ui.end_row();
            });
    });
}

fn auxiliary_envelope_module(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    fixed_panel_scroll(ui, "aux_envelope_grid_scroll", AUX_GRID_WIDTH, |ui| {
        egui::Grid::new("aux_envelope_grid")
            .num_columns(4)
            .spacing(egui::vec2(12.0, 12.0))
            .show(ui, |ui| {
                aux_destination_cell(ui, state, control);
                control_cell(ui, |ui| {
                    param_knob_bipolar(
                        ui,
                        "Env Amt",
                        &mut state.aux_env_amount,
                        0.0,
                        ParamId::AuxEgAmount,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Velocity",
                        &mut state.aux_velocity,
                        0.0..=1.0,
                        0.0,
                        ParamId::AuxEgVelocity,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Delay",
                        &mut state.aux_delay,
                        0.0..=5.0,
                        0.0,
                        ParamId::AuxEgDelay,
                        control,
                    );
                });
                ui.end_row();

                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Attack",
                        &mut state.aux_attack,
                        0.0005..=5.0,
                        DEFAULT_ATTACK_SECONDS,
                        ParamId::AuxEgAttack,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Decay",
                        &mut state.aux_decay,
                        0.0005..=5.0,
                        DEFAULT_DECAY_SECONDS,
                        ParamId::AuxEgDecay,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Sustain",
                        &mut state.aux_sustain,
                        0.0..=1.0,
                        DEFAULT_SUSTAIN_LEVEL,
                        ParamId::AuxEgSustain,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Release",
                        &mut state.aux_release,
                        0.0005..=10.0,
                        DEFAULT_RELEASE_SECONDS,
                        ParamId::AuxEgRelease,
                        control,
                    );
                });
                ui.end_row();
            });
    });
}

fn aux_destination_cell(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    ui.allocate_ui_with_layout(
        egui::vec2(DEST_CELL_W, CONTROL_CELL_H),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.add_space(8.0);
            let current = LfoDestination::from_index(state.aux_destination);
            egui::ComboBox::from_id_salt("aux_destination")
                .width(104.0)
                .selected_text(current.name())
                .show_ui(ui, |ui| {
                    for destination in LfoDestination::ALL {
                        let destination_index = destination.index();
                        if ui
                            .selectable_label(
                                state.aux_destination == destination_index,
                                destination.name(),
                            )
                            .clicked()
                        {
                            state.aux_destination = destination_index;
                            control.set_param(ParamId::AuxEgDestination, destination_index as f32);
                            ui.close();
                        }
                    }
                });
            ui.add_space(4.0);
            ui.label("Destination");
            ui.add_space(4.0);
            param_toggle(
                ui,
                "Repeat",
                &mut state.aux_repeat,
                ParamId::AuxEgLoop,
                control,
            );
        },
    );
}

fn pole_toggle(ui: &mut egui::Ui, filter_poles: &mut usize, control: &SynthEngineControl) {
    if ui.selectable_label(*filter_poles == 1, "4 Pole").clicked() {
        *filter_poles = if *filter_poles == 1 { 0 } else { 1 };
        control.set_param(ParamId::FilterPoles, *filter_poles as f32);
    }
}

fn fixed_panel_scroll(
    ui: &mut egui::Ui,
    id: &'static str,
    min_width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_min_width(min_width);
            add_contents(ui);
        });
}

fn control_cell(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(CONTROL_CELL_W, CONTROL_CELL_H),
        egui::Layout::top_down(egui::Align::Center),
        add_contents,
    );
}

fn strong_label(ui: &mut egui::Ui, text: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(36.0, CONTROL_CELL_H),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), KNOB_SIZE),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label(egui::RichText::new(text).strong());
                },
            );
        },
    );
}

fn wave_selector_cell(
    ui: &mut egui::Ui,
    waveform: &mut usize,
    enabled: &mut bool,
    waveform_param: ParamId,
    enabled_param: ParamId,
    control: &SynthEngineControl,
) {
    ui.allocate_ui_with_layout(
        egui::vec2(WAVE_CELL_W, CONTROL_CELL_H),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            for (index, name) in ["Saw", "Saw+Tri", "Triangle", "Pulse"].iter().enumerate() {
                if ui.selectable_label(*waveform == index, *name).clicked() {
                    *waveform = index;
                    *enabled = true;
                    control.set_param(enabled_param, 1.0);
                    control.set_param(waveform_param, index as f32);
                }
            }
        },
    );
}

fn lfo_rate_param(index: usize) -> ParamId {
    match index {
        0 => ParamId::Lfo1Rate,
        1 => ParamId::Lfo2Rate,
        2 => ParamId::Lfo3Rate,
        _ => ParamId::Lfo4Rate,
    }
}

fn lfo_depth_param(index: usize) -> ParamId {
    match index {
        0 => ParamId::Lfo1Depth,
        1 => ParamId::Lfo2Depth,
        2 => ParamId::Lfo3Depth,
        _ => ParamId::Lfo4Depth,
    }
}

fn lfo_waveform_param(index: usize) -> ParamId {
    match index {
        0 => ParamId::Lfo1Waveform,
        1 => ParamId::Lfo2Waveform,
        2 => ParamId::Lfo3Waveform,
        _ => ParamId::Lfo4Waveform,
    }
}

fn lfo_destination_param(index: usize) -> ParamId {
    match index {
        0 => ParamId::Lfo1Destination,
        1 => ParamId::Lfo2Destination,
        2 => ParamId::Lfo3Destination,
        _ => ParamId::Lfo4Destination,
    }
}

fn lfo_clock_sync_param(index: usize) -> ParamId {
    match index {
        0 => ParamId::Lfo1ClockSync,
        1 => ParamId::Lfo2ClockSync,
        2 => ParamId::Lfo3ClockSync,
        _ => ParamId::Lfo4ClockSync,
    }
}

fn lfo_key_sync_param(index: usize) -> ParamId {
    match index {
        0 => ParamId::Lfo1KeySync,
        1 => ParamId::Lfo2KeySync,
        2 => ParamId::Lfo3KeySync,
        _ => ParamId::Lfo4KeySync,
    }
}

pub struct PatchManager {
    pub save_name: String,
    pub patch_names: Vec<String>,
    config_dir: PathBuf,
    patches_dir: PathBuf,
}

impl PatchManager {
    pub fn new() -> Self {
        let config_dir = directories::ProjectDirs::from("", "", "AnalogSynth")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_default();
        let patches_dir = config_dir.join("patches");
        let _ = std::fs::create_dir_all(&patches_dir);
        let patch_names = list_patch_files(&patches_dir);
        Self {
            save_name: String::new(),
            patch_names,
            config_dir,
            patches_dir,
        }
    }

    pub fn save_patch(&self, name: &str, patch: &Patch) {
        let path = self.patches_dir.join(format!("{name}.json"));
        if let Ok(json) = serde_json::to_string_pretty(patch) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub fn save_autosave(&self, patch: &Patch) {
        let path = self.config_dir.join("patch.json");
        if let Ok(json) = serde_json::to_string_pretty(patch) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub fn load_autosave(&self) -> Option<Patch> {
        let path = self.config_dir.join("patch.json");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn load_patch(&self, name: &str) -> Option<Patch> {
        let path = self.patches_dir.join(format!("{name}.json"));
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn refresh(&mut self) {
        self.patch_names = list_patch_files(&self.patches_dir);
    }
}

fn list_patch_files(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension()?.to_str()? == "json" {
                path.file_stem()?.to_str().map(String::from)
            } else {
                None
            }
        })
        .collect();
    names.sort();
    names
}
