//! Top-level synthesis engine and audio render entry point.

use crate::EffectType;
use crate::effects::EngineEffects;
use crate::midi_clock::MidiClockFollower;
use crate::output_limiter::OutputLimiter;
#[cfg(feature = "profiling")]
use crate::profiling::{NoopProfiler, RenderProfiler, RenderStage};
use crate::render_rate::EngineRateAdapter;
use crate::voices::Voices;
use crate::{
    ActiveNotes, ClockDivision, ControlMessage, DEFAULT_TEMPO_BPM, FilterOversampling, FilterType,
    MidiClockMode, MidiClockStatus, MidiRealtimeEvent, ParamId, Patch, VOICE_PACKS,
};

/// Fixed headroom between the polyphonic voice sum and global effects.
///
/// This models the Prophet's calibrated voice/output summing gain without
/// changing gain dynamically with the number of active voices.
const MIX_BUS_GAIN: f32 = 0.55;

/// Synthesis engine with inline effects storage.
pub type SynthEngine<const PACKS: usize = VOICE_PACKS, const FX_SAMPLES: usize = 48_000> =
    SynthEngineWithMemory<PACKS, [f32; FX_SAMPLES]>;

/// Owns all voices and renders stereo audio from [`ControlMessage`] input.
///
/// Construct with [`SynthEngine::new`], feed control messages from the host
/// thread, then call [`SynthEngine::process`] or
/// [`SynthEngine::process_interleaved`] on the audio thread.
pub struct SynthEngineWithMemory<const PACKS: usize, Memory> {
    voices: Voices<PACKS>,
    effects: EngineEffects<Memory>,
    local_tempo_bpm: f32,
    tempo_bpm: f32,
    midi_clock: MidiClockFollower,
    clock_division: ClockDivision,
    master_volume: f32,
    output_limiter: OutputLimiter,
    output_rate: EngineRateAdapter,
}

impl<const PACKS: usize, const FX_SAMPLES: usize> SynthEngineWithMemory<PACKS, [f32; FX_SAMPLES]> {
    /// Creates an engine at `sample_rate` with inline effects storage.
    pub fn new(sample_rate: f32) -> Self {
        let internal_sample_rate = EngineRateAdapter::internal_sample_rate(sample_rate);
        let mut effects = crate::effects::Effects::new(internal_sample_rate);
        effects.set_tempo_bpm(DEFAULT_TEMPO_BPM);
        Self {
            voices: Voices::<PACKS>::new(internal_sample_rate),
            effects,
            local_tempo_bpm: DEFAULT_TEMPO_BPM,
            tempo_bpm: DEFAULT_TEMPO_BPM,
            midi_clock: MidiClockFollower::new(sample_rate),
            clock_division: ClockDivision::default(),
            master_volume: 0.8,
            output_limiter: OutputLimiter::new(internal_sample_rate),
            output_rate: EngineRateAdapter::default(),
        }
    }
}

impl<const PACKS: usize, Memory> SynthEngineWithMemory<PACKS, Memory>
where
    Memory: AsRef<[f32]> + AsMut<[f32]>,
{
    /// Creates an engine using caller-provided effects memory.
    pub fn new_with_effects_memory(sample_rate: f32, effects_memory: Memory) -> Self {
        let internal_sample_rate = EngineRateAdapter::internal_sample_rate(sample_rate);
        let mut effects = EngineEffects::new_with_memory(internal_sample_rate, effects_memory);
        effects.set_tempo_bpm(DEFAULT_TEMPO_BPM);
        Self {
            voices: Voices::<PACKS>::new(internal_sample_rate),
            effects,
            local_tempo_bpm: DEFAULT_TEMPO_BPM,
            tempo_bpm: DEFAULT_TEMPO_BPM,
            midi_clock: MidiClockFollower::new(sample_rate),
            clock_division: ClockDivision::default(),
            master_volume: 0.8,
            output_limiter: OutputLimiter::new(internal_sample_rate),
            output_rate: EngineRateAdapter::default(),
        }
    }

    /// Applies a single control or performance message.
    pub fn handle_control(&mut self, msg: ControlMessage) {
        match msg {
            ControlMessage::SetParam(ParamId::MasterVolume, value) => {
                self.master_volume = value.clamp(0.0, 1.0);
            }
            ControlMessage::SetParam(ParamId::EffectEnabled, value) => {
                self.effects.set_enabled(value >= 0.5);
            }
            ControlMessage::SetParam(ParamId::EffectType, value) => {
                self.effects
                    .set_type(EffectType::from_index(value as usize));
            }
            ControlMessage::SetParam(ParamId::EffectMix, value) => {
                self.effects.set_mix(value);
            }
            ControlMessage::SetParam(ParamId::EffectClockSync, value) => {
                self.effects.set_clock_sync(value >= 0.5);
            }
            ControlMessage::SetParam(ParamId::EffectParam1, value) => {
                self.effects.set_param1(value);
            }
            ControlMessage::SetParam(ParamId::EffectParam2, value) => {
                self.effects.set_param2(value);
            }
            ControlMessage::SetTempoBpm { bpm } => self.set_tempo_bpm(bpm),
            ControlMessage::SetMidiClockMode(mode) => self.set_midi_clock_mode(mode),
            ControlMessage::MidiRealtime(event) => self.handle_midi_realtime(event),
            ControlMessage::SetParam(ParamId::Bpm, value) => self.set_tempo_bpm(value),
            ControlMessage::SetParam(ParamId::ClockDivide, value) => {
                self.set_clock_division(ClockDivision::from_index(value as usize));
            }
            ControlMessage::SetFilterOversampling(oversampling) => {
                self.set_filter_oversampling(oversampling);
            }
            ControlMessage::SetFilterType(filter_type) => self.set_filter_type(filter_type),
            message => self.voices.handle_control(message),
        }
    }

    pub fn set_param(&mut self, param: ParamId, value: f32) {
        self.handle_control(ControlMessage::SetParam(param, value));
    }

    /// Applies every parameter and modulation route in a patch.
    pub fn apply_patch(&mut self, patch: &Patch) {
        self.set_tempo_bpm(patch.bpm);
        let effective_tempo_bpm = self
            .midi_clock
            .learned_bpm()
            .filter(|_| self.midi_clock.mode().receives_clock())
            .unwrap_or(self.local_tempo_bpm);
        self.set_clock_division(patch.clock_divide);
        self.voices.apply_patch(patch);
        self.effects.set_params(patch.effects);
        // Voices::apply_patch writes the patch BPM into each block. Restore the
        // externally learned tempo after that write when the engine is slaved;
        // in local modes this simply reapplies the patch BPM.
        self.apply_effective_tempo(effective_tempo_bpm);
        self.master_volume = patch.master_volume.clamp(0.0, 1.0);
    }

    /// Updates the global tempo and propagates it to clock-synchronized consumers.
    ///
    /// In slave modes this updates the editable fallback without displacing an
    /// already-learned external tempo.
    pub fn set_tempo_bpm(&mut self, tempo_bpm: f32) {
        self.local_tempo_bpm = tempo_bpm.clamp(30.0, 250.0);
        if self.midi_clock.learned_bpm().is_none() || !self.midi_clock.mode().receives_clock() {
            self.apply_effective_tempo(self.local_tempo_bpm);
        }
    }

    fn apply_effective_tempo(&mut self, tempo_bpm: f32) {
        self.tempo_bpm = tempo_bpm.clamp(30.0, 250.0);
        self.effects.set_tempo_bpm(self.tempo_bpm);
        self.voices.set_tempo_bpm(self.tempo_bpm);
    }

    pub fn tempo_bpm(&self) -> f32 {
        self.tempo_bpm
    }

    pub fn local_tempo_bpm(&self) -> f32 {
        self.local_tempo_bpm
    }

    pub fn set_midi_clock_mode(&mut self, mode: MidiClockMode) {
        if self.midi_clock.set_mode(mode) {
            self.apply_effective_tempo(self.local_tempo_bpm);
        }
    }

    pub fn handle_midi_realtime(&mut self, event: MidiRealtimeEvent) {
        if let Some(bpm) = self.midi_clock.handle(event) {
            self.apply_effective_tempo(bpm);
        }
    }

    pub fn midi_clock_status(&self) -> MidiClockStatus {
        self.midi_clock.status(self.tempo_bpm)
    }

    pub fn set_clock_division(&mut self, division: ClockDivision) {
        self.clock_division = division;
        self.voices.set_clock_division(division);
    }

    pub fn clock_division(&self) -> ClockDivision {
        self.clock_division
    }

    /// Applies the nonlinear filter oversampling policy to all voices.
    pub fn set_filter_oversampling(&mut self, oversampling: FilterOversampling) {
        self.voices.set_filter_oversampling(oversampling);
    }

    /// Applies a filter model to all voices, resetting their filter state.
    pub fn set_filter_type(&mut self, filter_type: FilterType) {
        self.voices.set_filter_type(filter_type);
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
        #[cfg(feature = "profiling")]
        {
            self.process_interleaved_inner(buffer, channels, &mut NoopProfiler);
        }
        #[cfg(not(feature = "profiling"))]
        self.process_interleaved_inner(buffer, channels);
    }

    /// Renders audio while reporting DSP stage boundaries to `profiler`.
    #[cfg(feature = "profiling")]
    pub fn process_interleaved_profiled(
        &mut self,
        buffer: &mut [f32],
        channels: usize,
        profiler: &mut impl RenderProfiler,
    ) {
        self.process_interleaved_inner(buffer, channels, profiler);
    }

    fn process_interleaved_inner(
        &mut self,
        buffer: &mut [f32],
        channels: usize,
        #[cfg(feature = "profiling")] profiler: &mut impl RenderProfiler,
    ) {
        if channels == 0 {
            return;
        }

        self.midi_clock.advance(buffer.len() / channels);

        for frame in buffer.chunks_exact_mut(channels) {
            if self.output_rate.needs_render() {
                #[cfg(feature = "profiling")]
                let rendered = self.next_profiled(profiler);
                #[cfg(not(feature = "profiling"))]
                let rendered = self.next();
                self.output_rate.submit(rendered);
            }
            let (left, right) = self.output_rate.output();
            self.output_rate.advance();
            if channels == 1 {
                frame[0] = (0.5 * (left + right)).clamp(-1.0, 1.0);
            } else {
                for (channel, sample) in frame.iter_mut().enumerate() {
                    *sample = if channel % 2 == 0 { left } else { right };
                }
            }
        }
    }

    #[cfg(not(feature = "profiling"))]
    fn next(&mut self) -> (f32, f32) {
        self.next_inner()
    }

    #[cfg(feature = "profiling")]
    fn next_profiled(&mut self, profiler: &mut impl RenderProfiler) -> (f32, f32) {
        self.next_inner(profiler)
    }

    fn next_inner(
        &mut self,
        #[cfg(feature = "profiling")] profiler: &mut impl RenderProfiler,
    ) -> (f32, f32) {
        #[cfg(feature = "profiling")]
        let (left, right) = self.voices.next_profiled(profiler);
        #[cfg(not(feature = "profiling"))]
        let (left, right) = self.voices.next();

        #[cfg(feature = "profiling")]
        profiler.begin(RenderStage::Effects);
        let left = left * MIX_BUS_GAIN;
        let right = right * MIX_BUS_GAIN;
        #[cfg(feature = "profiling")]
        let (left, right) = self.effects.next_profiled(
            left,
            right,
            self.voices.effect_modulation(),
            self.voices.lowest_active_note(),
            profiler,
        );
        #[cfg(not(feature = "profiling"))]
        let (left, right) = self.effects.next(
            left,
            right,
            self.voices.effect_modulation(),
            self.voices.lowest_active_note(),
        );
        #[cfg(feature = "profiling")]
        profiler.end(RenderStage::Effects);

        #[cfg(feature = "profiling")]
        profiler.begin(RenderStage::MasterOutput);
        let gain = self.master_volume;
        let (left, right) = self.output_limiter.next(left * gain, right * gain);
        let output = (left.clamp(-1.0, 1.0), right.clamp(-1.0, 1.0));
        #[cfg(feature = "profiling")]
        profiler.end(RenderStage::MasterOutput);
        output
    }

    pub fn active_notes(&self) -> ActiveNotes<PACKS> {
        self.voices.active_notes()
    }

    pub fn active_voice_count(&self) -> usize {
        self.voices.active_voice_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_patch_updates_engine_owned_parameters() {
        let mut engine = SynthEngine::<1, 64>::new(48_000.0);
        let mut patch = Patch::default();
        patch.master_volume = 0.25;
        patch.effects.enabled = true;
        patch.effects.mix = 0.75;
        engine.apply_patch(&patch);
        assert_eq!(engine.master_volume, 0.25);
    }
}
