use eframe::egui;

use crate::engine::{AudioMetrics, SynthEngineControl};
use crate::ui::widgets::{
    KNOB_SIZE, param_knob_bipolar, param_knob_f32, param_knob_log_hz, param_toggle,
};
use synth_core::{
    DEFAULT_ATTACK_SECONDS, DEFAULT_DECAY_SECONDS, DEFAULT_RELEASE_SECONDS, DEFAULT_SUSTAIN_LEVEL,
    LfoDestination, MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ, ParamId,
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
) {
    command_row(ui, control, voice_active, voice_total, metrics, analysis_open);

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
}

fn command_row(
    ui: &mut egui::Ui,
    control: &SynthEngineControl,
    voice_active: usize,
    voice_total: usize,
    metrics: Option<AudioMetrics>,
    analysis_open: &mut bool,
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

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(metrics) = metrics {
                ui.label(metrics_text(&metrics));
                ui.separator();
            }
            ui.label(format!("Voices: {voice_active}/{voice_total}"));
        });
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
