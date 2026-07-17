//! Top-level synthesis engine and audio render entry point.

use crate::EffectType;
use crate::effects::EffectsWithMemory;
use crate::output_limiter::OutputLimiter;
#[cfg(feature = "profiling")]
use crate::profiling::{NoopProfiler, RenderProfiler, RenderStage};
use crate::voices::Voices;
use crate::{
    ActiveNotes, ControlMessage, DEFAULT_TEMPO_BPM, FilterOversampling, FilterType, ParamId, Patch,
    VOICE_PACKS,
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
    effects: EffectsWithMemory<Memory>,
    tempo_bpm: f32,
    master_volume: f32,
    output_limiter: OutputLimiter,
}

impl<const PACKS: usize, const FX_SAMPLES: usize> SynthEngineWithMemory<PACKS, [f32; FX_SAMPLES]> {
    /// Creates an engine at `sample_rate` with inline effects storage.
    pub fn new(sample_rate: f32) -> Self {
        let mut effects = crate::effects::Effects::new(sample_rate);
        effects.set_tempo_bpm(DEFAULT_TEMPO_BPM);
        Self {
            voices: Voices::<PACKS>::new(sample_rate),
            effects,
            tempo_bpm: DEFAULT_TEMPO_BPM,
            master_volume: 0.8,
            output_limiter: OutputLimiter::new(sample_rate),
        }
    }
}

impl<const PACKS: usize, Memory> SynthEngineWithMemory<PACKS, Memory>
where
    Memory: AsRef<[f32]> + AsMut<[f32]>,
{
    /// Creates an engine using caller-provided effects memory.
    pub fn new_with_effects_memory(sample_rate: f32, effects_memory: Memory) -> Self {
        let mut effects = EffectsWithMemory::new_with_memory(sample_rate, effects_memory);
        effects.set_tempo_bpm(DEFAULT_TEMPO_BPM);
        Self {
            voices: Voices::<PACKS>::new(sample_rate),
            effects,
            tempo_bpm: DEFAULT_TEMPO_BPM,
            master_volume: 0.8,
            output_limiter: OutputLimiter::new(sample_rate),
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
        patch.for_each_param(|param, value| {
            self.handle_control(ControlMessage::SetParam(param, value))
        });
        patch.for_each_modulation(|route, slot| {
            self.handle_control(ControlMessage::SetModulation {
                route,
                enabled: slot.enabled,
                source: slot.source,
                destination: slot.destination,
                amount: slot.amount,
            });
        });
    }

    /// Updates the global tempo and propagates it to clock-synchronized effects.
    pub fn set_tempo_bpm(&mut self, tempo_bpm: f32) {
        self.tempo_bpm = tempo_bpm.clamp(30.0, 250.0);
        self.effects.set_tempo_bpm(self.tempo_bpm);
    }

    pub fn tempo_bpm(&self) -> f32 {
        self.tempo_bpm
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

        for frame in buffer.chunks_exact_mut(channels) {
            #[cfg(feature = "profiling")]
            let (left, right) = self.next_profiled(profiler);
            #[cfg(not(feature = "profiling"))]
            let (left, right) = self.next();
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
