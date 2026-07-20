use std::path::PathBuf;

use eframe::egui;

use crate::engine::{MidiUiUpdate, SynthEngineControl};
use crate::ui::widgets::{
    KNOB_SIZE, framed_selectable, framed_selectable_sized, master_volume, param_knob_bipolar,
    param_knob_f32, param_knob_f32_offset, param_knob_log_hz, param_knob_note, param_toggle,
    param_toggle_sized,
};
use synth_core::{
    DEFAULT_ATTACK_SECONDS, DEFAULT_DECAY_SECONDS, DEFAULT_RELEASE_SECONDS, DEFAULT_SUSTAIN_LEVEL,
    DedicatedModSlot, DedicatedModSource, EffectParams, EffectType, FilterType, GlideMode, KeyMode,
    MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ, ModDestination, ModMatrix, ModMatrixSlot, ModRoute,
    ModSource, ModulationParam, OscillatorPatch, PanModMode, ParamId, Patch, UnisonMode,
};

const WIDE_LAYOUT_MIN_WIDTH: f32 = 860.0;
const OSC_GRID_WIDTH: f32 = 840.0;
const LFO_PANEL_WIDTH: f32 = 400.0;
const MOD_MATRIX_PANEL_WIDTH: f32 = 760.0;
const MOD_MATRIX_EXPANDED_ID: &str = "mod_matrix_expanded";
const MOD_DEDICATED_LABEL_WIDTH: f32 = 110.0;
const EFFECTS_PANEL_WIDTH: f32 = 560.0;
const FILTER_GRID_WIDTH: f32 = 360.0;
const AMP_GRID_WIDTH: f32 = 320.0;
const AUX_GRID_WIDTH: f32 = 360.0;
const CONTROL_CELL_W: f32 = 46.0;
const CONTROL_CELL_H: f32 = 64.0;
const DEST_CELL_W: f32 = 112.0;
const WAVE_CELL_W: f32 = 112.0;
const WAVE_BUTTON_SIZE: egui::Vec2 = egui::vec2(56.0, 22.0);
const LFO_SHAPE_BUTTON_SIZE: egui::Vec2 = egui::vec2(78.0, 22.0);
const LFO_SYNC_BUTTON_SIZE: egui::Vec2 = egui::vec2(72.0, 22.0);
const LFO_INDEX_BUTTON_SIZE: egui::Vec2 = egui::vec2(34.0, 28.0);
const COMBO_COLUMN_W: f32 = 80.0;
const COMBO_CONTROL_W: f32 = 78.0;
const COMBO_BUTTON_SIZE: egui::Vec2 = egui::vec2(COMBO_CONTROL_W, 22.0);
const GLIDE_COLUMN_W: f32 = 98.0;
const GLIDE_CONTROL_W: f32 = 96.0;
const GLIDE_BUTTON_SIZE: egui::Vec2 = egui::vec2(GLIDE_CONTROL_W, 22.0);
const GLIDE_CELL_H: f32 = 64.0;
const UNISON_COLUMN_W: f32 = 90.0;
const UNISON_CONTROL_W: f32 = 88.0;
const UNISON_BUTTON_SIZE: egui::Vec2 = egui::vec2(UNISON_CONTROL_W, 22.0);
const UNISON_CELL_H: f32 = 64.0;
const MOD_SLOT_BUTTON_SIZE: egui::Vec2 = egui::vec2(28.0, 26.0);
const EFFECT_TYPE_COUNT: usize = 13;

#[derive(Clone, Copy)]
struct EffectRuntimeParams {
    mix: f32,
    clock_sync: bool,
    param1: f32,
    param2: f32,
}

impl Default for EffectRuntimeParams {
    fn default() -> Self {
        let params = EffectParams::default();
        Self {
            mix: params.mix,
            clock_sync: params.clock_sync,
            param1: params.param1,
            param2: params.param2,
        }
    }
}

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
    pub osc1_shape_mod: f32,
    pub osc2_shape_mod: f32,
    pub osc_mix: f32,
    pub sync: bool,
    pub osc_slop: f32,
    pub osc1_note_reset: bool,
    pub osc2_note_reset: bool,
    pub osc1_glide: f32,
    pub osc2_glide: f32,
    pub osc1_keyboard_on: bool,
    pub osc2_keyboard_on: bool,
    pub glide_mode: usize,
    pub glide_enabled: bool,
    pub glide_time: f32,
    pub pitch_bend_range: f32,
    pub key_mode: usize,
    pub unison_enabled: bool,
    pub unison_mode: usize,
    pub unison_detune: f32,
    pub bpm: f32,
    pub clock_divide: usize,
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
    pub amp_pan_mod_mode: PanModMode,
    pub amp_vca_initial_level: f32,
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
    pub selected_mod_route: usize,
    pub mod_enabled: [bool; 8],
    pub mod_sources: [usize; 8],
    pub mod_destinations: [usize; 8],
    pub mod_amounts: [f32; 8],
    pub dedicated_mod_enabled: [bool; 5],
    pub dedicated_mod_destinations: [usize; 5],
    pub dedicated_mod_amounts: [f32; 5],
    pub effect_enabled: bool,
    pub effect_type: usize,
    pub effect_mix: f32,
    pub effect_clock_sync: bool,
    pub effect_param1: f32,
    pub effect_param2: f32,
    effect_runtime_params: [EffectRuntimeParams; EFFECT_TYPE_COUNT],
    pub master_volume: f32,
    pub play_pitch_class: u8,
    pub play_octave: i8,
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
            osc1_shape_mod: 0.0,
            osc2_shape_mod: 0.0,
            osc_mix: 0.0,
            sync: false,
            osc_slop: 0.0,
            osc1_note_reset: true,
            osc2_note_reset: true,
            osc1_glide: 0.0,
            osc2_glide: 0.0,
            osc1_keyboard_on: true,
            osc2_keyboard_on: true,
            glide_mode: 0,
            glide_enabled: false,
            glide_time: 0.0,
            pitch_bend_range: 2.0,
            key_mode: 0,
            unison_enabled: false,
            unison_mode: 0,
            unison_detune: 0.0,
            bpm: 120.0,
            clock_divide: 1,
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
            amp_pan_mod_mode: PanModMode::Alternate,
            amp_vca_initial_level: 0.0,
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
            selected_mod_route: 0,
            mod_enabled: [false; 8],
            mod_sources: [0; 8],
            mod_destinations: [0; 8],
            mod_amounts: [0.0; 8],
            dedicated_mod_enabled: [false; 5],
            dedicated_mod_destinations: [0; 5],
            dedicated_mod_amounts: [0.0; 5],
            effect_enabled: false,
            effect_type: 0,
            effect_mix: 0.0,
            effect_clock_sync: false,
            effect_param1: 0.25,
            effect_param2: 0.25,
            effect_runtime_params: [EffectRuntimeParams::default(); EFFECT_TYPE_COUNT],
            master_volume: 0.8,
            play_pitch_class: 0,
            play_octave: 4,
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
        self.osc1_shape_mod = patch.osc1.shape_mod;
        self.osc2_shape_mod = patch.osc2.shape_mod;
        self.osc_mix = patch.osc_mix;
        self.sync = patch.hard_sync;
        self.osc_slop = patch.osc_slop;
        self.osc1_note_reset = patch.osc1.note_reset;
        self.osc2_note_reset = patch.osc2.note_reset;
        self.osc1_glide = patch.osc1.glide;
        self.osc2_glide = patch.osc2.glide;
        self.osc1_keyboard_on = patch.osc1.keyboard_on;
        self.osc2_keyboard_on = patch.osc2.keyboard_on;
        self.glide_mode = patch.glide_mode.index();
        self.glide_enabled = patch.glide_enabled;
        self.key_mode = patch.key_mode.index();
        self.unison_enabled = patch.unison_enabled;
        self.unison_mode = patch.unison_mode.index();
        self.unison_detune = patch.unison_detune;
        self.bpm = patch.bpm;
        self.clock_divide = patch.clock_divide as usize;
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
        self.amp_pan_mod_mode = patch.amplifier.pan_mod_mode;
        self.amp_vca_initial_level = patch.amplifier.initial_level;
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
        for i in 0..8 {
            let slot = patch.mod_matrix.free_slots[i];
            self.mod_enabled[i] = slot.enabled;
            self.mod_sources[i] = slot.source.index();
            self.mod_destinations[i] = slot.destination.index();
            self.mod_amounts[i] = slot.amount;
        }
        for i in 0..5 {
            let slot = patch.mod_matrix.dedicated[i];
            self.dedicated_mod_enabled[i] = slot.enabled;
            self.dedicated_mod_destinations[i] = slot.destination.index();
            self.dedicated_mod_amounts[i] = slot.amount;
        }
        self.effect_enabled = patch.effects.enabled;
        self.effect_type = patch.effects.effect_type.index();
        self.effect_mix = patch.effects.mix;
        self.effect_clock_sync = patch.effects.clock_sync;
        if !EffectType::from_index(self.effect_type).is_delay() {
            self.effect_clock_sync = false;
        }
        self.effect_param1 = patch.effects.param1;
        self.effect_param2 = patch.effects.param2;
        self.effect_runtime_params = [EffectRuntimeParams::default(); EFFECT_TYPE_COUNT];
        self.store_active_effect_params();
        self.master_volume = patch.master_volume;
    }

    pub fn apply_midi_update(&mut self, update: MidiUiUpdate) {
        match update {
            MidiUiUpdate::Param(param, value) => self.apply_midi_param(param, value),
            MidiUiUpdate::Modulation { route, parameter } => {
                self.apply_midi_modulation(route, parameter);
            }
        }
    }

    fn apply_midi_param(&mut self, param: ParamId, value: f32) {
        let enabled = value >= 0.5;
        match param {
            ParamId::Osc1Waveform => self.osc1_waveform = value as usize,
            ParamId::Osc1Enabled => self.osc1_enabled = enabled,
            ParamId::Osc1Frequency => self.osc1_freq = value,
            ParamId::Osc1FineTune => self.osc1_fine = value,
            ParamId::Osc1ShapeMod => self.osc1_shape_mod = value,
            ParamId::Osc2Waveform => self.osc2_waveform = value as usize,
            ParamId::Osc2Enabled => self.osc2_enabled = enabled,
            ParamId::Osc2Frequency => self.osc2_freq = value,
            ParamId::Osc2FineTune => self.osc2_fine = value,
            ParamId::Osc2ShapeMod => self.osc2_shape_mod = value,
            ParamId::OscMix => self.osc_mix = value,
            ParamId::SubOscLevel => self.sub_level = value,
            ParamId::NoiseLevel => self.noise_level = value,
            ParamId::HardSync => self.sync = enabled,
            ParamId::OscSlop | ParamId::AnalogDrift => self.osc_slop = value,
            ParamId::Osc1NoteReset => self.osc1_note_reset = enabled,
            ParamId::Osc2NoteReset => self.osc2_note_reset = enabled,
            ParamId::Osc1KeyboardOn => self.osc1_keyboard_on = enabled,
            ParamId::Osc2KeyboardOn => self.osc2_keyboard_on = enabled,
            ParamId::Osc1Glide => self.osc1_glide = value,
            ParamId::Osc2Glide => self.osc2_glide = value,
            ParamId::GlideMode => self.glide_mode = value as usize,
            ParamId::GlideEnabled => self.glide_enabled = enabled,
            ParamId::GlideTime => self.glide_time = value,
            ParamId::PitchBendRange => self.pitch_bend_range = value,
            ParamId::KeyMode => self.key_mode = value as usize,
            ParamId::UnisonEnabled => self.unison_enabled = enabled,
            ParamId::UnisonMode => self.unison_mode = value as usize,
            ParamId::UnisonDetune => self.unison_detune = value,
            ParamId::Bpm => self.bpm = value.clamp(30.0, 250.0),
            ParamId::ClockDivide => self.clock_divide = value as usize,
            ParamId::FilterCutoff => self.filter_cutoff = value,
            ParamId::FilterResonance => self.filter_resonance = value,
            ParamId::FilterPoles => self.filter_poles = usize::from(enabled),
            ParamId::FilterKeyTrack => self.filter_key_track = value,
            ParamId::FilterEnvAmount => self.filter_env_amount = value,
            ParamId::FilterVelocity => self.filter_velocity = value,
            ParamId::FilterAudioMod => self.filter_audio_mod = value,
            ParamId::FilterEgDelay => self.filter_delay = value,
            ParamId::FilterEgAttack => self.filter_attack = value,
            ParamId::FilterEgDecay => self.filter_decay = value,
            ParamId::FilterEgSustain => self.filter_sustain = value,
            ParamId::FilterEgRelease => self.filter_release = value,
            ParamId::PanSpread => self.amp_pan_spread = value,
            ParamId::PanModMode => self.amp_pan_mod_mode = PanModMode::from_param(value),
            ParamId::VcaInitialLevel => self.amp_vca_initial_level = value,
            ParamId::AmpEnvAmount => self.amp_env_amount = value,
            ParamId::AmpVelocity => self.amp_velocity = value,
            ParamId::AmpEgDelay => self.amp_delay = value,
            ParamId::AmpEgAttack => self.amp_attack = value,
            ParamId::AmpEgDecay => self.amp_decay = value,
            ParamId::AmpEgSustain => self.amp_sustain = value,
            ParamId::AmpEgRelease => self.amp_release = value,
            ParamId::AuxEgDestination => self.aux_destination = value as usize,
            ParamId::AuxEgAmount => self.aux_env_amount = value,
            ParamId::AuxEgVelocity => self.aux_velocity = value,
            ParamId::AuxEgDelay => self.aux_delay = value,
            ParamId::AuxEgAttack => self.aux_attack = value,
            ParamId::AuxEgDecay => self.aux_decay = value,
            ParamId::AuxEgSustain => self.aux_sustain = value,
            ParamId::AuxEgRelease => self.aux_release = value,
            ParamId::AuxEgLoop => self.aux_repeat = enabled,
            ParamId::Lfo1Rate => self.lfo_rates[0] = value,
            ParamId::Lfo1Depth => self.lfo_depths[0] = value,
            ParamId::Lfo1Waveform => self.lfo_waveforms[0] = value as usize,
            ParamId::Lfo1Destination => self.lfo_destinations[0] = value as usize,
            ParamId::Lfo1ClockSync => self.lfo_clock_sync[0] = enabled,
            ParamId::Lfo1KeySync => self.lfo_key_sync[0] = enabled,
            ParamId::Lfo2Rate => self.lfo_rates[1] = value,
            ParamId::Lfo2Depth => self.lfo_depths[1] = value,
            ParamId::Lfo2Waveform => self.lfo_waveforms[1] = value as usize,
            ParamId::Lfo2Destination => self.lfo_destinations[1] = value as usize,
            ParamId::Lfo2ClockSync => self.lfo_clock_sync[1] = enabled,
            ParamId::Lfo2KeySync => self.lfo_key_sync[1] = enabled,
            ParamId::Lfo3Rate => self.lfo_rates[2] = value,
            ParamId::Lfo3Depth => self.lfo_depths[2] = value,
            ParamId::Lfo3Waveform => self.lfo_waveforms[2] = value as usize,
            ParamId::Lfo3Destination => self.lfo_destinations[2] = value as usize,
            ParamId::Lfo3ClockSync => self.lfo_clock_sync[2] = enabled,
            ParamId::Lfo3KeySync => self.lfo_key_sync[2] = enabled,
            ParamId::Lfo4Rate => self.lfo_rates[3] = value,
            ParamId::Lfo4Depth => self.lfo_depths[3] = value,
            ParamId::Lfo4Waveform => self.lfo_waveforms[3] = value as usize,
            ParamId::Lfo4Destination => self.lfo_destinations[3] = value as usize,
            ParamId::Lfo4ClockSync => self.lfo_clock_sync[3] = enabled,
            ParamId::Lfo4KeySync => self.lfo_key_sync[3] = enabled,
            ParamId::EffectEnabled => self.effect_enabled = enabled,
            ParamId::EffectType => self.select_effect((value as usize).min(EFFECT_TYPE_COUNT - 1)),
            ParamId::EffectMix => self.effect_mix = value,
            ParamId::EffectClockSync => self.effect_clock_sync = enabled,
            ParamId::EffectParam1 => self.effect_param1 = value,
            ParamId::EffectParam2 => self.effect_param2 = value,
            ParamId::MasterVolume => self.master_volume = value,
            _ => return,
        }
        if matches!(
            param,
            ParamId::EffectMix
                | ParamId::EffectClockSync
                | ParamId::EffectParam1
                | ParamId::EffectParam2
        ) {
            self.store_active_effect_params();
        }
    }

    fn apply_midi_modulation(&mut self, route: ModRoute, parameter: ModulationParam) {
        match route {
            ModRoute::Free(index) if index < self.mod_enabled.len() => {
                match parameter {
                    ModulationParam::Source(source) => self.mod_sources[index] = source.index(),
                    ModulationParam::Destination(destination) => {
                        self.mod_destinations[index] = destination.index();
                    }
                    ModulationParam::Amount(amount) => self.mod_amounts[index] = amount,
                }
                if !matches!(parameter, ModulationParam::Amount(_)) {
                    self.mod_enabled[index] = self.mod_sources[index] != ModSource::Off.index()
                        && self.mod_destinations[index] != ModDestination::Off.index();
                }
            }
            ModRoute::Dedicated(source) => {
                let Some(index) = DedicatedModSource::ALL
                    .iter()
                    .position(|item| *item == source)
                else {
                    return;
                };
                match parameter {
                    ModulationParam::Destination(destination) => {
                        self.dedicated_mod_destinations[index] = destination.index();
                        self.dedicated_mod_enabled[index] = destination != ModDestination::Off;
                    }
                    ModulationParam::Amount(amount) => {
                        self.dedicated_mod_amounts[index] = amount;
                    }
                    ModulationParam::Source(_) => {}
                }
            }
            ModRoute::Free(_) => {}
        }
    }

    fn store_active_effect_params(&mut self) {
        self.effect_runtime_params[self.effect_type] = EffectRuntimeParams {
            mix: self.effect_mix,
            clock_sync: self.effect_clock_sync,
            param1: self.effect_param1,
            param2: self.effect_param2,
        };
    }

    fn select_effect(&mut self, effect_type: usize) {
        self.store_active_effect_params();
        self.effect_type = effect_type;
        let params = self.effect_runtime_params[effect_type];
        self.effect_mix = params.mix;
        self.effect_clock_sync = params.clock_sync;
        self.effect_param1 = params.param1;
        self.effect_param2 = params.param2;
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
                shape_mod: state.osc1_shape_mod,
                level: 1.0,
                note_reset: state.osc1_note_reset,
                keyboard_on: state.osc1_keyboard_on,
                glide: state.osc1_glide,
            },
            osc2: OscillatorPatch {
                waveform: state.osc2_waveform as u8,
                enabled: state.osc2_enabled,
                frequency: state.osc2_freq,
                fine_tune: state.osc2_fine,
                shape_mod: state.osc2_shape_mod,
                level: 1.0,
                note_reset: state.osc2_note_reset,
                keyboard_on: state.osc2_keyboard_on,
                glide: state.osc2_glide,
            },
            osc_mix: state.osc_mix,
            sub_osc_level: state.sub_level,
            noise_level: state.noise_level,
            hard_sync: state.sync,
            osc_slop: state.osc_slop,
            glide_time: state.glide_time,
            glide_mode: GlideMode::from_index(state.glide_mode),
            glide_enabled: state.glide_enabled,
            pitch_bend_range: state.pitch_bend_range,
            key_mode: KeyMode::from_index(state.key_mode),
            unison_enabled: state.unison_enabled,
            unison_mode: UnisonMode::from_index(state.unison_mode),
            unison_detune: state.unison_detune,
            bpm: state.bpm,
            clock_divide: state.clock_divide as f32,
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
                pan_mod_mode: state.amp_pan_mod_mode,
                initial_level: state.amp_vca_initial_level,
                env_amount: state.amp_env_amount,
                velocity: state.amp_velocity,
                eg_delay: state.amp_delay,
                eg_attack: state.amp_attack,
                eg_decay: state.amp_decay,
                eg_sustain: state.amp_sustain,
                eg_release: state.amp_release,
            },
            aux_envelope: synth_core::AuxEnvelopeParams {
                destination: ModDestination::from_index(state.aux_destination),
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
                    destination: ModDestination::from_index(state.lfo_destinations[0]),
                    clock_sync: state.lfo_clock_sync[0],
                    key_sync: state.lfo_key_sync[0],
                },
                synth_core::LfoParams {
                    rate_hz: state.lfo_rates[1],
                    depth: state.lfo_depths[1],
                    waveform: lfo_wf(1),
                    destination: ModDestination::from_index(state.lfo_destinations[1]),
                    clock_sync: state.lfo_clock_sync[1],
                    key_sync: state.lfo_key_sync[1],
                },
                synth_core::LfoParams {
                    rate_hz: state.lfo_rates[2],
                    depth: state.lfo_depths[2],
                    waveform: lfo_wf(2),
                    destination: ModDestination::from_index(state.lfo_destinations[2]),
                    clock_sync: state.lfo_clock_sync[2],
                    key_sync: state.lfo_key_sync[2],
                },
                synth_core::LfoParams {
                    rate_hz: state.lfo_rates[3],
                    depth: state.lfo_depths[3],
                    waveform: lfo_wf(3),
                    destination: ModDestination::from_index(state.lfo_destinations[3]),
                    clock_sync: state.lfo_clock_sync[3],
                    key_sync: state.lfo_key_sync[3],
                },
            ],
            mod_matrix: ModMatrix {
                free_slots: core::array::from_fn(|i| ModMatrixSlot {
                    enabled: state.mod_enabled[i],
                    source: ModSource::from_index(state.mod_sources[i]),
                    destination: ModDestination::from_index(state.mod_destinations[i]),
                    amount: state.mod_amounts[i],
                }),
                dedicated: core::array::from_fn(|i| DedicatedModSlot {
                    enabled: state.dedicated_mod_enabled[i],
                    destination: ModDestination::from_index(state.dedicated_mod_destinations[i]),
                    amount: state.dedicated_mod_amounts[i],
                }),
            },
            effects: EffectParams {
                enabled: state.effect_enabled,
                effect_type: EffectType::from_index(state.effect_type),
                mix: state.effect_mix,
                clock_sync: state.effect_clock_sync,
                param1: state.effect_param1,
                param2: state.effect_param2,
            },
            master_volume: state.master_volume,
            name: synth_core::PatchName::new(),
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut UiState,
    control: &SynthEngineControl,
    analysis_open: &mut bool,
    patch_mgr: &mut PatchManager,
    filter_type: &mut FilterType,
    midi_output_port: Option<&str>,
    muted: &mut bool,
) {
    let patch_before = Patch::from(&*state);

    command_row(
        ui,
        control,
        analysis_open,
        state,
        patch_mgr,
        midi_output_port,
        muted,
    );

    ui.add_space(6.0);

    ui.spacing_mut().scroll.fade.strength = 0.0;
    egui::ScrollArea::vertical().show(ui, |ui| {
        module_panel(ui, "Oscillators", |ui| {
            oscillators_module(ui, state, control);
        });

        ui.add_space(8.0);

        if ui.available_width() >= WIDE_LAYOUT_MIN_WIDTH {
            ui.columns(2, |columns| {
                module_panel_with_header(
                    &mut columns[0],
                    "Low-Pass Filter",
                    |ui| filter_model_combo(ui, filter_type, control),
                    |ui| filter_module(ui, state, control),
                );
                module_panel(&mut columns[1], "Amplifier", |ui| {
                    amplifier_module(ui, state, control);
                });
            });
        } else {
            module_panel_with_header(
                ui,
                "Low-Pass Filter",
                |ui| filter_model_combo(ui, filter_type, control),
                |ui| filter_module(ui, state, control),
            );
            ui.add_space(8.0);
            module_panel(ui, "Amplifier", |ui| {
                amplifier_module(ui, state, control);
            });
        }

        ui.add_space(8.0);

        ui.columns(2, |columns| {
            module_panel(&mut columns[0], "Low Frequency Oscillators", |ui| {
                lfo_module(ui, state, control);
            });

            module_panel(&mut columns[1], "Auxiliary Envelope", |ui| {
                auxiliary_envelope_module(ui, state, control);
            });
        });

        ui.add_space(8.0);

        modulation_matrix_panel(ui, state, control);

        ui.add_space(8.0);

        module_panel(ui, "Effects", |ui| {
            effects_module(ui, state, control);
        });

        ui.add_space(8.0);

        module_panel(ui, "Misc", |ui| {
            ui.horizontal(|ui| {
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Bend Range",
                        &mut state.pitch_bend_range,
                        0.0..=12.0,
                        2.0,
                        ParamId::PitchBendRange,
                        control,
                    );
                });
            });
        });
    });

    let finalized = patch_mgr.finalize_loaded_patch(state);
    if !finalized && user_edited_patch(ui, &patch_before, &Patch::from(&*state)) {
        patch_mgr.mark_user_modified();
    }
    let name_focused = ui.memory(|memory| memory.has_focus(egui::Id::new(PATCH_NAME_FIELD_ID)));
    patch_mgr.sync_display_name(name_focused);
}

const MODIFIED_SUFFIX: &str = " (modified)";
const PATCH_NAME_FIELD_ID: &str = "patch_name_field";
const PATCH_LOAD_FILTER_ID: &str = "patch_load_filter";
const PATCH_LOAD_WAS_OPEN_ID: &str = "patch_load_was_open";
const PATCH_LOAD_POPUP_WIDTH: f32 = 380.0;
const PATCH_LOAD_POPUP_HEIGHT: f32 = 440.0;
const PATCH_LOAD_POPUP_CHROME_HEIGHT: f32 = 40.0;

fn load_patch_by_name(
    patch_mgr: &mut PatchManager,
    control: &SynthEngineControl,
    state: &mut UiState,
    name: &str,
    muted: bool,
) {
    if let Some(patch) = patch_mgr.load_patch(name) {
        state.apply_from_patch(&patch);
        control.load_patch_respecting_mute(&patch, muted);
        patch_mgr.begin_loaded_patch(name);
    }
}

fn command_row(
    ui: &mut egui::Ui,
    control: &SynthEngineControl,
    analysis_open: &mut bool,
    state: &mut UiState,
    patch_mgr: &mut PatchManager,
    midi_output_port: Option<&str>,
    muted: &mut bool,
) {
    ui.horizontal(|ui| {
        let left = ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label("Patch:");
                let has_patches = !patch_mgr.patch_names.is_empty();
                let prev = ui.add_enabled(has_patches, egui::Button::new("◀"));
                if prev.clicked() {
                    patch_mgr.refresh();
                    if let Some(name) = patch_mgr.adjacent_patch_name(-1).map(str::to_string) {
                        load_patch_by_name(patch_mgr, control, state, &name, *muted);
                    }
                }
                prev.on_hover_text("Previous patch");
                egui::Frame::NONE
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .inner_margin(egui::Margin::symmetric(4, 2))
                    .corner_radius(2.0)
                    .show(ui, |ui| {
                        ui.add_sized(
                            [200.0, 18.0],
                            egui::TextEdit::singleline(&mut patch_mgr.save_name)
                                .id_salt(PATCH_NAME_FIELD_ID)
                                .frame(egui::Frame::NONE),
                        );
                    });
                let next = ui.add_enabled(has_patches, egui::Button::new("▶"));
                if next.clicked() {
                    patch_mgr.refresh();
                    if let Some(name) = patch_mgr.adjacent_patch_name(1).map(str::to_string) {
                        load_patch_by_name(patch_mgr, control, state, &name, *muted);
                    }
                }
                next.on_hover_text("Next patch");
                let was_open = ui
                    .data(|data| data.get_temp::<bool>(egui::Id::new(PATCH_LOAD_WAS_OPEN_ID)))
                    .unwrap_or(false);
                let load_response = egui::ComboBox::from_id_salt("patch_load")
                    .selected_text("Load")
                    .width(56.0)
                    .height(PATCH_LOAD_POPUP_HEIGHT + PATCH_LOAD_POPUP_CHROME_HEIGHT)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show_ui(ui, |ui| {
                        patch_mgr.refresh();
                        ui.set_min_width(PATCH_LOAD_POPUP_WIDTH);

                        let filter_id = egui::Id::new(PATCH_LOAD_FILTER_ID);
                        let mut filter = ui.data_mut(|data| {
                            data.get_temp_mut_or_default::<String>(filter_id).clone()
                        });
                        ui.horizontal(|ui| {
                            ui.label("Filter:");
                            let filter_response = ui.add(
                                egui::TextEdit::singleline(&mut filter)
                                    .id(filter_id)
                                    .hint_text("File name...")
                                    .desired_width(ui.available_width()),
                            );
                            if !was_open {
                                filter_response.request_focus();
                            }
                        });
                        ui.data_mut(|data| {
                            *data.get_temp_mut_or_default::<String>(filter_id) = filter
                        });

                        ui.separator();

                        if patch_mgr.patch_names.is_empty() {
                            ui.label("No saved patches yet.");
                        } else {
                            let current = patch_mgr.canonical_save_name();
                            let filter = ui.data(|data| {
                                data.get_temp::<String>(filter_id)
                                    .map(|value| value.to_ascii_lowercase())
                                    .unwrap_or_default()
                            });
                            let mut selected_name = None;
                            egui::ScrollArea::vertical()
                                .max_height(PATCH_LOAD_POPUP_HEIGHT)
                                .show(ui, |ui| {
                                    for name in &patch_mgr.patch_names {
                                        if !filter.is_empty()
                                            && !name.to_ascii_lowercase().contains(&filter)
                                        {
                                            continue;
                                        }
                                        if ui
                                            .selectable_label(current == *name, name.as_str())
                                            .clicked()
                                        {
                                            selected_name = Some(name.clone());
                                        }
                                    }
                                });
                            if let Some(name) = selected_name {
                                ui.close();
                                load_patch_by_name(patch_mgr, control, state, &name, *muted);
                            } else if !filter.is_empty()
                                && !patch_mgr
                                    .patch_names
                                    .iter()
                                    .any(|name| name.to_ascii_lowercase().contains(&filter))
                            {
                                ui.label("No matching patches.");
                            }
                        }
                    });
                ui.data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new(PATCH_LOAD_WAS_OPEN_ID),
                        load_response.inner.is_some(),
                    );
                });
                if ui.button("Save").clicked() {
                    let name = patch_mgr.canonical_save_name();
                    if !name.is_empty() {
                        let patch = Patch::from(&*state);
                        patch_mgr.save_patch(&name, &patch);
                        patch_mgr.begin_loaded_patch(&name);
                        patch_mgr.refresh();
                    }
                }
                let send = ui.add_enabled(midi_output_port.is_some(), egui::Button::new("Send"));
                if send.clicked() {
                    let patch = Patch::from(&*state);
                    let sent = control.send_midi_patch(&patch)
                        || (control.set_midi_output_port(midi_output_port)
                            && control.send_midi_patch(&patch));
                    if !sent {
                        eprintln!("Failed to send Rev2 Program Edit Buffer");
                    }
                }
                send.on_hover_text(if midi_output_port.is_some() {
                    "Send the current program to the MIDI output as a Rev2 Program Edit Buffer"
                } else {
                    "Select a MIDI output device in Settings"
                });

                ui.separator();

                ui.label("BPM:");
                let mut bpm = state.bpm;
                let response = ui.add(
                    egui::DragValue::new(&mut bpm)
                        .range(30.0..=250.0)
                        .speed(0.5)
                        .fixed_decimals(0),
                );
                if response.changed() {
                    state.bpm = bpm;
                    control.set_param(ParamId::Bpm, bpm);
                }
                response.on_hover_text("Beats per minute (30–250)");

                ui.label("Div:");
                let divide_names = [
                    "1/2", "1/4", "1/8", "1/8h", "1/8s", "1/8t", "1/16", "1/16h", "1/16s", "1/16t",
                    "1/32", "1/32t", "1/64t",
                ];
                let current_label = divide_names
                    .get(state.clock_divide)
                    .copied()
                    .unwrap_or("1/4");
                egui::ComboBox::from_id_salt("clock_divide")
                    .width(52.0)
                    .selected_text(current_label)
                    .show_ui(ui, |ui| {
                        for (index, label) in divide_names.iter().enumerate() {
                            if ui
                                .selectable_label(state.clock_divide == index, *label)
                                .clicked()
                            {
                                state.clock_divide = index;
                                control.set_param(ParamId::ClockDivide, index as f32);
                                ui.close();
                            }
                        }
                    });
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.button("Analysis").clicked() {
                    *analysis_open = !*analysis_open;
                }

                ui.separator();

                let input_enabled = control.input_enabled();
                if framed_selectable(ui, input_enabled, "Audio In")
                    .on_hover_text("Toggle mixing the audio input into the output")
                    .clicked()
                {
                    control.set_input_enabled(!input_enabled);
                }

                ui.separator();

                egui::ComboBox::from_id_salt("play_pitch_class")
                    .selected_text(PLAY_PITCH_CLASSES[state.play_pitch_class as usize])
                    .width(40.0)
                    .show_ui(ui, |ui| {
                        for (index, label) in PLAY_PITCH_CLASSES.iter().enumerate() {
                            if ui
                                .selectable_label(state.play_pitch_class as usize == index, *label)
                                .clicked()
                            {
                                state.play_pitch_class = index as u8;
                                ui.close();
                            }
                        }
                    });
                egui::ComboBox::from_id_salt("play_octave")
                    .selected_text(state.play_octave.to_string())
                    .width(36.0)
                    .show_ui(ui, |ui| {
                        for octave in PLAY_OCTAVE_MIN..=PLAY_OCTAVE_MAX {
                            if ui
                                .selectable_label(state.play_octave == octave, octave.to_string())
                                .clicked()
                            {
                                state.play_octave = octave;
                                ui.close();
                            }
                        }
                    });
                if ui.button("Play").clicked() {
                    control.note_on(
                        play_midi_note(state.play_pitch_class, state.play_octave),
                        0.8,
                    );
                }

                if ui.button("Stop All").clicked() {
                    control.all_notes_off();
                }
            });
        });

        let height = left.response.rect.height();
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let row_width = 200.0;
                ui.add_space((ui.available_width() - row_width).max(0.0));

                let mut displayed_volume = state.master_volume;
                let volume_before_edit = displayed_volume;
                master_volume(ui, &mut displayed_volume, control, !*muted);
                if *muted {
                    if (displayed_volume - volume_before_edit).abs() > f32::EPSILON {
                        state.master_volume = displayed_volume;
                        control.set_param_audio_only(ParamId::MasterVolume, 0.0);
                    }
                } else {
                    state.master_volume = displayed_volume;
                }

                if framed_selectable(ui, *muted, "Mute").clicked() {
                    *muted = !*muted;
                    if *muted {
                        control.set_param_audio_only(ParamId::MasterVolume, 0.0);
                    } else {
                        control.set_param_audio_only(ParamId::MasterVolume, state.master_volume);
                    }
                }
            },
        );
    });
}

const PLAY_PITCH_CLASSES: &[&str] = &[
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const PLAY_OCTAVE_MIN: i8 = 0;
const PLAY_OCTAVE_MAX: i8 = 8;

fn play_midi_note(pitch_class: u8, octave: i8) -> u8 {
    let note = i32::from(octave + 1) * 12 + i32::from(pitch_class);
    note.clamp(0, 127) as u8
}

fn module_panel(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    module_panel_with_header(ui, title, |_| {}, add_contents);
}

fn module_panel_with_header(
    ui: &mut egui::Ui,
    title: &str,
    add_header_right: impl FnOnce(&mut egui::Ui),
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let horizontal_margin = 10;
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin {
            left: horizontal_margin,
            right: horizontal_margin,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(title).strong()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    add_header_right(ui);
                });
            });
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
            .num_columns(12)
            .spacing(egui::vec2(8.0, 10.0))
            .show(ui, |ui| {
                strong_label(ui, "OSC 1");
                control_cell(ui, |ui| {
                    if state.osc1_keyboard_on {
                        param_knob_f32_offset(
                            ui,
                            "Freq",
                            &mut state.osc1_freq,
                            0.0..=120.0,
                            60.0,
                            60.0,
                            ParamId::Osc1Frequency,
                            control,
                        );
                    } else {
                        param_knob_note(
                            ui,
                            "Freq",
                            &mut state.osc1_freq,
                            0.0..=120.0,
                            60.0,
                            ParamId::Osc1Frequency,
                            control,
                        );
                    }
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
                        "Shape Mod",
                        &mut state.osc1_shape_mod,
                        0.0..=1.0,
                        0.0,
                        ParamId::Osc1ShapeMod,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Glide",
                        &mut state.osc1_glide,
                        0.0..=1.0,
                        0.0,
                        ParamId::Osc1Glide,
                        control,
                    );
                });
                ui.allocate_ui_with_layout(
                    egui::vec2(32.0, CONTROL_CELL_H),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.vertical(|ui| {
                            param_toggle_sized(
                                ui,
                                WAVE_BUTTON_SIZE,
                                "On",
                                &mut state.osc1_enabled,
                                ParamId::Osc1Enabled,
                                control,
                            );
                            param_toggle_sized(
                                ui,
                                WAVE_BUTTON_SIZE,
                                "Key",
                                &mut state.osc1_keyboard_on,
                                ParamId::Osc1KeyboardOn,
                                control,
                            );
                            param_toggle_sized(
                                ui,
                                WAVE_BUTTON_SIZE,
                                "Reset",
                                &mut state.osc1_note_reset,
                                ParamId::Osc1NoteReset,
                                control,
                            );
                        });
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
                    egui::vec2(COMBO_COLUMN_W, CONTROL_CELL_H),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        param_toggle_sized(
                            ui,
                            COMBO_BUTTON_SIZE,
                            "Sync",
                            &mut state.sync,
                            ParamId::HardSync,
                            control,
                        );
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(GLIDE_COLUMN_W, GLIDE_CELL_H),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_max_width(GLIDE_COLUMN_W);
                        ui.vertical(|ui| {
                            param_toggle_sized(
                                ui,
                                GLIDE_BUTTON_SIZE,
                                "Glide",
                                &mut state.glide_enabled,
                                ParamId::GlideEnabled,
                                control,
                            );
                            ui.add_space(4.0);
                            let current = GlideMode::from_index(state.glide_mode);
                            egui::ComboBox::from_id_salt("glide_mode")
                                .width(GLIDE_CONTROL_W)
                                .truncate()
                                .selected_text(current.name())
                                .show_ui(ui, |ui| {
                                    for (index, mode) in GlideMode::ALL.iter().enumerate() {
                                        if ui
                                            .selectable_label(
                                                state.glide_mode == index,
                                                mode.name(),
                                            )
                                            .clicked()
                                        {
                                            state.glide_mode = index;
                                            control.set_param(ParamId::GlideMode, index as f32);
                                            ui.close();
                                        }
                                    }
                                });
                        });
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(UNISON_COLUMN_W, UNISON_CELL_H),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_max_width(UNISON_COLUMN_W);
                        ui.vertical(|ui| {
                            param_toggle_sized(
                                ui,
                                UNISON_BUTTON_SIZE,
                                "Unison",
                                &mut state.unison_enabled,
                                ParamId::UnisonEnabled,
                                control,
                            );
                            ui.add_space(4.0);
                            let current = UnisonMode::from_index(state.unison_mode);
                            egui::ComboBox::from_id_salt("unison_mode")
                                .width(UNISON_CONTROL_W)
                                .truncate()
                                .selected_text(current.name())
                                .show_ui(ui, |ui| {
                                    for (index, mode) in UnisonMode::ALL.iter().enumerate() {
                                        if ui
                                            .selectable_label(
                                                state.unison_mode == index,
                                                mode.name(),
                                            )
                                            .clicked()
                                        {
                                            state.unison_mode = index;
                                            control.set_param(ParamId::UnisonMode, index as f32);
                                            ui.close();
                                        }
                                    }
                                });
                        });
                    },
                );
                ui.end_row();

                strong_label(ui, "OSC 2");
                control_cell(ui, |ui| {
                    if state.osc2_keyboard_on {
                        param_knob_f32_offset(
                            ui,
                            "Freq",
                            &mut state.osc2_freq,
                            0.0..=120.0,
                            60.0,
                            60.0,
                            ParamId::Osc2Frequency,
                            control,
                        );
                    } else {
                        param_knob_note(
                            ui,
                            "Freq",
                            &mut state.osc2_freq,
                            0.0..=120.0,
                            60.0,
                            ParamId::Osc2Frequency,
                            control,
                        );
                    }
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
                        "Shape Mod",
                        &mut state.osc2_shape_mod,
                        0.0..=1.0,
                        0.0,
                        ParamId::Osc2ShapeMod,
                        control,
                    );
                });
                control_cell(ui, |ui| {
                    param_knob_f32(
                        ui,
                        "Glide",
                        &mut state.osc2_glide,
                        0.0..=1.0,
                        0.0,
                        ParamId::Osc2Glide,
                        control,
                    );
                });
                ui.allocate_ui_with_layout(
                    egui::vec2(32.0, CONTROL_CELL_H),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.vertical(|ui| {
                            param_toggle_sized(
                                ui,
                                WAVE_BUTTON_SIZE,
                                "On",
                                &mut state.osc2_enabled,
                                ParamId::Osc2Enabled,
                                control,
                            );
                            param_toggle_sized(
                                ui,
                                WAVE_BUTTON_SIZE,
                                "Key",
                                &mut state.osc2_keyboard_on,
                                ParamId::Osc2KeyboardOn,
                                control,
                            );
                            param_toggle_sized(
                                ui,
                                WAVE_BUTTON_SIZE,
                                "Reset",
                                &mut state.osc2_note_reset,
                                ParamId::Osc2NoteReset,
                                control,
                            );
                        });
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
                ui.allocate_ui_with_layout(
                    egui::vec2(COMBO_COLUMN_W, CONTROL_CELL_H),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_max_width(COMBO_COLUMN_W);
                        ui.label("Key Mode:");
                        let current = KeyMode::from_index(state.key_mode);
                        egui::ComboBox::from_id_salt("key_mode")
                            .width(COMBO_CONTROL_W)
                            .truncate()
                            .selected_text(current.name())
                            .show_ui(ui, |ui| {
                                for (index, mode) in KeyMode::ALL.iter().enumerate() {
                                    if ui
                                        .selectable_label(state.key_mode == index, mode.name())
                                        .clicked()
                                    {
                                        state.key_mode = index;
                                        control.set_param(ParamId::KeyMode, index as f32);
                                        ui.close();
                                    }
                                }
                            });
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(GLIDE_COLUMN_W, GLIDE_CELL_H),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        param_knob_f32(
                            ui,
                            "Glide",
                            &mut state.glide_time,
                            0.0..=1.0,
                            0.0,
                            ParamId::GlideTime,
                            control,
                        );
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(UNISON_COLUMN_W, CONTROL_CELL_H),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        param_knob_f32(
                            ui,
                            "Detune",
                            &mut state.unison_detune,
                            0.0..=1.0,
                            0.0,
                            ParamId::UnisonDetune,
                            control,
                        );
                    },
                );
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
                    let active =
                        state.lfo_depths[index] > 0.0 && state.lfo_destinations[index] != 0;
                    let mut button = egui::Button::selectable(selected, format!("{}", index + 1))
                        .frame_when_inactive(true);
                    if active && !selected {
                        button = button.fill(egui::Color32::from_rgb(0, 55, 90));
                    }
                    if ui.add_sized(LFO_INDEX_BUTTON_SIZE, button).clicked() {
                        state.selected_lfo = index;
                    }
                    ui.add_space(6.0);
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
                param_toggle_sized(
                    ui,
                    LFO_SYNC_BUTTON_SIZE,
                    "Clk Sync",
                    &mut state.lfo_clock_sync[index],
                    lfo_clock_sync_param(index),
                    control,
                );
                param_toggle_sized(
                    ui,
                    LFO_SYNC_BUTTON_SIZE,
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
            if framed_selectable_sized(
                ui,
                LFO_SHAPE_BUTTON_SIZE,
                state.lfo_waveforms[index] == waveform,
                *name,
            )
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
    let current = ModDestination::from_index(state.lfo_destinations[index]);
    egui::ComboBox::from_id_salt(("lfo_destination", index))
        .width(150.0)
        .selected_text(current.name())
        .show_ui(ui, |ui| {
            for destination in ModDestination::ALL {
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

fn modulation_matrix_panel(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    module_panel_with_header(
        ui,
        "Modulation Matrix",
        |ui| {
            let expanded_id = egui::Id::new(MOD_MATRIX_EXPANDED_ID);
            let expanded = ui.data(|data| data.get_temp::<bool>(expanded_id).unwrap_or(false));
            let chevron = if expanded { "▼" } else { "▶" };
            if ui
                .small_button(chevron)
                .on_hover_text(if expanded { "Collapse" } else { "Expand" })
                .clicked()
            {
                ui.data_mut(|data| {
                    let current = data.get_temp::<bool>(expanded_id).unwrap_or(false);
                    data.insert_temp(expanded_id, !current);
                });
            }
        },
        |ui| {
            let expanded = ui.data(|data| {
                data.get_temp::<bool>(egui::Id::new(MOD_MATRIX_EXPANDED_ID))
                    .unwrap_or(false)
            });
            if expanded {
                modulation_matrix_module_expanded(ui, state, control);
            } else {
                modulation_matrix_module_collapsed(ui, state, control);
            }
        },
    );
}

fn modulation_matrix_module_collapsed(
    ui: &mut egui::Ui,
    state: &mut UiState,
    control: &SynthEngineControl,
) {
    fixed_panel_scroll(ui, "mod_matrix_scroll", MOD_MATRIX_PANEL_WIDTH, |ui| {
        ui.horizontal(|ui| {
            for index in 0..8 {
                mod_route_button(ui, state, index, &(index + 1).to_string());
            }
            ui.separator();
            for (index, source) in DedicatedModSource::ALL.iter().enumerate() {
                mod_route_button(ui, state, 8 + index, source.name());
            }
        });

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let selected = state.selected_mod_route.min(12);
            state.selected_mod_route = selected;

            if selected < 8 {
                free_mod_route_row(ui, state, control, selected, false);
            } else {
                dedicated_mod_route_row(ui, state, control, selected - 8, false);
            }
        });
        ui.add_space(6.0);
    });
}

fn modulation_matrix_module_expanded(
    ui: &mut egui::Ui,
    state: &mut UiState,
    control: &SynthEngineControl,
) {
    fixed_panel_scroll(ui, "mod_matrix_scroll", MOD_MATRIX_PANEL_WIDTH, |ui| {
        for index in 0..8 {
            ui.horizontal(|ui| {
                free_mod_route_row(ui, state, control, index, true);
            });
            ui.add_space(4.0);
        }
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);
        for index in 0..DedicatedModSource::ALL.len() {
            ui.horizontal(|ui| {
                dedicated_mod_route_row(ui, state, control, index, true);
            });
            ui.add_space(4.0);
        }
        ui.add_space(6.0);
    });
}

fn mod_slot_label_button(ui: &mut egui::Ui, state: &mut UiState, index: usize) -> bool {
    let enabled = state.mod_enabled[index];
    let mut button = egui::Button::new((index + 1).to_string()).frame_when_inactive(true);
    if enabled {
        button = button.fill(egui::Color32::from_rgb(0, 55, 90));
    }
    if ui
        .add_sized(MOD_SLOT_BUTTON_SIZE, button)
        .on_hover_text("Toggle enabled")
        .clicked()
    {
        state.mod_enabled[index] = !state.mod_enabled[index];
        return true;
    }
    false
}

fn dedicated_slot_label_button(ui: &mut egui::Ui, state: &mut UiState, index: usize) -> bool {
    let enabled = state.dedicated_mod_enabled[index];
    let label = DedicatedModSource::ALL[index].name();
    let mut button = egui::Button::new(label).frame_when_inactive(true);
    if enabled {
        button = button.fill(egui::Color32::from_rgb(0, 55, 90));
    }
    if ui
        .add_sized(
            egui::vec2(MOD_DEDICATED_LABEL_WIDTH, MOD_SLOT_BUTTON_SIZE.y),
            button,
        )
        .on_hover_text("Toggle enabled")
        .clicked()
    {
        state.dedicated_mod_enabled[index] = !state.dedicated_mod_enabled[index];
        return true;
    }
    false
}

fn mod_route_button(ui: &mut egui::Ui, state: &mut UiState, route_index: usize, label: &str) {
    let selected = state.selected_mod_route == route_index;
    let enabled = if route_index < 8 {
        state.mod_enabled[route_index]
    } else {
        state.dedicated_mod_enabled[route_index - 8]
    };

    let mut button = egui::Button::selectable(selected, label).frame_when_inactive(true);
    if enabled && !selected {
        button = button.fill(egui::Color32::from_rgb(0, 55, 90));
    }

    let response = if route_index < 8 {
        ui.add_sized(MOD_SLOT_BUTTON_SIZE, button)
    } else {
        ui.add(button.min_size(egui::vec2(0.0, MOD_SLOT_BUTTON_SIZE.y)))
    };
    if response.clicked() {
        state.selected_mod_route = route_index;
    }
}

fn free_mod_route_row(
    ui: &mut egui::Ui,
    state: &mut UiState,
    control: &SynthEngineControl,
    index: usize,
    expanded: bool,
) {
    let mut changed = false;
    if expanded {
        changed |= mod_slot_label_button(ui, state, index);
        ui.add_space(8.0);
    } else if framed_selectable(ui, state.mod_enabled[index], "Enabled").clicked() {
        state.mod_enabled[index] = !state.mod_enabled[index];
        changed = true;
    }

    if !expanded {
        ui.add_space(8.0);
    }
    changed |= mod_source_combo(
        ui,
        ("mod_source", index),
        &mut state.mod_sources[index],
        true,
    );
    ui.add_space(8.0);
    changed |= mod_destination_combo(
        ui,
        ("mod_destination", index),
        &mut state.mod_destinations[index],
    );
    ui.add_space(8.0);
    changed |= mod_amount_control(ui, &mut state.mod_amounts[index]);

    if changed {
        send_free_mod_route(control, state, index);
    }
}

fn dedicated_mod_route_row(
    ui: &mut egui::Ui,
    state: &mut UiState,
    control: &SynthEngineControl,
    index: usize,
    expanded: bool,
) {
    let mut changed = false;
    if expanded {
        changed |= dedicated_slot_label_button(ui, state, index);
        ui.add_space(8.0);
    } else if framed_selectable(ui, state.dedicated_mod_enabled[index], "Enabled").clicked() {
        state.dedicated_mod_enabled[index] = !state.dedicated_mod_enabled[index];
        changed = true;
    }

    if !expanded {
        ui.add_space(8.0);
    }
    fixed_mod_source_field(ui, DedicatedModSource::ALL[index].name());
    ui.add_space(8.0);
    changed |= mod_destination_combo(
        ui,
        ("dedicated_mod_destination", index),
        &mut state.dedicated_mod_destinations[index],
    );
    ui.add_space(8.0);
    changed |= mod_amount_control(ui, &mut state.dedicated_mod_amounts[index]);

    if changed {
        send_dedicated_mod_route(control, state, index);
    }
}

fn mod_source_combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    source_index: &mut usize,
    enabled: bool,
) -> bool {
    let current = ModSource::from_index(*source_index);
    let mut changed = false;
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(id)
            .width(150.0)
            .selected_text(current.name())
            .show_ui(ui, |ui| {
                for source in ModSource::ALL {
                    let index = source.index();
                    if ui
                        .selectable_label(*source_index == index, source.name())
                        .clicked()
                    {
                        *source_index = index;
                        changed = true;
                        ui.close();
                    }
                }
            });
    });
    changed
}

fn fixed_mod_source_field(ui: &mut egui::Ui, label: &str) {
    ui.add_enabled_ui(false, |ui| {
        egui::ComboBox::from_id_salt(("dedicated_mod_source", label))
            .width(150.0)
            .selected_text(label)
            .show_ui(ui, |_| {});
    });
}

fn mod_destination_combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    destination_index: &mut usize,
) -> bool {
    let current = ModDestination::from_index(*destination_index);
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .width(180.0)
        .selected_text(current.name())
        .show_ui(ui, |ui| {
            for destination in ModDestination::ALL {
                let index = destination.index();
                if ui
                    .selectable_label(*destination_index == index, destination.name())
                    .clicked()
                {
                    *destination_index = index;
                    changed = true;
                    ui.close();
                }
            }
        });
    changed
}

fn mod_amount_control(ui: &mut egui::Ui, amount: &mut f32) -> bool {
    ui.horizontal(|ui| {
        ui.label("Amount");
        ui.add(
            egui::DragValue::new(amount)
                .range(-1.0..=1.0)
                .speed(0.01)
                .fixed_decimals(2),
        )
        .changed()
    })
    .inner
}

fn send_free_mod_route(control: &SynthEngineControl, state: &UiState, index: usize) {
    control.set_modulation(
        ModRoute::Free(index),
        state.mod_enabled[index],
        ModSource::from_index(state.mod_sources[index]),
        ModDestination::from_index(state.mod_destinations[index]),
        state.mod_amounts[index],
    );
}

fn send_dedicated_mod_route(control: &SynthEngineControl, state: &UiState, index: usize) {
    let source = DedicatedModSource::ALL[index];
    control.set_modulation(
        ModRoute::Dedicated(source),
        state.dedicated_mod_enabled[index],
        source.source(),
        ModDestination::from_index(state.dedicated_mod_destinations[index]),
        state.dedicated_mod_amounts[index],
    );
}

fn effects_module(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    fixed_panel_scroll(ui, "effects_scroll", EFFECTS_PANEL_WIDTH, |ui| {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(70.0, CONTROL_CELL_H),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.add_space(18.0);
                    param_toggle(
                        ui,
                        "On",
                        &mut state.effect_enabled,
                        ParamId::EffectEnabled,
                        control,
                    );
                },
            );

            ui.add_space(8.0);
            effect_type_selector(ui, state, control);
            ui.add_space(8.0);

            control_cell(ui, |ui| {
                param_knob_f32(
                    ui,
                    "Mix",
                    &mut state.effect_mix,
                    0.0..=1.0,
                    0.0,
                    ParamId::EffectMix,
                    control,
                );
            });

            ui.allocate_ui_with_layout(
                egui::vec2(74.0, CONTROL_CELL_H),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.add_space(18.0);
                    let active = EffectType::from_index(state.effect_type).is_delay();
                    ui.add_enabled_ui(active, |ui| {
                        param_toggle(
                            ui,
                            "Clk Sync",
                            &mut state.effect_clock_sync,
                            ParamId::EffectClockSync,
                            control,
                        );
                    });
                    if !active && state.effect_clock_sync {
                        state.effect_clock_sync = false;
                        control.set_param(ParamId::EffectClockSync, 0.0);
                    }
                },
            );

            let (param1_label, param2_label) =
                effect_param_labels(EffectType::from_index(state.effect_type));
            control_cell(ui, |ui| {
                param_knob_f32(
                    ui,
                    param1_label,
                    &mut state.effect_param1,
                    0.0..=1.0,
                    0.25,
                    ParamId::EffectParam1,
                    control,
                );
            });
            control_cell(ui, |ui| {
                param_knob_f32(
                    ui,
                    param2_label,
                    &mut state.effect_param2,
                    0.0..=1.0,
                    0.25,
                    ParamId::EffectParam2,
                    control,
                );
            });
        });
    });
    state.store_active_effect_params();
}

fn effect_type_selector(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    ui.allocate_ui_with_layout(
        egui::vec2(140.0, CONTROL_CELL_H),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.add_space(8.0);
            let current = EffectType::from_index(state.effect_type);
            egui::ComboBox::from_id_salt("effect_type")
                .width(132.0)
                .selected_text(current.name())
                .show_ui(ui, |ui| {
                    for effect in EffectType::ALL {
                        let index = effect.index();
                        if ui
                            .selectable_label(state.effect_type == index, effect.name())
                            .clicked()
                        {
                            state.select_effect(index);
                            control.set_param(ParamId::EffectType, index as f32);
                            control.set_param(ParamId::EffectMix, state.effect_mix);
                            control.set_param(
                                ParamId::EffectClockSync,
                                f32::from(state.effect_clock_sync),
                            );
                            control.set_param(ParamId::EffectParam1, state.effect_param1);
                            control.set_param(ParamId::EffectParam2, state.effect_param2);
                            ui.close();
                        }
                    }
                });
            ui.add_space(4.0);
            ui.label("Type");
        },
    );
}

fn effect_param_labels(effect: EffectType) -> (&'static str, &'static str) {
    match effect {
        EffectType::DelayMono | EffectType::DdlStereo | EffectType::BucketBrigadeDelay => {
            ("Delay", "Feedback")
        }
        EffectType::Chorus
        | EffectType::PhaserHigh
        | EffectType::PhaserLow
        | EffectType::PhaserMst
        | EffectType::Flanger1
        | EffectType::Flanger2 => ("Rate", "Depth"),
        EffectType::Reverb => ("Time", "Color"),
        EffectType::RingMod => ("Tuning", "Tracking"),
        EffectType::Distortion => ("Gain", "Tone"),
        EffectType::HighPassFilter => ("Cutoff", "Res"),
    }
}

fn filter_model_combo(
    ui: &mut egui::Ui,
    filter_type: &mut FilterType,
    control: &SynthEngineControl,
) {
    egui::ComboBox::from_id_salt("filter_model")
        .selected_text(filter_type.name())
        .show_ui(ui, |ui| {
            for candidate in FilterType::ALL {
                let response = ui
                    .add_enabled(
                        candidate.is_implemented(),
                        egui::Button::selectable(*filter_type == candidate, candidate.name())
                            .frame_when_inactive(true),
                    )
                    .on_disabled_hover_text("Implemented in a later experiment phase");
                if response.clicked() {
                    *filter_type = candidate;
                    control.set_filter_type(candidate);
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
            .num_columns(5)
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
                        "VCA Level",
                        &mut state.amp_vca_initial_level,
                        0.0..=1.0,
                        0.0,
                        ParamId::VcaInitialLevel,
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
                ui.allocate_ui_with_layout(
                    egui::vec2(82.0, CONTROL_CELL_H),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.label(egui::RichText::new("Pan Mode").strong());
                        ui.add_space(4.0);
                        egui::ComboBox::from_id_salt("amp_pan_mod_mode")
                            .width(76.0)
                            .selected_text(match state.amp_pan_mod_mode {
                                PanModMode::Alternate => "Alternate",
                                PanModMode::Fixed => "Fixed",
                            })
                            .show_ui(ui, |ui| {
                                for (mode, label) in [
                                    (PanModMode::Alternate, "Alternate"),
                                    (PanModMode::Fixed, "Fixed"),
                                ] {
                                    if ui
                                        .selectable_value(&mut state.amp_pan_mod_mode, mode, label)
                                        .clicked()
                                    {
                                        control.set_param(ParamId::PanModMode, mode.as_param());
                                    }
                                }
                            });
                    },
                );
                ui.end_row();
            });
    });
}

fn auxiliary_envelope_module(ui: &mut egui::Ui, state: &mut UiState, control: &SynthEngineControl) {
    fixed_panel_scroll(ui, "aux_envelope_grid_scroll", AUX_GRID_WIDTH, |ui| {
        egui::Grid::new("aux_envelope_grid")
            .num_columns(5)
            .spacing(egui::vec2(12.0, 12.0))
            .show(ui, |ui| {
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
                ui.allocate_ui_with_layout(
                    egui::vec2(CONTROL_CELL_W, CONTROL_CELL_H),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), KNOB_SIZE),
                            egui::Layout::centered_and_justified(egui::Direction::TopDown),
                            |ui| {
                                let font_size =
                                    ui.style().text_styles[&egui::TextStyle::Button].size - 1.0;
                                if ui
                                    .add(
                                        egui::Button::selectable(
                                            state.aux_repeat,
                                            egui::RichText::new("Repeat").size(font_size),
                                        )
                                        .frame_when_inactive(true)
                                        .truncate(),
                                    )
                                    .clicked()
                                {
                                    state.aux_repeat = !state.aux_repeat;
                                    control.set_param(
                                        ParamId::AuxEgLoop,
                                        if state.aux_repeat { 1.0 } else { 0.0 },
                                    );
                                }
                            },
                        );
                    },
                );
                aux_destination_cell(ui, state, control);
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
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.label(egui::RichText::new("Destination").strong());
            let current = ModDestination::from_index(state.aux_destination);
            egui::ComboBox::from_id_salt("aux_destination")
                .width(104.0)
                .selected_text(current.name())
                .show_ui(ui, |ui| {
                    for destination in ModDestination::ALL {
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
        },
    );
}

fn pole_toggle(ui: &mut egui::Ui, filter_poles: &mut usize, control: &SynthEngineControl) {
    if framed_selectable(ui, *filter_poles == 1, "4 Pole").clicked() {
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
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
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
            egui::Grid::new(ui.id().with("wave_selector"))
                .num_columns(2)
                .spacing(egui::vec2(4.0, 4.0))
                .show(ui, |ui| {
                    for (index, name) in ["Saw", "Saw+Tri", "Triangle", "Pulse"].iter().enumerate()
                    {
                        if framed_selectable_sized(ui, WAVE_BUTTON_SIZE, *waveform == index, *name)
                            .clicked()
                        {
                            *waveform = index;
                            *enabled = true;
                            control.set_param(enabled_param, 1.0);
                            control.set_param(waveform_param, index as f32);
                        }
                        if index % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
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

#[derive(serde::Serialize, serde::Deserialize)]
struct AutosaveFile {
    patch: Patch,
    #[serde(default)]
    loaded_name: String,
    #[serde(default)]
    baseline: Option<Patch>,
    #[serde(default)]
    save_name: String,
}

pub struct PatchManager {
    pub save_name: String,
    loaded_name: String,
    baseline: Option<Patch>,
    baseline_pending: bool,
    user_modified: bool,
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
            loaded_name: String::new(),
            baseline: None,
            baseline_pending: false,
            user_modified: false,
            patch_names,
            config_dir,
            patches_dir,
        }
    }

    pub fn restore_autosave_metadata(
        &mut self,
        loaded_name: String,
        baseline: Option<Patch>,
        save_name: String,
        restored_patch: &Patch,
    ) {
        self.loaded_name = loaded_name;
        self.baseline = baseline;
        self.baseline_pending = false;
        if !save_name.is_empty() {
            self.save_name = save_name;
        } else if !self.loaded_name.is_empty() {
            self.save_name = self.loaded_name.clone();
        }

        let was_modified = self
            .baseline
            .as_ref()
            .is_some_and(|baseline| !patches_equal(baseline, restored_patch));
        self.user_modified = was_modified;
        if !was_modified && !self.loaded_name.is_empty() {
            self.baseline = None;
            self.baseline_pending = true;
        }
    }

    pub fn mark_user_modified(&mut self) {
        if !self.loaded_name.is_empty() {
            self.user_modified = true;
        }
    }

    pub fn begin_loaded_patch(&mut self, name: &str) {
        self.loaded_name = name.to_string();
        self.save_name = name.to_string();
        self.user_modified = false;
        self.baseline_pending = true;
    }

    pub fn finalize_loaded_patch(&mut self, state: &UiState) -> bool {
        if self.baseline_pending {
            self.baseline = Some(Patch::from(state));
            self.baseline_pending = false;
            self.user_modified = false;
            return true;
        }
        false
    }

    pub fn canonical_save_name(&self) -> String {
        strip_modified_suffix(self.save_name.trim()).to_string()
    }

    pub fn sync_display_name(&mut self, name_focused: bool) {
        if self.loaded_name.is_empty() || name_focused || self.baseline_pending {
            return;
        }
        self.save_name = if self.user_modified {
            format!("{}{}", self.loaded_name, MODIFIED_SUFFIX)
        } else {
            self.loaded_name.clone()
        };
    }

    pub fn save_patch(&self, name: &str, patch: &Patch) {
        let path = self.patches_dir.join(format!("{name}.json"));
        if let Ok(json) = serde_json::to_string_pretty(patch) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub fn save_midi_program(
        &self,
        program: &synth_core::MidiProgramImport,
    ) -> std::io::Result<PathBuf> {
        let name = match program {
            synth_core::MidiProgramImport::Rev2(program) => {
                rev2_program_filename(program.bank, program.program, program.patch.name.as_str())
            }
            synth_core::MidiProgramImport::P08(program) => {
                p08_program_filename(program.bank, program.program, program.patch.name.as_str())
            }
        }
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "MIDI program bank or number is outside the factory library",
            )
        })?;
        let path = self.patches_dir.join(format!("{name}.json"));
        let json = serde_json::to_string_pretty(program.patch()).map_err(std::io::Error::other)?;
        std::fs::write(&path, json)?;
        Ok(path)
    }

    pub fn save_autosave(&self, patch: &Patch) {
        let path = self.config_dir.join("patch.json");
        let file = AutosaveFile {
            patch: patch.clone(),
            loaded_name: self.loaded_name.clone(),
            baseline: self.baseline.clone(),
            save_name: self.save_name.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub fn load_autosave(&self) -> Option<(Patch, String, Option<Patch>, String)> {
        let path = self.config_dir.join("patch.json");
        let contents = std::fs::read_to_string(&path).ok()?;
        if let Ok(file) = serde_json::from_str::<AutosaveFile>(&contents) {
            return Some((file.patch, file.loaded_name, file.baseline, file.save_name));
        }
        let patch = serde_json::from_str(&contents).ok()?;
        Some((patch, String::new(), None, String::new()))
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

    pub fn adjacent_patch_name(&self, delta: isize) -> Option<&str> {
        if self.patch_names.is_empty() {
            return None;
        }
        let current = self.loaded_name.trim();
        let idx = self
            .patch_names
            .iter()
            .position(|n| n == current)
            .unwrap_or(0);
        let len = self.patch_names.len() as isize;
        let next_idx = (idx as isize + delta).rem_euclid(len) as usize;
        Some(self.patch_names[next_idx].as_str())
    }
}

fn rev2_program_filename(bank: u8, program: u8, patch_name: &str) -> Option<String> {
    if bank > 7 || program > 127 {
        return None;
    }
    let (bank_kind, bank_number) = if bank < 4 {
        ('F', bank + 1)
    } else {
        ('U', bank - 3)
    };
    let name = midi_import_filename(patch_name);
    Some(format!(
        "{bank_kind}{bank_number}-{:03}-{name}",
        program + 1
    ))
}

fn p08_program_filename(bank: u8, program: u8, patch_name: &str) -> Option<String> {
    if bank > 1 || program > 127 {
        return None;
    }
    let name = midi_import_filename(patch_name);
    Some(format!("F{}-{:03}-{name}", bank + 5, program + 1))
}

fn midi_import_filename(patch_name: &str) -> String {
    if patch_name.is_empty() {
        "Patch".to_string()
    } else {
        sanitize_filename(patch_name)
    }
}

fn sanitize_filename(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    let mut separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            output.push(character);
            separator = false;
        } else if !separator && !output.is_empty() {
            output.push('_');
            separator = true;
        }
    }
    while output.ends_with('_') {
        output.pop();
    }
    output
}

fn user_edited_patch(ui: &egui::Ui, before: &Patch, after: &Patch) -> bool {
    if patches_equal(before, after) {
        return false;
    }
    ui.input(|input| input.pointer.is_decidedly_dragging() || input.pointer.any_click())
}

fn strip_modified_suffix(name: &str) -> &str {
    name.strip_suffix(MODIFIED_SUFFIX).unwrap_or(name)
}

fn patches_equal(a: &Patch, b: &Patch) -> bool {
    serde_json::to_string(a).ok() == serde_json::to_string(b).ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_parameter_updates_visible_ui_state() {
        let mut state = UiState::default();
        state.apply_midi_update(MidiUiUpdate::Param(ParamId::FilterCutoff, 440.0));
        state.apply_midi_update(MidiUiUpdate::Param(ParamId::Osc2Enabled, 1.0));
        state.apply_midi_update(MidiUiUpdate::Param(ParamId::EffectType, 3.0));
        state.apply_midi_update(MidiUiUpdate::Param(ParamId::EffectMix, 0.75));

        assert_eq!(state.filter_cutoff, 440.0);
        assert!(state.osc2_enabled);
        assert_eq!(state.effect_type, 3);
        assert_eq!(state.effect_mix, 0.75);
        assert_eq!(state.effect_runtime_params[3].mix, 0.75);
    }

    #[test]
    fn midi_modulation_fields_update_and_enable_ui_routes() {
        let mut state = UiState::default();
        state.apply_midi_update(MidiUiUpdate::Modulation {
            route: ModRoute::Free(2),
            parameter: ModulationParam::Source(ModSource::Lfo2),
        });
        assert!(!state.mod_enabled[2]);

        state.apply_midi_update(MidiUiUpdate::Modulation {
            route: ModRoute::Free(2),
            parameter: ModulationParam::Destination(ModDestination::FilterCutoff),
        });
        state.apply_midi_update(MidiUiUpdate::Modulation {
            route: ModRoute::Free(2),
            parameter: ModulationParam::Amount(-0.5),
        });

        assert!(state.mod_enabled[2]);
        assert_eq!(state.mod_sources[2], ModSource::Lfo2.index());
        assert_eq!(
            state.mod_destinations[2],
            ModDestination::FilterCutoff.index()
        );
        assert_eq!(state.mod_amounts[2], -0.5);
    }

    fn test_patch_manager(names: &[&str], loaded_name: &str) -> PatchManager {
        PatchManager {
            save_name: loaded_name.to_string(),
            loaded_name: loaded_name.to_string(),
            baseline: None,
            baseline_pending: false,
            user_modified: false,
            patch_names: names.iter().map(|name| (*name).to_string()).collect(),
            config_dir: PathBuf::new(),
            patches_dir: PathBuf::new(),
        }
    }

    #[test]
    fn adjacent_patch_name_empty_list() {
        let mgr = test_patch_manager(&[], "foo");
        assert_eq!(mgr.adjacent_patch_name(-1), None);
        assert_eq!(mgr.adjacent_patch_name(1), None);
    }

    #[test]
    fn adjacent_patch_name_single_patch() {
        let mgr = test_patch_manager(&["only"], "only");
        assert_eq!(mgr.adjacent_patch_name(-1), Some("only"));
        assert_eq!(mgr.adjacent_patch_name(1), Some("only"));
    }

    #[test]
    fn adjacent_patch_name_multi_patch() {
        let mgr = test_patch_manager(&["a", "b", "c"], "b");
        assert_eq!(mgr.adjacent_patch_name(-1), Some("a"));
        assert_eq!(mgr.adjacent_patch_name(1), Some("c"));
    }

    #[test]
    fn adjacent_patch_name_wraps() {
        let mut mgr = test_patch_manager(&["a", "b", "c"], "a");
        assert_eq!(mgr.adjacent_patch_name(-1), Some("c"));
        mgr.loaded_name = "c".to_string();
        assert_eq!(mgr.adjacent_patch_name(1), Some("a"));
    }

    #[test]
    fn adjacent_patch_name_unknown_save_name() {
        let mgr = test_patch_manager(&["a", "b", "c"], "unknown");
        assert_eq!(mgr.adjacent_patch_name(-1), Some("c"));
        assert_eq!(mgr.adjacent_patch_name(1), Some("b"));
    }

    #[test]
    fn sync_display_name_appends_modified_suffix() {
        let mut mgr = test_patch_manager(&["a"], "a");
        let state = UiState::default();
        mgr.begin_loaded_patch("a");
        mgr.finalize_loaded_patch(&state);
        mgr.sync_display_name(false);
        assert_eq!(mgr.save_name, "a");

        mgr.mark_user_modified();
        mgr.sync_display_name(false);
        assert_eq!(mgr.save_name, "a (modified)");
    }

    #[test]
    fn loaded_patch_baseline_survives_effect_ui_normalization() {
        let mut state = UiState::default();
        let mut patch = Patch::default();
        patch.effects.enabled = true;
        patch.effects.effect_type = EffectType::Chorus;
        patch.effects.clock_sync = true;
        state.apply_from_patch(&patch);

        let mut mgr = test_patch_manager(&["a"], "a");
        mgr.begin_loaded_patch("a");
        mgr.finalize_loaded_patch(&state);
        mgr.sync_display_name(false);
        assert_eq!(mgr.save_name, "a");
    }

    #[test]
    fn canonical_save_name_strips_modified_suffix() {
        let mgr = test_patch_manager(&[], "My Patch (modified)");
        assert_eq!(mgr.canonical_save_name(), "My Patch");
    }

    #[test]
    fn loaded_patch_not_modified_after_finalize() {
        let mut state = UiState::default();
        let mut patch = Patch::default();
        patch.filter.cutoff = 4_500.0;
        patch.effects.enabled = true;
        patch.effects.effect_type = EffectType::Chorus;
        patch.effects.clock_sync = true;
        state.apply_from_patch(&patch);

        let mut mgr = test_patch_manager(&["a"], "a");
        mgr.begin_loaded_patch("a");
        mgr.finalize_loaded_patch(&state);
        mgr.sync_display_name(false);
        assert_eq!(mgr.save_name, "a");
    }

    #[test]
    fn restore_unmodified_autosave_refreshes_baseline() {
        let mut state = UiState::default();
        let mut patch = Patch::default();
        patch.filter.cutoff = 4_500.0;
        state.apply_from_patch(&patch);
        let snapshot = Patch::from(&state);

        let mut mgr = test_patch_manager(&["a"], "a");
        mgr.restore_autosave_metadata(
            "a".to_string(),
            Some(snapshot.clone()),
            "a".to_string(),
            &snapshot,
        );
        assert!(mgr.baseline_pending);
        mgr.finalize_loaded_patch(&state);
        mgr.sync_display_name(false);
        assert_eq!(mgr.save_name, "a");
    }

    #[test]
    fn autosave_roundtrip_preserves_patch_name_and_metadata() {
        let root =
            std::env::temp_dir().join(format!("analog-synth-autosave-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let manager = PatchManager {
            save_name: "preset (modified)".to_string(),
            loaded_name: "preset".to_string(),
            baseline: Some(Patch::default()),
            baseline_pending: false,
            user_modified: false,
            patch_names: Vec::new(),
            config_dir: root.clone(),
            patches_dir: root.join("patches"),
        };
        let mut patch = Patch::default();
        patch.master_volume = 0.25;
        manager.save_autosave(&patch);
        let (loaded_patch, loaded_name, baseline, save_name) = manager.load_autosave().unwrap();
        assert_eq!(loaded_name, "preset");
        assert_eq!(save_name, "preset (modified)");
        assert_eq!(loaded_patch.master_volume, 0.25);
        assert!(baseline.is_some());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn midi_program_save_uses_deterministic_overwriteable_json() {
        let root =
            std::env::temp_dir().join(format!("analog-synth-midi-import-{}", std::process::id()));
        let patches_dir = root.join("patches");
        std::fs::create_dir_all(&patches_dir).unwrap();
        let manager = PatchManager {
            save_name: String::new(),
            loaded_name: String::new(),
            baseline: None,
            baseline_pending: false,
            user_modified: false,
            patch_names: Vec::new(),
            config_dir: root.clone(),
            patches_dir,
        };
        let mut program = synth_core::MidiProgramImport::Rev2(synth_core::Rev2ProgramData {
            bank: 4,
            program: 0,
            patch: {
                let mut patch = Patch::default();
                patch.name.push_str("LosVangelis2041").unwrap();
                patch
            },
        });
        let path = manager.save_midi_program(&program).unwrap();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("U1-001-LosVangelis2041.json")
        );
        if let synth_core::MidiProgramImport::Rev2(program) = &mut program {
            program.patch.master_volume = 0.25;
        }
        manager.save_midi_program(&program).unwrap();
        let decoded: Patch =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(decoded.master_volume, 0.25);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn p08_midi_program_save_uses_embedded_patch_name() {
        let root =
            std::env::temp_dir().join(format!("analog-synth-p08-import-{}", std::process::id()));
        let patches_dir = root.join("patches");
        std::fs::create_dir_all(&patches_dir).unwrap();
        let manager = PatchManager {
            save_name: String::new(),
            loaded_name: String::new(),
            baseline: None,
            baseline_pending: false,
            user_modified: false,
            patch_names: Vec::new(),
            config_dir: root.clone(),
            patches_dir,
        };
        let program = synth_core::MidiProgramImport::P08(synth_core::P08ProgramData {
            bank: 0,
            program: 0,
            patch: {
                let mut patch = Patch::default();
                patch.name.push_str("Wagnerian").unwrap();
                patch
            },
        });
        let path = manager.save_midi_program(&program).unwrap();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("F5-001-Wagnerian.json")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
