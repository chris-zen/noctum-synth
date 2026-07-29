//! Top-level synthesis engine and audio render entry point.

use crate::EffectType;
#[cfg(feature = "profiling")]
use crate::RenderProfiler;
use crate::dsp::lookahead_limiter::LookaheadLimiter;
use crate::dsp::{FilterOversampling, FilterType};
use crate::midi::clock::MidiClockFollower;
use crate::midi::clock::{MidiClockMode, MidiClockStatus, MidiRealtimeEvent};
use crate::profiling::{RenderContext, RenderStage};
use crate::rate_adapter::RateAdapter;
use crate::voice::{LayerEngine, VoicePool, VoiceRegion};
use crate::{
    ActiveNotes, ClockDivision, ControlMessage, LayerId, LayerMode, LayerTarget, ModDestination,
    ModRoute, ModSource, ParamId, Patch, VOICE_PACKS,
};

/// Fixed headroom between the polyphonic voice sum and global effects.
///
/// This models the Prophet's calibrated voice/output summing gain without
/// changing gain dynamically with the number of active voices.
const MIX_BUS_GAIN: f32 = 0.55;
const TOPOLOGY_FADE_SAMPLES: usize = 256;

/// Synthesis engine with inline effects storage.
pub type SynthEngine<const PACKS: usize = VOICE_PACKS, const FX_SAMPLES: usize = 48_000> =
    SynthEngineWithMemory<[f32; FX_SAMPLES], PACKS>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineInitError {
    UnsupportedLayerCount,
    EmptyEffectsMemory,
    UnevenLayerMemory,
    InvalidStereoMemory,
    UnevenVoiceRegions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerPlaybackStatus {
    pub mode: LayerMode,
    pub edit_layer: LayerId,
    pub rendered_mask: u8,
    pub degraded: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransitionState {
    Idle,
    FadeOut,
    FadeIn,
}

struct TopologyTransition {
    state: TransitionState,
    gain: f32,
}

impl Default for TopologyTransition {
    fn default() -> Self {
        Self {
            state: TransitionState::Idle,
            gain: 1.0,
        }
    }
}

impl TopologyTransition {
    fn request(&mut self) {
        self.state = TransitionState::FadeOut;
    }

    fn advance(&mut self) -> bool {
        let step = 1.0 / TOPOLOGY_FADE_SAMPLES as f32;
        match self.state {
            TransitionState::Idle => self.gain = 1.0,
            TransitionState::FadeOut => {
                self.gain = (self.gain - step).max(0.0);
                if self.gain == 0.0 {
                    return true;
                }
            }
            TransitionState::FadeIn => {
                self.gain = (self.gain + step).min(1.0);
                if self.gain == 1.0 {
                    self.state = TransitionState::Idle;
                }
            }
        }
        false
    }

    fn begin_fade_in(&mut self) {
        self.gain = 0.0;
        self.state = TransitionState::FadeIn;
    }
}

/// Owns all voices and renders stereo audio from [`ControlMessage`] input.
///
/// Construct with [`SynthEngine::new`], feed control messages from the host
/// thread, then call [`SynthEngine::process`] or
/// [`SynthEngine::process_interleaved`] on the audio thread.
pub struct SynthEngineWithMemory<Memory, const PACKS: usize, const LAYERS: usize = 1> {
    effects_memory: Memory,
    voice_pool: VoicePool<PACKS>,
    layers: [LayerEngine<PACKS>; LAYERS],
    patch: Patch,
    edit_layer: LayerId,
    applied_mode: LayerMode,
    applied_split_point: u8,
    applied_edit_layer: LayerId,
    route_mask: [u8; 128],
    physical_keys: crate::pressed_keys::PressedKeys,
    sustain_pressed: bool,
    transition: TopologyTransition,
    filter_oversampling: FilterOversampling,
    filter_type: FilterType,
    midi_clock: MidiClockFollower,
    output_limiter: LookaheadLimiter,
    rate_adapter: RateAdapter,
}

impl<const PACKS: usize, const FX_SAMPLES: usize>
    SynthEngineWithMemory<[f32; FX_SAMPLES], PACKS, 1>
{
    const VALID_INLINE_MEMORY: () = assert!(
        FX_SAMPLES > 0 && FX_SAMPLES % 2 == 0,
        "inline effects memory must contain equal stereo halves"
    );

    /// Creates an engine at `sample_rate` with inline effects storage.
    pub fn new(sample_rate: f32) -> Self {
        let () = Self::VALID_INLINE_MEMORY;
        Self::build(sample_rate, [0.0; FX_SAMPLES])
    }
}

impl<Memory, const PACKS: usize, const LAYERS: usize> SynthEngineWithMemory<Memory, PACKS, LAYERS>
where
    Memory: AsRef<[f32]> + AsMut<[f32]>,
{
    /// Creates an engine using caller-provided effects memory.
    pub fn new_with_effects_memory(
        sample_rate: f32,
        mut effects_memory: Memory,
    ) -> Result<Self, EngineInitError> {
        let memory_len = effects_memory.as_ref().len();
        if LAYERS != 1 && LAYERS != 2 {
            return Err(EngineInitError::UnsupportedLayerCount);
        }
        if memory_len == 0 {
            return Err(EngineInitError::EmptyEffectsMemory);
        }
        if memory_len % LAYERS != 0 {
            return Err(EngineInitError::UnevenLayerMemory);
        }
        if (memory_len / LAYERS) % 2 != 0 {
            return Err(EngineInitError::InvalidStereoMemory);
        }
        if LAYERS == 2 && (PACKS < 2 || PACKS % 2 != 0) {
            return Err(EngineInitError::UnevenVoiceRegions);
        }
        effects_memory.as_mut().fill(0.0);
        Ok(Self::build(sample_rate, effects_memory))
    }

    fn build(sample_rate: f32, effects_memory: Memory) -> Self {
        let internal_sample_rate = RateAdapter::internal_sample_rate(sample_rate);
        Self {
            effects_memory,
            voice_pool: VoicePool::<PACKS>::new(internal_sample_rate),
            layers: core::array::from_fn(|_| LayerEngine::<PACKS>::new(internal_sample_rate)),
            patch: Patch::default(),
            edit_layer: LayerId::A,
            applied_mode: LayerMode::Normal,
            applied_split_point: crate::DEFAULT_SPLIT_POINT,
            applied_edit_layer: LayerId::A,
            route_mask: [0; 128],
            physical_keys: crate::pressed_keys::PressedKeys::default(),
            sustain_pressed: false,
            transition: TopologyTransition::default(),
            filter_oversampling: FilterOversampling::default(),
            filter_type: FilterType::default(),
            midi_clock: MidiClockFollower::new(sample_rate),
            output_limiter: LookaheadLimiter::new(internal_sample_rate),
            rate_adapter: RateAdapter::default(),
        }
    }

    /// Applies a single control or performance message.
    pub fn handle_control(&mut self, msg: ControlMessage) {
        match msg {
            ControlMessage::SetParam {
                target,
                param,
                value,
            } => self.set_target_param(target, param, value),
            ControlMessage::SetUnisonChord { target, chord } => {
                let layer_id = self.resolve_target(target);
                self.patch.layer_mut(layer_id).unison_chord = chord;
                if let Some(index) = self.active_engine_index(layer_id) {
                    self.layers[index].handle_control(
                        &mut self.voice_pool,
                        ControlMessage::SetUnisonChord {
                            target: LayerTarget::Explicit(layer_id),
                            chord,
                        },
                    );
                }
            }
            ControlMessage::SetTempoBpm { target, bpm } => {
                self.set_target_param(target, ParamId::Bpm, bpm)
            }
            ControlMessage::SetMidiClockMode(mode) => self.set_midi_clock_mode(mode),
            ControlMessage::MidiRealtime(event) => self.handle_midi_realtime(event),
            ControlMessage::SetModulation {
                target,
                route,
                enabled,
                source,
                destination,
                amount,
            } => {
                let layer_id = self.resolve_target(target);
                self.cache_modulation(layer_id, route, enabled, source, destination, amount);
                if let Some(index) = self.active_engine_index(layer_id) {
                    self.layers[index].handle_control(
                        &mut self.voice_pool,
                        ControlMessage::SetModulation {
                            target: LayerTarget::Explicit(layer_id),
                            route,
                            enabled,
                            source,
                            destination,
                            amount,
                        },
                    );
                }
            }
            ControlMessage::SetModulationParam {
                target,
                route,
                parameter,
            } => {
                let layer_id = self.resolve_target(target);
                self.patch
                    .layer_mut(layer_id)
                    .set_modulation_param(route, parameter);
                if let Some(index) = self.active_engine_index(layer_id) {
                    self.layers[index].handle_control(
                        &mut self.voice_pool,
                        ControlMessage::SetModulationParam {
                            target: LayerTarget::Explicit(layer_id),
                            route,
                            parameter,
                        },
                    );
                }
            }
            ControlMessage::SetLayerMode(mode) => {
                self.patch.mode = mode;
                self.request_topology_change();
            }
            ControlMessage::SetSplitPoint(split_point) => {
                self.patch.set_split_point(split_point);
                self.request_topology_change();
            }
            ControlMessage::SetEditLayer(layer) => {
                self.edit_layer = layer;
                self.request_topology_change();
            }
            ControlMessage::SetFilterOversampling(oversampling) => {
                self.set_filter_oversampling(oversampling);
            }
            ControlMessage::SetFilterType(filter_type) => self.set_filter_type(filter_type),
            ControlMessage::NoteOn { note, velocity } => self.route_note_on(note, velocity),
            ControlMessage::NoteOff { note } => self.route_note_off(note),
            ControlMessage::AllNotesOff => self.clear_all_notes(),
            ControlMessage::PitchBend { value } => self.for_each_layer_state(|layer, pool| {
                layer.handle_control(pool, ControlMessage::PitchBend { value })
            }),
            ControlMessage::ModWheel { value } => self.for_each_layer_state(|layer, pool| {
                layer.handle_control(pool, ControlMessage::ModWheel { value })
            }),
            ControlMessage::Pressure { value } => self.for_each_layer_state(|layer, pool| {
                layer.handle_control(pool, ControlMessage::Pressure { value })
            }),
            ControlMessage::SustainPedal { pressed } => {
                self.sustain_pressed = pressed;
                self.fan_out_performance(|layer, pool| {
                    layer.handle_control(pool, ControlMessage::SustainPedal { pressed })
                });
            }
            ControlMessage::ControlChange { controller, value } => {
                if matches!(controller, 2 | 4 | 11) {
                    self.for_each_layer_state(|layer, pool| {
                        layer.handle_control(
                            pool,
                            ControlMessage::ControlChange { controller, value },
                        )
                    });
                } else {
                    self.fan_out_performance(|layer, pool| {
                        layer.handle_control(
                            pool,
                            ControlMessage::ControlChange { controller, value },
                        )
                    });
                }
            }
        }
    }

    pub fn set_param(&mut self, param: ParamId, value: f32) {
        self.handle_control(ControlMessage::SetParam {
            target: LayerTarget::Edit,
            param,
            value,
        });
    }

    /// Applies a complete two-layer program and resets the audition layer to A.
    pub fn apply_patch(&mut self, patch: &Patch) {
        let mut next_patch = patch.clone();
        next_patch.validate();
        let topology_unchanged = next_patch.mode == self.applied_mode
            && next_patch.split_point == self.applied_split_point
            && self.applied_edit_layer == LayerId::A;
        self.patch = next_patch;
        self.edit_layer = LayerId::A;
        if topology_unchanged {
            if LAYERS == 1 {
                self.layers[0].apply_patch(&mut self.voice_pool, &self.patch.layer_a);
            } else {
                let mask = self.rendered_mask();
                if mask & Self::layer_bit(LayerId::A) != 0 {
                    self.layers[0].apply_patch(&mut self.voice_pool, &self.patch.layer_a);
                }
                if mask & Self::layer_bit(LayerId::B) != 0 {
                    self.layers[1].apply_patch(&mut self.voice_pool, &self.patch.layer_b);
                }
            }
            self.apply_effective_tempos();
            self.transition = TopologyTransition::default();
            return;
        }
        self.commit_topology();
        self.transition = TopologyTransition::default();
    }

    /// Updates the edit layer's local tempo.
    ///
    /// In slave modes this updates the editable fallback without displacing an
    /// already-learned external tempo.
    pub fn set_tempo_bpm(&mut self, tempo_bpm: f32) {
        self.set_target_param(LayerTarget::Edit, ParamId::Bpm, tempo_bpm);
    }

    pub fn tempo_bpm(&self) -> f32 {
        if let Some(external) = self.external_tempo() {
            return external;
        }
        self.active_engine_index(self.edit_layer)
            .map(|index| self.layers[index].tempo_bpm())
            .unwrap_or_else(|| self.patch.layer(self.edit_layer).bpm.clamp(30.0, 250.0))
    }

    pub fn local_tempo_bpm(&self) -> f32 {
        self.patch.layer(self.edit_layer).bpm.clamp(30.0, 250.0)
    }

    pub fn set_midi_clock_mode(&mut self, mode: MidiClockMode) {
        if self.midi_clock.set_mode(mode) {
            self.apply_effective_tempos();
        }
    }

    pub fn handle_midi_realtime(&mut self, event: MidiRealtimeEvent) {
        if let Some(bpm) = self.midi_clock.handle(event) {
            self.apply_common_tempo(bpm);
        }
    }

    pub fn midi_clock_status(&self) -> MidiClockStatus {
        self.midi_clock.status(self.tempo_bpm())
    }

    pub fn set_clock_division(&mut self, division: ClockDivision) {
        self.set_target_param(
            LayerTarget::Edit,
            ParamId::ClockDivide,
            division.index() as f32,
        );
    }

    pub fn clock_division(&self) -> ClockDivision {
        self.patch.layer(self.edit_layer).clock_divide
    }

    /// Applies the nonlinear filter oversampling policy to all voices.
    pub fn set_filter_oversampling(&mut self, oversampling: FilterOversampling) {
        self.filter_oversampling = oversampling;
        self.for_each_active_layer(|layer, pool| layer.set_filter_oversampling(pool, oversampling));
    }

    /// Applies a filter model to all voices, resetting their filter state.
    pub fn set_filter_type(&mut self, filter_type: FilterType) {
        self.filter_type = filter_type;
        self.for_each_active_layer(|layer, pool| layer.set_filter_type(pool, filter_type));
    }

    fn resolve_target(&self, target: LayerTarget) -> LayerId {
        match target {
            LayerTarget::Edit => self.edit_layer,
            LayerTarget::Explicit(layer) => layer,
        }
    }

    const fn layer_bit(layer: LayerId) -> u8 {
        match layer {
            LayerId::A => 0b01,
            LayerId::B => 0b10,
        }
    }

    const fn layer_index(layer: LayerId) -> usize {
        match layer {
            LayerId::A => 0,
            LayerId::B => 1,
        }
    }

    fn rendered_mask(&self) -> u8 {
        if LAYERS == 1 {
            return Self::layer_bit(self.applied_edit_layer);
        }
        match self.applied_mode {
            LayerMode::Normal => Self::layer_bit(self.applied_edit_layer),
            LayerMode::Stack | LayerMode::Split => 0b11,
        }
    }

    fn active_engine_index(&self, layer: LayerId) -> Option<usize> {
        if self.rendered_mask() & Self::layer_bit(layer) == 0 {
            return None;
        }
        if LAYERS == 1 {
            Some(0)
        } else {
            Some(Self::layer_index(layer))
        }
    }

    fn for_each_active_layer(
        &mut self,
        mut f: impl FnMut(&mut LayerEngine<PACKS>, &mut VoicePool<PACKS>),
    ) {
        if LAYERS == 1 {
            f(&mut self.layers[0], &mut self.voice_pool);
            return;
        }
        let mask = self.rendered_mask();
        for index in 0..LAYERS {
            if mask & (1 << index) != 0 {
                f(&mut self.layers[index], &mut self.voice_pool);
            }
        }
    }

    fn fan_out_performance(
        &mut self,
        f: impl FnMut(&mut LayerEngine<PACKS>, &mut VoicePool<PACKS>),
    ) {
        self.for_each_active_layer(f);
    }

    fn for_each_layer_state(
        &mut self,
        mut f: impl FnMut(&mut LayerEngine<PACKS>, &mut VoicePool<PACKS>),
    ) {
        for layer in &mut self.layers {
            f(layer, &mut self.voice_pool);
        }
    }

    fn set_target_param(&mut self, target: LayerTarget, param: ParamId, value: f32) {
        let layer_id = self.resolve_target(target);
        self.patch.layer_mut(layer_id).set_param(param, value);
        let Some(index) = self.active_engine_index(layer_id) else {
            return;
        };

        match param {
            ParamId::MasterVolume => self.layers[index].set_program_volume(value),
            ParamId::EffectEnabled => self.layers[index].effects_mut().set_enabled(value >= 0.5),
            ParamId::EffectType => self.layers[index]
                .effects_mut()
                .set_type(EffectType::from_index(value as usize)),
            ParamId::EffectMix => self.layers[index].effects_mut().set_mix(value),
            ParamId::EffectClockSync => self.layers[index]
                .effects_mut()
                .set_clock_sync(value >= 0.5),
            ParamId::EffectParam1 => self.layers[index].effects_mut().set_param1(value),
            ParamId::EffectParam2 => self.layers[index].effects_mut().set_param2(value),
            ParamId::Bpm => {
                self.layers[index].set_local_tempo_bpm(value);
                if let Some(external) = self.external_tempo() {
                    self.layers[index].set_tempo_bpm(&mut self.voice_pool, external);
                } else {
                    let local = self.layers[index].local_tempo_bpm();
                    self.layers[index].set_tempo_bpm(&mut self.voice_pool, local);
                }
            }
            ParamId::ClockDivide => self.layers[index].set_clock_division(
                &mut self.voice_pool,
                ClockDivision::from_index(value as usize),
            ),
            _ => self.layers[index].handle_control(
                &mut self.voice_pool,
                ControlMessage::SetParam {
                    target: LayerTarget::Explicit(layer_id),
                    param,
                    value,
                },
            ),
        }
    }

    fn cache_modulation(
        &mut self,
        layer: LayerId,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
    ) {
        let patch = self.patch.layer_mut(layer);
        match route {
            ModRoute::Free(index) => {
                if let Some(slot) = patch.mod_matrix.free_slots.get_mut(index) {
                    slot.enabled = enabled;
                    slot.source = source;
                    slot.destination = destination;
                    slot.amount = amount;
                }
            }
            ModRoute::Dedicated(dedicated) => {
                if let Some(slot) = patch.mod_matrix.dedicated.get_mut(dedicated.index()) {
                    slot.enabled = enabled;
                    slot.destination = destination;
                    slot.amount = amount;
                }
            }
        }
    }

    fn external_tempo(&self) -> Option<f32> {
        self.midi_clock
            .learned_bpm()
            .filter(|_| self.midi_clock.mode().receives_clock())
    }

    fn apply_common_tempo(&mut self, bpm: f32) {
        self.for_each_active_layer(|layer, pool| layer.set_tempo_bpm(pool, bpm));
    }

    fn apply_effective_tempos(&mut self) {
        if let Some(external) = self.external_tempo() {
            self.apply_common_tempo(external);
            return;
        }
        if LAYERS == 1 {
            let bpm = self.patch.layer(self.applied_edit_layer).bpm;
            self.layers[0].set_local_tempo_bpm(bpm);
            self.layers[0].set_tempo_bpm(&mut self.voice_pool, bpm);
            return;
        }
        let mask = self.rendered_mask();
        for layer_id in [LayerId::A, LayerId::B] {
            if mask & Self::layer_bit(layer_id) == 0 {
                continue;
            }
            let index = Self::layer_index(layer_id);
            let bpm = self.patch.layer(layer_id).bpm;
            self.layers[index].set_local_tempo_bpm(bpm);
            self.layers[index].set_tempo_bpm(&mut self.voice_pool, bpm);
        }
    }

    fn request_topology_change(&mut self) {
        if self.patch.mode != self.applied_mode
            || self.patch.split_point != self.applied_split_point
            || self.edit_layer != self.applied_edit_layer
        {
            self.transition.request();
        }
    }

    fn commit_topology(&mut self) {
        for layer in &mut self.layers {
            layer.clear_note_state();
        }
        self.voice_pool.reset();
        self.output_limiter.reset();
        self.applied_mode = self.patch.mode;
        self.applied_split_point = self.patch.split_point;
        self.applied_edit_layer = self.edit_layer;

        if LAYERS == 1 {
            self.layers[0].assign_region(VoiceRegion::all::<PACKS>());
            self.layers[0].apply_patch(
                &mut self.voice_pool,
                self.patch.layer(self.applied_edit_layer),
            );
        } else if self.applied_mode == LayerMode::Normal {
            let index = Self::layer_index(self.applied_edit_layer);
            self.layers[index].assign_region(VoiceRegion::all::<PACKS>());
            self.layers[index].apply_patch(
                &mut self.voice_pool,
                self.patch.layer(self.applied_edit_layer),
            );
        } else {
            let half = PACKS / 2;
            self.layers[0].assign_region(VoiceRegion::from_packs(0, half));
            self.layers[1].assign_region(VoiceRegion::from_packs(half, half));
            self.layers[0].apply_patch(&mut self.voice_pool, &self.patch.layer_a);
            self.layers[1].apply_patch(&mut self.voice_pool, &self.patch.layer_b);
        }
        self.apply_effective_tempos();
        let oversampling = self.filter_oversampling;
        self.for_each_active_layer(|layer, pool| {
            layer.set_filter_oversampling(pool, oversampling)
        });
        let filter_type = self.filter_type;
        self.for_each_active_layer(|layer, pool| layer.set_filter_type(pool, filter_type));

        self.route_mask.fill(0);
        let held = self.physical_keys.clone();
        for (note, velocity) in held.iter() {
            let mask = self.route_for_note(note);
            self.route_mask[note as usize] = mask;
            self.send_note_on(mask, note, velocity);
        }
        if self.sustain_pressed {
            self.fan_out_performance(|layer, pool| {
                layer.handle_control(pool, ControlMessage::SustainPedal { pressed: true })
            });
        }
    }

    fn route_for_note(&self, note: u8) -> u8 {
        if LAYERS == 1 {
            return Self::layer_bit(self.applied_edit_layer);
        }
        match self.applied_mode {
            LayerMode::Normal => Self::layer_bit(self.applied_edit_layer),
            LayerMode::Stack => 0b11,
            LayerMode::Split => {
                if note < self.applied_split_point {
                    Self::layer_bit(LayerId::A)
                } else {
                    Self::layer_bit(LayerId::B)
                }
            }
        }
    }

    fn send_note_on(&mut self, mask: u8, note: u8, velocity: f32) {
        for layer_id in [LayerId::A, LayerId::B] {
            if mask & Self::layer_bit(layer_id) == 0 {
                continue;
            }
            if let Some(index) = self.active_engine_index(layer_id) {
                self.layers[index].handle_control(
                    &mut self.voice_pool,
                    ControlMessage::NoteOn { note, velocity },
                );
            }
        }
    }

    fn send_note_off(&mut self, mask: u8, note: u8) {
        for layer_id in [LayerId::A, LayerId::B] {
            if mask & Self::layer_bit(layer_id) == 0 {
                continue;
            }
            if let Some(index) = self.active_engine_index(layer_id) {
                self.layers[index]
                    .handle_control(&mut self.voice_pool, ControlMessage::NoteOff { note });
            }
        }
    }

    fn route_note_on(&mut self, note: u8, velocity: f32) {
        if note >= 128 {
            return;
        }
        if velocity <= 0.0 {
            self.route_note_off(note);
            return;
        }
        self.physical_keys.press(note, velocity);
        let mask = self.route_for_note(note);
        self.route_mask[note as usize] = mask;
        self.send_note_on(mask, note, velocity.clamp(0.0, 1.0));
    }

    fn route_note_off(&mut self, note: u8) {
        if note >= 128 {
            return;
        }
        self.physical_keys.release(note);
        let mask = core::mem::take(&mut self.route_mask[note as usize]);
        self.send_note_off(mask, note);
    }

    fn clear_all_notes(&mut self) {
        self.for_each_active_layer(|layer, pool| {
            layer.handle_control(pool, ControlMessage::AllNotesOff)
        });
        for layer in &mut self.layers {
            layer.clear_note_state();
        }
        self.physical_keys.clear();
        self.sustain_pressed = false;
        self.route_mask.fill(0);
    }

    pub fn playback_status(&self) -> LayerPlaybackStatus {
        LayerPlaybackStatus {
            mode: self.patch.mode,
            edit_layer: self.edit_layer,
            rendered_mask: self.rendered_mask(),
            degraded: LAYERS == 1 && self.patch.mode != LayerMode::Normal,
        }
    }

    pub fn layer_active_voice_count(&self, layer: LayerId) -> usize {
        self.active_engine_index(layer)
            .map(|index| self.layers[index].active_voice_count(&self.voice_pool))
            .unwrap_or(0)
    }

    pub fn note_on(&mut self, note: u8, velocity: f32) {
        self.handle_control(ControlMessage::NoteOn { note, velocity });
    }

    pub fn note_off(&mut self, note: u8) {
        self.handle_control(ControlMessage::NoteOff { note });
    }

    pub fn all_notes_off(&mut self) {
        self.handle_control(ControlMessage::AllNotesOff);
    }

    pub fn pitch_bend(&mut self, value: f32) {
        self.handle_control(ControlMessage::PitchBend { value });
    }

    pub fn mod_wheel(&mut self, value: f32) {
        self.handle_control(ControlMessage::ModWheel { value });
    }

    pub fn pressure(&mut self, value: f32) {
        self.handle_control(ControlMessage::Pressure { value });
    }

    pub fn sustain_pedal(&mut self, pressed: bool) {
        self.handle_control(ControlMessage::SustainPedal { pressed });
    }

    pub fn control_change(&mut self, controller: u8, value: f32) {
        self.handle_control(ControlMessage::ControlChange { controller, value });
    }

    /// Renders mono audio into `buffer` (duplicated internally from the stereo mix).
    pub fn process(&mut self, buffer: &mut [f32]) {
        self.process_interleaved(buffer, 2);
    }

    /// Renders interleaved audio with `channels` samples per frame (1 = mono, 2 = stereo).
    pub fn process_interleaved(&mut self, buffer: &mut [f32], channels: usize) {
        let mut ctx = crate::create_render_context!();
        self.process_interleaved_inner(buffer, channels, &mut ctx);
    }

    /// Renders audio while reporting DSP stage boundaries to `profiler`.
    #[cfg(feature = "profiling")]
    pub fn process_interleaved_profiled(
        &mut self,
        buffer: &mut [f32],
        channels: usize,
        profiler: &mut impl RenderProfiler,
    ) {
        let mut ctx = RenderContext::new(profiler);
        self.process_interleaved_inner(buffer, channels, &mut ctx);
    }

    fn process_interleaved_inner(
        &mut self,
        buffer: &mut [f32],
        channels: usize,
        ctx: &mut RenderContext<'_>,
    ) {
        if channels == 0 {
            return;
        }

        self.midi_clock.advance(buffer.len() / channels);

        for frame in buffer.chunks_exact_mut(channels) {
            if self.rate_adapter.needs_render() {
                let rendered = self.next(ctx);
                self.rate_adapter.submit(rendered);
            }
            let (left, right) = self.rate_adapter.output();
            self.rate_adapter.advance();
            if channels == 1 {
                frame[0] = (0.5 * (left + right)).clamp(-1.0, 1.0);
            } else {
                for (channel, sample) in frame.iter_mut().enumerate() {
                    *sample = if channel % 2 == 0 { left } else { right };
                }
            }
        }
    }

    fn render_layer(
        layer: &mut LayerEngine<PACKS>,
        voice_pool: &mut VoicePool<PACKS>,
        effects_memory: &mut [f32],
        ctx: &mut RenderContext<'_>,
    ) -> (f32, f32) {
        let (left, right) = layer.next(voice_pool, ctx);
        let effect_modulation = layer.effect_modulation();
        let lowest_active_note = layer.lowest_active_note(voice_pool);
        crate::profiler_begin!(ctx, RenderStage::Effects);
        let left = left * MIX_BUS_GAIN;
        let right = right * MIX_BUS_GAIN;
        let (effects, program_volume) = layer.effects_and_volume();
        let (left, right) = effects.next(
            left,
            right,
            effects_memory,
            effect_modulation,
            lowest_active_note,
            ctx,
        );
        crate::profiler_end!(ctx, RenderStage::Effects);
        (left * program_volume, right * program_volume)
    }

    fn next(&mut self, ctx: &mut RenderContext<'_>) -> (f32, f32) {
        let mut mixed_left = 0.0;
        let mut mixed_right = 0.0;
        let mask = self.rendered_mask();

        if LAYERS == 1 {
            let (left, right) = Self::render_layer(
                &mut self.layers[0],
                &mut self.voice_pool,
                self.effects_memory.as_mut(),
                ctx,
            );
            mixed_left = left;
            mixed_right = right;
        } else {
            let half = self.effects_memory.as_ref().len() / 2;
            let (memory_a, memory_b) = self.effects_memory.as_mut().split_at_mut(half);
            if mask & Self::layer_bit(LayerId::A) != 0 {
                let (left, right) =
                    Self::render_layer(&mut self.layers[0], &mut self.voice_pool, memory_a, ctx);
                mixed_left += left;
                mixed_right += right;
            }
            if mask & Self::layer_bit(LayerId::B) != 0 {
                let (left, right) =
                    Self::render_layer(&mut self.layers[1], &mut self.voice_pool, memory_b, ctx);
                mixed_left += left;
                mixed_right += right;
            }
        }

        crate::profiler_begin!(ctx, RenderStage::MasterOutput);
        let (left, right) = self.output_limiter.next(mixed_left, mixed_right);
        let apply_topology = self.transition.advance();
        let gain = self.transition.gain;
        let output = (
            (left * gain).clamp(-1.0, 1.0),
            (right * gain).clamp(-1.0, 1.0),
        );
        crate::profiler_end!(ctx, RenderStage::MasterOutput);
        if apply_topology {
            self.commit_topology();
            self.transition.begin_fade_in();
        }
        output
    }

    pub fn active_notes(&self) -> ActiveNotes<PACKS> {
        let mut notes = ActiveNotes::new();
        if LAYERS == 1 {
            self.layers[0].for_each_active_note(&self.voice_pool, |note| {
                notes.push(note);
            });
            return notes;
        }
        let mask = self.rendered_mask();
        for index in 0..LAYERS {
            if mask & (1 << index) != 0 {
                self.layers[index].for_each_active_note(&self.voice_pool, |note| {
                    notes.push(note);
                });
            }
        }
        notes
    }

    pub fn active_voice_count(&self) -> usize {
        if LAYERS == 1 {
            return self.layers[0].active_voice_count(&self.voice_pool);
        }
        let mask = self.rendered_mask();
        (0..LAYERS)
            .filter(|index| mask & (1 << index) != 0)
            .map(|index| self.layers[index].active_voice_count(&self.voice_pool))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use crate::dsp::FilterOversampling;
    #[cfg(feature = "filter-all")]
    use crate::dsp::FilterType;
    use crate::midi::clock::{MidiClockMode, MidiRealtimeEvent, MidiTransportState};
    use crate::{
        ClockDivision, ControlMessage, DEFAULT_SAMPLE_RATE, DEFAULT_TEMPO_BPM, DedicatedModSource,
        EffectType, EngineInitError, LayerId, LayerMode, LayerPatch, LayerTarget, ModDestination,
        ModRoute, ModSource, ParamId, Patch, SynthEngine, SynthEngineWithMemory, VOICE_PACKS,
    };

    extern crate std;
    use std::vec::Vec;

    fn left_rms(buffer: &[f32]) -> f32 {
        let mut sum = 0.0;
        let mut count = 0;

        for frame in buffer.chunks_exact(2) {
            sum += frame[0] * frame[0];
            count += 1;
        }

        (sum / count as f32).sqrt()
    }

    fn channel_samples(buffer: &[f32], channels: usize, channel: usize) -> Vec<f32> {
        buffer
            .chunks_exact(channels)
            .map(|frame| frame[channel])
            .collect()
    }

    fn finish_topology_transition<Memory, const PACKS: usize, const LAYERS: usize>(
        engine: &mut SynthEngineWithMemory<Memory, PACKS, LAYERS>,
    ) where
        Memory: AsRef<[f32]> + AsMut<[f32]>,
    {
        let mut output = [0.0; super::TOPOLOGY_FADE_SAMPLES * 4];
        engine.process(&mut output);
        assert!(output.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn invalid_effects_memory_layouts_are_rejected_before_rendering() {
        assert!(matches!(
            SynthEngineWithMemory::<[f32; 8], 2, 3>::new_with_effects_memory(48_000.0, [0.0; 8]),
            Err(EngineInitError::UnsupportedLayerCount)
        ));
        assert!(matches!(
            SynthEngineWithMemory::<[f32; 0], 2, 2>::new_with_effects_memory(48_000.0, []),
            Err(EngineInitError::EmptyEffectsMemory)
        ));
        assert!(matches!(
            SynthEngineWithMemory::<[f32; 5], 2, 2>::new_with_effects_memory(48_000.0, [0.0; 5]),
            Err(EngineInitError::UnevenLayerMemory)
        ));
        assert!(matches!(
            SynthEngineWithMemory::<[f32; 6], 2, 2>::new_with_effects_memory(48_000.0, [0.0; 6]),
            Err(EngineInitError::InvalidStereoMemory)
        ));
        assert!(matches!(
            SynthEngineWithMemory::<[f32; 8], 3, 2>::new_with_effects_memory(48_000.0, [0.0; 8]),
            Err(EngineInitError::UnevenVoiceRegions)
        ));
    }

    #[test]
    fn normal_a_and_b_each_own_the_complete_voice_pool() {
        let mut engine =
            SynthEngineWithMemory::<[f32; 64], { VOICE_PACKS }, 2>::new_with_effects_memory(48_000.0, [0.0; 64])
                .unwrap();
        for note in 48..64 {
            engine.note_on(note, 1.0);
        }
        assert_eq!(engine.layer_active_voice_count(LayerId::A), 16);
        assert_eq!(engine.layer_active_voice_count(LayerId::B), 0);

        engine.handle_control(ControlMessage::SetEditLayer(LayerId::B));
        finish_topology_transition(&mut engine);
        assert_eq!(engine.playback_status().rendered_mask, 0b10);
        assert_eq!(engine.layer_active_voice_count(LayerId::A), 0);
        assert_eq!(engine.layer_active_voice_count(LayerId::B), 16);
    }

    #[test]
    fn stack_and_split_use_fixed_disjoint_half_pool_regions() {
        let mut stack_patch = Patch::default();
        stack_patch.mode = LayerMode::Stack;
        let mut stack =
            SynthEngineWithMemory::<[f32; 64], { VOICE_PACKS }, 2>::new_with_effects_memory(48_000.0, [0.0; 64])
                .unwrap();
        stack.apply_patch(&stack_patch);
        for note in 48..56 {
            stack.note_on(note, 1.0);
        }
        assert_eq!(stack.layer_active_voice_count(LayerId::A), 8);
        assert_eq!(stack.layer_active_voice_count(LayerId::B), 8);
        assert_eq!(stack.active_voice_count(), 16);

        let mut split_patch = Patch::default();
        split_patch.mode = LayerMode::Split;
        split_patch.split_point = 60;
        let mut split =
            SynthEngineWithMemory::<[f32; 64], { VOICE_PACKS }, 2>::new_with_effects_memory(48_000.0, [0.0; 64])
                .unwrap();
        split.apply_patch(&split_patch);
        for note in 40..48 {
            split.note_on(note, 1.0);
        }
        for note in 60..68 {
            split.note_on(note, 1.0);
        }
        assert_eq!(split.layer_active_voice_count(LayerId::A), 8);
        assert_eq!(split.layer_active_voice_count(LayerId::B), 8);
        let notes_a = split.layers[0].active_notes(&split.voice_pool);
        let notes_b = split.layers[1].active_notes(&split.voice_pool);
        assert!(!notes_a.contains(&60));
        assert!(notes_b.contains(&60), "the split-point note belongs to B");
    }

    #[test]
    fn note_off_uses_the_route_recorded_by_note_on() {
        let mut patch = Patch::default();
        patch.mode = LayerMode::Split;
        patch.split_point = 60;
        let mut engine = SynthEngineWithMemory::<[f32; 64], { VOICE_PACKS }, 2>::new_with_effects_memory(
            48_000.0,
            [0.0; 64],
        )
        .unwrap();
        engine.apply_patch(&patch);
        engine.note_on(59, 1.0);
        assert!(engine.layers[0].has_pressed_key(59));

        engine.handle_control(ControlMessage::SetSplitPoint(50));
        engine.note_off(59);
        assert!(!engine.layers[0].has_pressed_key(59));
        assert!(!engine.layers[1].has_pressed_key(59));
    }

    #[test]
    fn layer_effects_memory_regions_are_disjoint() {
        let mut engine = SynthEngineWithMemory::<[f32; 128], { VOICE_PACKS }, 2>::new_with_effects_memory(
            48_000.0, [0.0; 128],
        )
        .unwrap();
        engine.handle_control(ControlMessage::SetParam {
            target: LayerTarget::Explicit(LayerId::A),
            param: ParamId::EffectEnabled,
            value: 1.0,
        });
        engine.handle_control(ControlMessage::SetParam {
            target: LayerTarget::Explicit(LayerId::A),
            param: ParamId::EffectType,
            value: EffectType::DelayMono.index() as f32,
        });
        engine.effects_memory[64..].fill(37.0);
        engine.note_on(60, 1.0);
        let mut output = [0.0; 512];
        engine.process(&mut output);
        assert!(
            engine.effects_memory[64..]
                .iter()
                .all(|sample| *sample == 37.0)
        );

        engine.all_notes_off();
        engine.handle_control(ControlMessage::SetEditLayer(LayerId::B));
        finish_topology_transition(&mut engine);
        engine.effects_memory[..64].fill(19.0);
        engine.handle_control(ControlMessage::SetParam {
            target: LayerTarget::Explicit(LayerId::B),
            param: ParamId::EffectEnabled,
            value: 1.0,
        });
        engine.handle_control(ControlMessage::SetParam {
            target: LayerTarget::Explicit(LayerId::B),
            param: ParamId::EffectType,
            value: EffectType::DelayMono.index() as f32,
        });
        engine.note_on(67, 1.0);
        engine.process(&mut output);
        assert!(
            engine.effects_memory[..64]
                .iter()
                .all(|sample| *sample == 19.0)
        );
    }

    #[test]
    fn topology_change_repartitions_and_retriggers_physically_held_notes() {
        let mut engine =
            SynthEngineWithMemory::<[f32; 64], { VOICE_PACKS }, 2>::new_with_effects_memory(48_000.0, [0.0; 64])
        .unwrap();
        engine.set_filter_oversampling(FilterOversampling::X2);
        #[cfg(feature = "filter-all")]
        engine.set_filter_type(FilterType::GainLimitedTpt);
        engine.note_on(60, 0.75);
        engine.sustain_pedal(true);
        engine.handle_control(ControlMessage::SetLayerMode(LayerMode::Stack));
        finish_topology_transition(&mut engine);
        assert_eq!(engine.playback_status().rendered_mask, 0b11);
        assert_eq!(engine.route_mask[60], 0b11);
        assert_eq!(engine.layer_active_voice_count(LayerId::A), 1);
        assert_eq!(engine.layer_active_voice_count(LayerId::B), 1);
        assert_eq!(engine.filter_oversampling, FilterOversampling::X2);
        #[cfg(feature = "filter-all")]
        assert_eq!(engine.filter_type, FilterType::GainLimitedTpt);

        engine.note_off(60);
        assert_eq!(engine.layer_active_voice_count(LayerId::A), 1);
        assert_eq!(engine.layer_active_voice_count(LayerId::B), 1);
        engine.sustain_pedal(false);
        engine.all_notes_off();
        assert_eq!(engine.route_mask, [0; 128]);
        assert!(engine.physical_keys.is_empty());
    }

    #[test]
    fn targeted_layer_parameters_remain_independent() {
        let mut patch = Patch::default();
        patch.mode = LayerMode::Stack;
        patch.layer_a.master_volume = 0.2;
        patch.layer_b.master_volume = 0.8;
        let mut engine =
            SynthEngineWithMemory::<[f32; 64], { VOICE_PACKS }, 2>::new_with_effects_memory(48_000.0, [0.0; 64])
                .unwrap();
        engine.apply_patch(&patch);
        assert_eq!(engine.layers[0].program_volume(), 0.2);
        assert_eq!(engine.layers[1].program_volume(), 0.8);

        engine.handle_control(ControlMessage::SetParam {
            target: LayerTarget::Explicit(LayerId::B),
            param: ParamId::MasterVolume,
            value: 0.4,
        });
        assert_eq!(engine.layers[0].program_volume(), 0.2);
        assert_eq!(engine.layers[1].program_volume(), 0.4);
        assert_eq!(engine.patch.layer_a.master_volume, 0.2);
        assert_eq!(engine.patch.layer_b.master_volume, 0.4);

        engine.handle_control(ControlMessage::SetModulation {
            target: LayerTarget::Explicit(LayerId::B),
            route: ModRoute::Free(0),
            enabled: true,
            source: ModSource::Dc,
            destination: ModDestination::FilterCutoff,
            amount: 0.5,
        });
        assert!(!engine.patch.layer_a.mod_matrix.free_slots[0].enabled);
        assert!(engine.patch.layer_b.mod_matrix.free_slots[0].enabled);
    }

    #[test]
    fn layer_unison_and_sustain_state_stays_inside_each_region() {
        let mut patch = Patch::default();
        patch.mode = LayerMode::Stack;
        patch.layer_a.unison_enabled = true;
        patch.layer_a.unison_mode = crate::UnisonMode::V4;
        patch.layer_b.unison_enabled = false;
        let mut engine =
            SynthEngineWithMemory::<[f32; 64], { VOICE_PACKS }, 2>::new_with_effects_memory(48_000.0, [0.0; 64])
                .unwrap();
        engine.apply_patch(&patch);
        engine.note_on(60, 1.0);
        assert_eq!(engine.layer_active_voice_count(LayerId::A), 4);
        assert_eq!(engine.layer_active_voice_count(LayerId::B), 1);

        engine.sustain_pedal(true);
        engine.note_off(60);
        assert_eq!(engine.layer_active_voice_count(LayerId::A), 4);
        assert_eq!(engine.layer_active_voice_count(LayerId::B), 1);
        engine.sustain_pedal(false);
    }

    #[test]
    fn external_clock_is_common_but_layer_divisions_remain_independent() {
        let mut patch = Patch::default();
        patch.mode = LayerMode::Stack;
        patch.layer_a.bpm = 80.0;
        patch.layer_b.bpm = 120.0;
        patch.layer_a.clock_divide = ClockDivision::Quarter;
        patch.layer_b.clock_divide = ClockDivision::Sixteenth;
        let mut engine =
            SynthEngineWithMemory::<[f32; 64], { VOICE_PACKS }, 2>::new_with_effects_memory(48_000.0, [0.0; 64])
                .unwrap();
        engine.apply_patch(&patch);
        assert_eq!(engine.layers[0].tempo_bpm(), 80.0);
        assert_eq!(engine.layers[1].tempo_bpm(), 120.0);

        engine.set_midi_clock_mode(MidiClockMode::Slave);
        for timestamp in [0, 25_000, 50_000, 75_000, 100_000, 125_000] {
            engine.handle_midi_realtime(MidiRealtimeEvent::TimingClock {
                timestamp_micros: timestamp,
            });
        }
        assert!((engine.layers[0].tempo_bpm() - 100.0).abs() < 0.01);
        assert!((engine.layers[1].tempo_bpm() - 100.0).abs() < 0.01);
        assert_eq!(engine.patch.layer_a.clock_divide, ClockDivision::Quarter);
        assert_eq!(engine.patch.layer_b.clock_divide, ClockDivision::Sixteenth);
    }

    #[test]
    fn tempo_control_updates_and_clamps_the_engine_parameter() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        assert_eq!(engine.tempo_bpm(), DEFAULT_TEMPO_BPM);

        engine.handle_control(ControlMessage::SetTempoBpm {
            target: LayerTarget::Edit,
            bpm: 98.0,
        });
        assert_eq!(engine.tempo_bpm(), 98.0);
        engine.set_tempo_bpm(500.0);
        assert_eq!(engine.tempo_bpm(), 250.0);
    }

    #[test]
    fn external_clock_overrides_but_does_not_replace_local_tempo() {
        let mut engine = SynthEngine::<1, 64>::new(48_000.0);
        engine.set_tempo_bpm(90.0);
        engine.set_midi_clock_mode(MidiClockMode::Slave);
        for timestamp in [0, 25_000, 50_000, 75_000, 100_000, 125_000] {
            engine.handle_midi_realtime(MidiRealtimeEvent::TimingClock {
                timestamp_micros: timestamp,
            });
        }
        assert!((engine.tempo_bpm() - 100.0).abs() < 0.01);
        engine.set_tempo_bpm(72.0);
        assert!((engine.tempo_bpm() - 100.0).abs() < 0.01);
        let mut patch = LayerPatch::default();
        patch.bpm = 60.0;
        engine.apply_patch(&Patch {
            layer_a: patch,
            ..Patch::default()
        });
        assert!((engine.tempo_bpm() - 100.0).abs() < 0.01);
        assert_eq!(engine.local_tempo_bpm(), 60.0);
        engine.set_midi_clock_mode(MidiClockMode::Off);
        assert_eq!(engine.tempo_bpm(), 60.0);
    }

    #[test]
    fn slave_transport_tracks_start_and_stop() {
        let mut engine = SynthEngine::<1, 64>::new(48_000.0);
        engine.set_midi_clock_mode(MidiClockMode::Slave);
        engine.handle_midi_realtime(MidiRealtimeEvent::Start);
        engine.handle_midi_realtime(MidiRealtimeEvent::TimingClock {
            timestamp_micros: 1,
        });
        assert_eq!(
            engine.midi_clock_status().transport,
            MidiTransportState::Running
        );
        assert_eq!(engine.midi_clock_status().pulse_position, 1);
        engine.handle_midi_realtime(MidiRealtimeEvent::Stop);
        assert_eq!(
            engine.midi_clock_status().transport,
            MidiTransportState::Stopped
        );
    }

    #[test]
    fn clock_division_control_and_patch_application_update_engine_state() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        assert_eq!(engine.clock_division(), ClockDivision::Quarter);

        engine.set_param(
            ParamId::ClockDivide,
            ClockDivision::EighthTriplet.index() as f32,
        );
        assert_eq!(engine.clock_division(), ClockDivision::EighthTriplet);

        let mut patch = LayerPatch::default();
        patch.bpm = 87.0;
        patch.clock_divide = ClockDivision::ThirtySecondTriplet;
        engine.apply_patch(&Patch {
            layer_a: patch,
            ..Patch::default()
        });
        assert_eq!(engine.tempo_bpm(), 87.0);
        assert_eq!(engine.clock_division(), ClockDivision::ThirtySecondTriplet);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn legacy_bucket_brigade_name_loads_but_new_saves_use_the_expanded_name() {
        let effect_type: EffectType = serde_json::from_str("\"BbdDelay\"").unwrap();
        assert_eq!(effect_type, EffectType::BucketBrigadeDelay);
        assert_eq!(
            serde_json::to_string(&effect_type).unwrap(),
            "\"BucketBrigadeDelay\""
        );
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn profiled_render_balances_every_stage_boundary() {
        use crate::{
            ControlMessage, EffectType, ModDestination, ParamId, RenderProfiler, RenderStage,
        };

        struct BoundaryCounter {
            begins: [u32; RenderStage::COUNT],
            ends: [u32; RenderStage::COUNT],
        }

        impl RenderProfiler for BoundaryCounter {
            fn begin(&mut self, stage: RenderStage) {
                self.begins[stage.index()] += 1;
            }

            fn end(&mut self, stage: RenderStage) {
                self.ends[stage.index()] += 1;
            }
        }

        let mut engine = SynthEngine::<{ VOICE_PACKS }, 64>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectEnabled, 1.0));
        engine.handle_control(ControlMessage::edit_param(
            ParamId::EffectType,
            EffectType::Reverb.index() as f32,
        ));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectMix, 0.5));
        engine.handle_control(ControlMessage::edit_param(ParamId::Lfo1Depth, 1.0));
        engine.handle_control(ControlMessage::edit_param(
            ParamId::Lfo1Destination,
            ModDestination::FilterCutoff.index() as f32,
        ));
        engine.note_on(60, 1.0);
        let mut profiler = BoundaryCounter {
            begins: [0; RenderStage::COUNT],
            ends: [0; RenderStage::COUNT],
        };
        let mut output = [0.0; 64];

        engine.process_interleaved_profiled(&mut output, 2, &mut profiler);

        assert_eq!(profiler.begins, profiler.ends);

        // Firmware-only stages and unused control-rate hooks stay at zero here.
        let engine_stages = [
            RenderStage::EnvelopesAndModulation,
            RenderStage::EnvelopeAdvance,
            RenderStage::LfoControlRouting,
            RenderStage::LfoGeneration,
            RenderStage::AudioModulationRouting,
            RenderStage::Oscillators,
            RenderStage::OscillatorControl,
            RenderStage::OscillatorWaveform,
            RenderStage::OscillatorMix,
            RenderStage::Filter,
            RenderStage::AmplifierAndPan,
            RenderStage::Effects,
            RenderStage::EffectsPreparation,
            RenderStage::ReverbCombs,
            RenderStage::ReverbAllpasses,
            RenderStage::EffectsMix,
            RenderStage::MasterOutput,
        ];
        for stage in engine_stages {
            assert!(
                profiler.begins[stage.index()] > 0,
                "{stage:?} was never entered"
            );
        }
    }

    fn rendered_note_rms(mut engine: SynthEngine, note: u8, velocity: f32, frames: usize) -> f32 {
        engine.handle_control(ControlMessage::NoteOn { note, velocity });
        let mut buffer = std::vec![0.0; frames * 2];
        engine.process(&mut buffer);
        left_rms(&buffer)
    }

    #[test]
    #[cfg(all(not(feature = "downsampling"), not(feature = "wide-1")))]
    fn default_note_on_renders_oscillator_without_noise() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut buffer = std::vec![0.0; 16_384 * 2];
        engine.process(&mut buffer);
        let rms = left_rms(&buffer);

        assert!(rms > 0.09, "default osc1 note should be audible, RMS {rms}");
    }

    #[test]
    fn vca_initial_level_drone_produces_audio_without_amp_envelope() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::VcaInitialLevel, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEnvAmount, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut buffer = std::vec![0.0; 8_192 * 2];
        engine.process(&mut buffer);
        let rms = left_rms(&buffer);

        assert!(
            rms > 0.05,
            "full VCA level should produce audio without amp envelope amount, RMS {rms}"
        );
    }

    #[test]
    fn note_off_decays_instead_of_cutting_to_silence() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgAttack, 0.002));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgRelease, 0.05));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut attack_buffer = std::vec![0.0; 1024 * 2];
        engine.process(&mut attack_buffer);
        assert!(left_rms(&attack_buffer) > 0.05);

        engine.handle_control(ControlMessage::NoteOff { note: 60 });
        let mut release_start = std::vec![0.0; 128 * 2];
        engine.process(&mut release_start);
        let release_start_rms = left_rms(&release_start);

        let mut release_tail = std::vec![0.0; 4096 * 2];
        engine.process(&mut release_tail);
        let release_tail_rms = left_rms(&release_tail);

        assert!(
            release_start_rms > 0.001,
            "note-off should decay instead of hard-muting, RMS {release_start_rms}"
        );
        assert!(
            release_tail_rms < release_start_rms * 0.5,
            "release should decay over time, start RMS {release_start_rms}, tail RMS {release_tail_rms}"
        );
    }

    #[test]
    fn amp_release_param_controls_release_tail() {
        fn release_rms(release_seconds: f32) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(
                ParamId::AmpEgRelease,
                release_seconds,
            ));
            engine.handle_control(ControlMessage::NoteOn {
                note: 60,
                velocity: 1.0,
            });

            let mut attack_buffer = std::vec![0.0; 4096 * 2];
            engine.process(&mut attack_buffer);
            engine.handle_control(ControlMessage::NoteOff { note: 60 });

            // Measure after the short envelope has had time to close. Starting the
            // window at note-off makes the result depend unnecessarily on the
            // oscillator's phase during those first few cycles.
            let mut release_start = std::vec![0.0; 512 * 2];
            engine.process(&mut release_start);
            let mut release_buffer = std::vec![0.0; 2048 * 2];
            engine.process(&mut release_buffer);
            left_rms(&release_buffer)
        }

        let short_release = release_rms(0.005);
        let long_release = release_rms(0.1);

        assert!(
            long_release > short_release * 3.0,
            "amp release should shape release tail, short {short_release}, long {long_release}"
        );
    }

    #[test]
    fn amp_delay_param_delays_initial_output() {
        let mut delayed = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        delayed.handle_control(ControlMessage::edit_param(ParamId::AmpEgDelay, 0.05));
        let delayed_rms = rendered_note_rms(delayed, 60, 1.0, 512);

        let immediate = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        let immediate_rms = rendered_note_rms(immediate, 60, 1.0, 512);

        assert!(
            delayed_rms < immediate_rms * 0.01,
            "amp delay should suppress the initial output window, delayed {delayed_rms}, immediate {immediate_rms}"
        );
    }

    #[test]
    fn amp_env_amount_controls_output_level() {
        let mut full = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        full.handle_control(ControlMessage::edit_param(ParamId::AmpVelocity, 0.0));
        full.handle_control(ControlMessage::edit_param(ParamId::AmpEnvAmount, 1.0));
        let full_rms = rendered_note_rms(full, 60, 1.0, 4096);

        let mut reduced = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        reduced.handle_control(ControlMessage::edit_param(ParamId::AmpVelocity, 0.0));
        reduced.handle_control(ControlMessage::edit_param(ParamId::AmpEnvAmount, 0.25));
        let reduced_rms = rendered_note_rms(reduced, 60, 1.0, 4096);

        assert!(
            full_rms > reduced_rms * 3.0,
            "amp env amount should scale output level, full {full_rms}, reduced {reduced_rms}"
        );
    }

    #[test]
    fn amp_velocity_param_controls_velocity_sensitivity() {
        fn render(env_amount: f32, velocity_amount: f32, note_velocity: f32) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(
                ParamId::AmpEnvAmount,
                env_amount,
            ));
            engine.handle_control(ControlMessage::edit_param(
                ParamId::AmpVelocity,
                velocity_amount,
            ));
            rendered_note_rms(engine, 60, note_velocity, 4096)
        }

        let sensitive_low_rms = render(0.0, 1.0, 0.25);
        let sensitive_high_rms = render(0.0, 1.0, 1.0);
        let insensitive_low_rms = render(1.0, 0.0, 0.25);
        let insensitive_high_rms = render(1.0, 0.0, 1.0);

        assert!(
            sensitive_high_rms > sensitive_low_rms * 3.0,
            "amp velocity should make high velocity louder, low {sensitive_low_rms}, high {sensitive_high_rms}"
        );
        assert!(
            (insensitive_high_rms - insensitive_low_rms).abs() < insensitive_high_rms * 0.01,
            "amp velocity 0 should ignore note velocity, low {insensitive_low_rms}, high {insensitive_high_rms}"
        );
    }

    #[test]
    fn amp_velocity_adds_to_envelope_amount_and_clamps_at_full_level() {
        fn render(env_amount: f32, velocity_amount: f32) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(
                ParamId::AmpEnvAmount,
                env_amount,
            ));
            engine.handle_control(ControlMessage::edit_param(
                ParamId::AmpVelocity,
                velocity_amount,
            ));
            rendered_note_rms(engine, 60, 1.0, 4096)
        }

        let velocity_boosted_rms = render(0.45, 1.0);
        let full_envelope_rms = render(1.0, 0.0);

        assert!(
            (velocity_boosted_rms - full_envelope_rms).abs() < full_envelope_rms * 0.01,
            "envelope amount plus velocity should clamp at full VCA level, velocity {velocity_boosted_rms}, full {full_envelope_rms}"
        );
    }

    #[test]
    fn filter_envelope_params_shape_filter_modulation() {
        fn filtered_attack_rms(filter_attack_seconds: f32) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 112.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 1.0));
            engine.handle_control(ControlMessage::edit_param(
                ParamId::FilterEgAttack,
                filter_attack_seconds,
            ));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgDecay, 5.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgSustain, 1.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgAttack, 0.0005));
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgDecay, 0.0005));
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgSustain, 1.0));
            engine.handle_control(ControlMessage::NoteOn {
                note: 60,
                velocity: 1.0,
            });

            let mut buffer = std::vec![0.0; 2048 * 2];
            engine.process(&mut buffer);
            left_rms(&buffer)
        }

        let fast_attack = filtered_attack_rms(0.0005);
        let slow_attack = filtered_attack_rms(2.0);

        assert!(
            fast_attack > slow_attack * 1.2,
            "filter EG attack should affect filter modulation, fast RMS {fast_attack}, slow RMS {slow_attack}"
        );
    }

    #[test]
    fn filter_delay_param_delays_filter_envelope_modulation() {
        fn filtered_delay_rms(delay_seconds: f32) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 112.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 1.0));
            engine.handle_control(ControlMessage::edit_param(
                ParamId::FilterEgDelay,
                delay_seconds,
            ));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgAttack, 0.0005));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgDecay, 5.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgSustain, 1.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgAttack, 0.0005));
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgDecay, 0.0005));
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgSustain, 1.0));
            rendered_note_rms(engine, 60, 1.0, 2048)
        }

        let immediate = filtered_delay_rms(0.0);
        let delayed = filtered_delay_rms(0.05);

        assert!(
            immediate > delayed * 1.2,
            "filter EG delay should delay filter opening, immediate {immediate}, delayed {delayed}"
        );
    }

    #[test]
    fn filter_velocity_param_controls_filter_envelope_depth() {
        fn filtered_velocity_rms(filter_velocity: f32, note_velocity: f32) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 80.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
            engine.handle_control(ControlMessage::edit_param(
                ParamId::FilterVelocity,
                filter_velocity,
            ));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgAttack, 0.0005));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgDecay, 5.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgSustain, 1.0));
            rendered_note_rms(engine, 60, note_velocity, 4096)
        }

        let sensitive_low = filtered_velocity_rms(1.0, 0.25);
        let sensitive_high = filtered_velocity_rms(1.0, 1.0);
        let insensitive_low = filtered_velocity_rms(0.0, 0.25);
        let insensitive_high = filtered_velocity_rms(0.0, 1.0);

        assert!(
            sensitive_high > sensitive_low * 1.1,
            "filter velocity should add envelope depth independently, low {sensitive_low}, high {sensitive_high}"
        );
        assert!(
            (insensitive_high - insensitive_low).abs() < insensitive_high * 0.05,
            "filter velocity 0 should ignore note velocity, low {insensitive_low}, high {insensitive_high}"
        );
    }

    #[test]
    fn filter_velocity_offsets_inverted_filter_envelope_depth() {
        fn filtered_velocity_rms(note_velocity: f32) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 1780.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, -1.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterVelocity, 1.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgAttack, 0.0005));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgDecay, 5.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEgSustain, 1.0));
            rendered_note_rms(engine, 60, note_velocity, 4096)
        }

        let low_velocity = filtered_velocity_rms(0.25);
        let high_velocity = filtered_velocity_rms(1.0);

        assert!(
            high_velocity > low_velocity * 1.2,
            "positive filter velocity should offset inverted filter EG modulation, low {low_velocity}, high {high_velocity}"
        );
    }

    #[test]
    fn filter_control_params_remain_wired_and_stable() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgAttack, 0.0005));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgDecay, 0.0005));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgSustain, 1.0));

        for (param, value) in [
            (ParamId::FilterCutoff, 225.0),
            (ParamId::FilterResonance, 0.8),
            (ParamId::FilterPoles, 0.0),
            (ParamId::FilterKeyTrack, 0.5),
            (ParamId::FilterEnvAmount, 0.4),
            (ParamId::FilterVelocity, 0.5),
            (ParamId::FilterAudioMod, 0.25),
        ] {
            engine.handle_control(ControlMessage::edit_param(param, value));
        }

        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 0.8,
        });

        let mut buffer = std::vec![0.0; 4096 * 2];
        engine.process(&mut buffer);
        let rms = left_rms(&buffer);
        let peak = buffer
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0f32, f32::max);

        assert!(
            rms.is_finite() && rms > 0.001,
            "filter-controlled patch should render, RMS {rms}"
        );
        assert!(
            peak.is_finite() && peak < 1.0,
            "filter-controlled patch should stay bounded, peak {peak}"
        );
    }

    #[test]
    fn normal_chords_stay_below_output_clamp() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        for note in [48, 55, 60, 64, 67, 72] {
            engine.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        let mut buffer = std::vec![0.0; 4096 * 2];
        engine.process(&mut buffer);
        let peak = buffer
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0f32, f32::max);

        assert!(
            peak < 0.98,
            "normal chord should render without final-stage clipping, peak {peak}"
        );
    }

    #[test]
    fn multichannel_output_advances_once_per_audio_frame() {
        let mut stereo = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        stereo.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        let mut stereo_buffer = std::vec![0.0; 512 * 2];
        stereo.process(&mut stereo_buffer);
        let stereo_left = channel_samples(&stereo_buffer, 2, 0);

        let mut multichannel = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        multichannel.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        let mut multichannel_buffer = std::vec![0.0; 512 * 8];
        multichannel.process_interleaved(&mut multichannel_buffer, 8);
        let multichannel_first = channel_samples(&multichannel_buffer, 8, 0);

        for (idx, (stereo_sample, multichannel_sample)) in stereo_left
            .iter()
            .zip(multichannel_first.iter())
            .enumerate()
        {
            assert!(
                (stereo_sample - multichannel_sample).abs() < 1.0e-6,
                "frame {idx} advanced differently: stereo {stereo_sample}, multichannel {multichannel_sample}"
            );
        }

        for (idx, frame) in multichannel_buffer.chunks_exact(8).enumerate() {
            assert!(
                frame
                    .iter()
                    .all(|sample| (*sample - frame[0]).abs() < 1.0e-6),
                "frame {idx} should contain the same mono synth sample on every output channel"
            );
        }
    }

    #[test]
    fn multichannel_output_repeats_stereo_pairs() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::PanSpread, 1.0));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        engine.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        let mut buffer = std::vec![0.0; 2048 * 4];
        engine.process_interleaved(&mut buffer, 4);

        let pair_1_left = channel_samples(&buffer, 4, 0);
        let pair_1_right = channel_samples(&buffer, 4, 1);
        let pair_2_left = channel_samples(&buffer, 4, 2);
        let pair_2_right = channel_samples(&buffer, 4, 3);

        for (idx, (((left_1, right_1), left_2), right_2)) in pair_1_left
            .iter()
            .zip(pair_1_right.iter())
            .zip(pair_2_left.iter())
            .zip(pair_2_right.iter())
            .enumerate()
        {
            assert!(
                (left_1 - left_2).abs() < 1.0e-6,
                "frame {idx} should repeat left on channels 0 and 2"
            );
            assert!(
                (right_1 - right_2).abs() < 1.0e-6,
                "frame {idx} should repeat right on channels 1 and 3"
            );
        }

        let first_pair_difference = pair_1_left
            .iter()
            .zip(pair_1_right.iter())
            .map(|(left, right)| {
                let diff = left - right;
                diff * diff
            })
            .sum::<f32>()
            .sqrt();

        assert!(
            first_pair_difference > 0.01,
            "stereo spread should survive multichannel output routing"
        );
    }

    #[test]
    fn polyphonic_mix_is_not_divided_by_active_voice_count() {
        let mut single = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        single.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        let mut single_buffer = std::vec![0.0; 4096 * 2];
        single.process(&mut single_buffer);
        let single_rms = left_rms(&single_buffer);

        let mut chord = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        chord.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        chord.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });
        let mut chord_buffer = std::vec![0.0; 4096 * 2];
        chord.process(&mut chord_buffer);
        let chord_rms = left_rms(&chord_buffer);

        assert!(
            chord_rms > single_rms * 1.05,
            "two voices should add energy, single RMS {single_rms}, chord RMS {chord_rms}"
        );
    }

    #[test]
    fn hard_sync_keeps_osc1_audible_with_osc1_only_mix() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc2Enabled, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::HardSync, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgAttack, 0.002));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut buffer = std::vec![0.0; 4096 * 2];
        engine.process(&mut buffer);
        let rms = left_rms(&buffer);

        assert!(
            rms > 0.05,
            "hard sync should not mute osc1 with osc1-only mix, RMS {rms}"
        );
    }

    #[test]
    #[cfg(all(not(feature = "downsampling"), not(feature = "wide-1")))]
    fn enabling_hard_sync_on_active_note_keeps_osc1_audible() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc2Enabled, 1.0));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut before = std::vec![0.0; 1024 * 2];
        engine.process(&mut before);

        engine.handle_control(ControlMessage::edit_param(ParamId::HardSync, 1.0));
        let mut after = std::vec![0.0; 4096 * 2];
        engine.process(&mut after);
        let rms = left_rms(&after);

        assert!(
            rms > 0.05,
            "enabling hard sync on an active note should not mute osc1, RMS {rms}"
        );
    }

    #[test]
    fn hard_sync_with_osc2_off_does_not_mute_or_reset_osc1() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc2Enabled, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::HardSync, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgAttack, 0.002));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut buffer = std::vec![0.0; 4096 * 2];
        engine.process(&mut buffer);
        let rms = left_rms(&buffer);

        assert!(
            rms > 0.05,
            "hard sync with osc2 off should leave osc1 audible, RMS {rms}"
        );
    }

    #[test]
    fn lfo_to_filter_cutoff_opens_filter() {
        fn render_with_lfo(enabled: bool) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 46.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
            if enabled {
                engine.handle_control(ControlMessage::edit_param(ParamId::Lfo1Waveform, 3.0));
                engine.handle_control(ControlMessage::edit_param(ParamId::Lfo1Depth, 1.0));
                engine.handle_control(ControlMessage::edit_param(
                    ParamId::Lfo1Destination,
                    ModDestination::FilterCutoff.index() as f32,
                ));
            }
            rendered_note_rms(engine, 60, 1.0, 4096)
        }

        let static_filter = render_with_lfo(false);
        let modulated_filter = render_with_lfo(true);
        assert!(
            modulated_filter > static_filter * 1.5,
            "LFO cutoff modulation should open the filter, static {static_filter}, modulated {modulated_filter}"
        );
    }

    #[test]
    fn aux_envelope_to_filter_cutoff_opens_filter() {
        fn render_with_aux(enabled: bool) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 46.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
            if enabled {
                engine.handle_control(ControlMessage::edit_param(
                    ParamId::AuxEgDestination,
                    ModDestination::FilterCutoff.index() as f32,
                ));
                engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgAmount, 1.0));
                engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgAttack, 0.0005));
                engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgDecay, 5.0));
                engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgSustain, 1.0));
            }
            rendered_note_rms(engine, 60, 1.0, 4096)
        }

        let static_filter = render_with_aux(false);
        let modulated_filter = render_with_aux(true);
        assert!(
            modulated_filter > static_filter * 1.5,
            "aux envelope cutoff modulation should open the filter, static {static_filter}, modulated {modulated_filter}"
        );
    }

    #[test]
    fn aux_envelope_amount_can_invert_filter_modulation() {
        fn render_with_aux_amount(amount: f32) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 225.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
            engine.handle_control(ControlMessage::edit_param(
                ParamId::AuxEgDestination,
                ModDestination::FilterCutoff.index() as f32,
            ));
            engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgAmount, amount));
            engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgAttack, 0.0005));
            engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgDecay, 5.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgSustain, 1.0));
            rendered_note_rms(engine, 60, 1.0, 4096)
        }

        let positive = render_with_aux_amount(1.0);
        let negative = render_with_aux_amount(-1.0);
        assert!(
            positive > negative * 1.2,
            "positive aux amount should open the filter relative to inverted amount, positive {positive}, negative {negative}"
        );
    }

    #[test]
    fn aux_velocity_param_controls_modulation_depth() {
        fn render_with_velocity(note_velocity: f32) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 46.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
            engine.handle_control(ControlMessage::edit_param(
                ParamId::AuxEgDestination,
                ModDestination::FilterCutoff.index() as f32,
            ));
            engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgAmount, 1.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgVelocity, 1.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgAttack, 0.0005));
            engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgDecay, 5.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::AuxEgSustain, 1.0));
            rendered_note_rms(engine, 60, note_velocity, 4096)
        }

        let low = render_with_velocity(0.25);
        let high = render_with_velocity(1.0);
        assert!(
            high > low * 1.2,
            "aux velocity should increase modulation depth for high velocity notes, low {low}, high {high}"
        );
    }

    #[test]
    fn mod_matrix_lfo_to_filter_cutoff_opens_filter() {
        fn render_with_matrix(enabled: bool) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 46.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::Lfo1Waveform, 3.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::Lfo1Depth, 1.0));
            engine.handle_control(ControlMessage::SetModulation {
                target: LayerTarget::Edit,
                route: ModRoute::Free(0),
                enabled,
                source: ModSource::Lfo1,
                destination: ModDestination::FilterCutoff,
                amount: 1.0,
            });
            rendered_note_rms(engine, 60, 1.0, 4096)
        }

        let static_filter = render_with_matrix(false);
        let modulated_filter = render_with_matrix(true);
        assert!(
            modulated_filter > static_filter * 1.5,
            "matrix LFO cutoff modulation should open the filter, static {static_filter}, modulated {modulated_filter}"
        );
    }

    #[test]
    fn dedicated_mod_wheel_to_filter_cutoff_uses_controller_value() {
        fn render_with_wheel(value: f32) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 46.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
            engine.handle_control(ControlMessage::SetModulation {
                target: LayerTarget::Edit,
                route: ModRoute::Dedicated(DedicatedModSource::ModWheel),
                enabled: true,
                source: ModSource::ModWheel,
                destination: ModDestination::FilterCutoff,
                amount: 1.0,
            });
            engine.handle_control(ControlMessage::ModWheel { value });
            rendered_note_rms(engine, 60, 1.0, 4096)
        }

        let wheel_down = render_with_wheel(0.0);
        let wheel_up = render_with_wheel(1.0);
        assert!(
            wheel_up > wheel_down * 1.5,
            "mod wheel route should follow controller value, down {wheel_down}, up {wheel_up}"
        );
    }

    #[test]
    fn disabled_mod_matrix_route_has_no_effect() {
        fn render_with_route(enabled: bool) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpEnvAmount, 0.25));
            engine.handle_control(ControlMessage::edit_param(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::SetModulation {
                target: LayerTarget::Edit,
                route: ModRoute::Free(0),
                enabled,
                source: ModSource::Dc,
                destination: ModDestination::Vca,
                amount: 1.0,
            });
            rendered_note_rms(engine, 60, 1.0, 4096)
        }

        let disabled = render_with_route(false);
        let enabled = render_with_route(true);
        assert!(
            enabled > disabled * 1.5,
            "disabled route should leave VCA unmodulated, disabled {disabled}, enabled {enabled}"
        );
    }

    #[test]
    fn lfo_to_vca_changes_output_level_over_time() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEnvAmount, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpVelocity, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::Lfo1Rate, 67.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::Lfo1Depth, 1.0));
        engine.handle_control(ControlMessage::edit_param(
            ParamId::Lfo1Destination,
            ModDestination::Vca.index() as f32,
        ));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut first = std::vec![0.0; 1024 * 2];
        engine.process(&mut first);
        let first_rms = left_rms(&first);

        let mut second = std::vec![0.0; 1024 * 2];
        engine.process(&mut second);
        let second_rms = left_rms(&second);

        assert!(
            (first_rms - second_rms).abs() > first_rms.min(second_rms) * 0.1,
            "LFO VCA modulation should change level over time, first {first_rms}, second {second_rms}"
        );
    }

    #[test]
    fn filter_oversampling_control_message_can_change_while_rendering() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 2000.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 1.0));
        engine.handle_control(ControlMessage::SetFilterOversampling(
            FilterOversampling::Off,
        ));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut before = std::vec![0.0; 1024 * 2];
        engine.process(&mut before);

        engine.handle_control(ControlMessage::SetFilterOversampling(
            FilterOversampling::X4,
        ));
        let mut after = std::vec![0.0; 1024 * 2];
        engine.process(&mut after);

        let peak = after.iter().copied().map(f32::abs).fold(0.0f32, f32::max);
        assert!(
            peak.is_finite() && peak <= 1.0,
            "dynamic oversampling change should keep output finite and bounded, peak {peak}"
        );
    }

    #[test]
    fn disabled_effects_preserve_dry_output() {
        fn render(enabled: bool) -> Vec<f32> {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(
                ParamId::EffectEnabled,
                if enabled { 1.0 } else { 0.0 },
            ));
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectType, 11.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectMix, 1.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectParam1, 1.0));
            engine.handle_control(ControlMessage::NoteOn {
                note: 60,
                velocity: 1.0,
            });
            let mut buffer = std::vec![0.0; 2048 * 2];
            engine.process(&mut buffer);
            buffer
        }

        let dry = render(false);
        let wet = render(false);
        let max_delta = dry
            .iter()
            .zip(wet.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);

        assert!(
            max_delta < 1.0e-6,
            "disabled effects should leave dry render unchanged, max delta {max_delta}"
        );
    }

    #[test]
    fn mono_delay_produces_tail_after_note_release() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgRelease, 0.0005));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectEnabled, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectType, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectMix, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectParam1, 0.03));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectParam2, 0.55));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut attack = std::vec![0.0; 4096 * 2];
        engine.process(&mut attack);
        engine.handle_control(ControlMessage::NoteOff { note: 60 });
        let mut tail = std::vec![0.0; 4096 * 2];
        engine.process(&mut tail);

        assert!(
            left_rms(&tail) > 0.001,
            "delay should continue producing a tail after note release"
        );
    }

    #[test]
    fn high_pass_effect_reduces_low_notes_more_than_high_notes() {
        fn render_note(note: u8) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectEnabled, 1.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectType, 12.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectMix, 1.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectParam1, 0.65));
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectParam2, 0.0));
            rendered_note_rms(engine, note, 1.0, 4096)
        }

        let low = render_note(36);
        let high = render_note(84);
        assert!(
            high > low * 1.5,
            "HP filter should preserve high notes more than low notes, low {low}, high {high}"
        );
    }

    #[test]
    fn distortion_effect_stays_finite_and_bounded() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectEnabled, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectType, 11.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectMix, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectParam1, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectParam2, 1.0));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut buffer = std::vec![0.0; 4096 * 2];
        engine.process(&mut buffer);
        let peak = buffer.iter().copied().map(f32::abs).fold(0.0f32, f32::max);

        assert!(
            peak.is_finite() && peak <= 1.0,
            "distortion should remain finite and output-clamped, peak {peak}"
        );
    }

    #[test]
    fn reverb_effect_produces_decay_tail() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgRelease, 0.0005));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectEnabled, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectType, 9.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectMix, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectParam1, 0.8));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectParam2, 0.5));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut attack = std::vec![0.0; 4096 * 2];
        engine.process(&mut attack);
        engine.handle_control(ControlMessage::NoteOff { note: 60 });
        let mut tail = std::vec![0.0; 8192 * 2];
        engine.process(&mut tail);

        assert!(
            left_rms(&tail) > 0.001,
            "reverb should produce an audible tail after note release"
        );
    }

    #[test]
    fn modulation_matrix_can_control_fx_mix() {
        fn render_with_fx_mix_mod(enabled: bool) -> f32 {
            let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectEnabled, 1.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectType, 11.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectMix, 0.0));
            engine.handle_control(ControlMessage::edit_param(ParamId::EffectParam1, 1.0));
            engine.handle_control(ControlMessage::SetModulation {
                target: LayerTarget::Edit,
                route: ModRoute::Free(0),
                enabled,
                source: ModSource::Dc,
                destination: ModDestination::FxMix,
                amount: 1.0,
            });
            rendered_note_rms(engine, 60, 1.0, 4096)
        }

        let dry = render_with_fx_mix_mod(false);
        let modulated = render_with_fx_mix_mod(true);
        assert!(
            (modulated - dry).abs() > dry * 0.05,
            "DC -> FX Mix should change effect wet/dry balance, dry {dry}, modulated {modulated}"
        );
    }

    #[test]
    fn apply_patch_updates_engine_owned_parameters() {
        let mut engine = SynthEngine::<1, 64>::new(48_000.0);
        let mut patch = LayerPatch::default();
        patch.master_volume = 0.25;
        patch.effects.enabled = true;
        patch.effects.mix = 0.75;
        engine.apply_patch(&Patch {
            layer_a: patch,
            ..Patch::default()
        });
        assert_eq!(engine.layers[0].program_volume(), 0.25);
    }

    #[test]
    fn wide_pulse_sustain_settles_to_near_zero_dc() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc1Waveform, 3.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc1ShapeMod, 0.67));
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc2Enabled, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 8_000.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgAttack, 0.0005));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgDecay, 0.0005));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEgSustain, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::MasterVolume, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectEnabled, 0.0));
        engine.handle_control(ControlMessage::NoteOn {
            note: 36,
            velocity: 1.0,
        });

        let settle_frames = (DEFAULT_SAMPLE_RATE * 0.25) as usize;
        let mut settle = std::vec![0.0; settle_frames * 2];
        engine.process(&mut settle);

        let measure_frames = (DEFAULT_SAMPLE_RATE * 0.05) as usize;
        let mut measured = std::vec![0.0; measure_frames * 2];
        engine.process(&mut measured);
        let left = channel_samples(&measured, 2, 0);
        let dc = left.iter().sum::<f32>() / left.len() as f32;
        let rms = left_rms(&measured);

        assert!(
            dc.abs() < 0.02,
            "wide-pulse sustain should settle near zero DC, mean={dc}"
        );
        assert!(
            rms > 0.05,
            "blocked wide-pulse note should stay audible, RMS {rms}"
        );
    }

    #[test]
    fn warmed_wide_pulse_does_not_emit_a_note_on_dc_transient() {
        let sample_rate = 48_000.0;
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(sample_rate);
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc1Waveform, 3.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc1Frequency, 57.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc1ShapeMod, 0.67));
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc1KeyboardOn, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc1NoteReset, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::Osc2Enabled, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::FilterCutoff, 8_000.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::VcaInitialLevel, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::AmpEnvAmount, 0.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::MasterVolume, 1.0));
        engine.handle_control(ControlMessage::edit_param(ParamId::EffectEnabled, 0.0));

        let mut warmup = std::vec![0.0; sample_rate as usize];
        engine.process(&mut warmup);
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut onset = std::vec![0.0; (sample_rate * 0.1) as usize * 2];
        engine.process(&mut onset);
        let left = channel_samples(&onset, 2, 0);
        let mean = left.iter().sum::<f32>() / left.len() as f32;
        let rms = left_rms(&onset);
        assert!(
            mean.abs() < 0.002,
            "a warmed DC blocker should not turn pulse DC into a note-on transient, mean={mean}"
        );
        assert!(
            rms > 0.1,
            "the warmed pulse note should remain audible, RMS={rms}"
        );
    }
}
