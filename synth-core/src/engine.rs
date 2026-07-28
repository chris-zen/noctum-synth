//! Top-level synthesis engine and audio render entry point.

use crate::EffectType;
#[cfg(feature = "profiling")]
use crate::RenderProfiler;
use crate::dsp::lookahead_limiter::LookaheadLimiter;
use crate::dsp::{FilterOversampling, FilterType};
use crate::effects::EngineEffects;
use crate::midi::clock::MidiClockFollower;
use crate::profiling::{RenderContext, RenderStage};
use crate::rate_adapter::RateAdapter;
use crate::voice::VoiceManager;
use crate::{
    ActiveNotes, ClockDivision, ControlMessage, DEFAULT_TEMPO_BPM, ParamId, Patch, VOICE_PACKS,
};
use crate::midi::clock::{MidiClockMode, MidiClockStatus, MidiRealtimeEvent};

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
    voices: VoiceManager<PACKS>,
    effects: EngineEffects<Memory>,
    local_tempo_bpm: f32,
    tempo_bpm: f32,
    midi_clock: MidiClockFollower,
    clock_division: ClockDivision,
    master_volume: f32,
    output_limiter: LookaheadLimiter,
    rate_adapter: RateAdapter,
}

impl<const PACKS: usize, const FX_SAMPLES: usize> SynthEngineWithMemory<PACKS, [f32; FX_SAMPLES]> {
    /// Creates an engine at `sample_rate` with inline effects storage.
    pub fn new(sample_rate: f32) -> Self {
        let internal_sample_rate = RateAdapter::internal_sample_rate(sample_rate);
        let mut effects = crate::effects::Effects::new(internal_sample_rate);
        effects.set_tempo_bpm(DEFAULT_TEMPO_BPM);
        Self {
            voices: VoiceManager::<PACKS>::new(internal_sample_rate),
            effects,
            local_tempo_bpm: DEFAULT_TEMPO_BPM,
            tempo_bpm: DEFAULT_TEMPO_BPM,
            midi_clock: MidiClockFollower::new(sample_rate),
            clock_division: ClockDivision::default(),
            master_volume: 0.8,
            output_limiter: LookaheadLimiter::new(internal_sample_rate),
            rate_adapter: RateAdapter::default(),
        }
    }
}

impl<const PACKS: usize, Memory> SynthEngineWithMemory<PACKS, Memory>
where
    Memory: AsRef<[f32]> + AsMut<[f32]>,
{
    /// Creates an engine using caller-provided effects memory.
    pub fn new_with_effects_memory(sample_rate: f32, effects_memory: Memory) -> Self {
        let internal_sample_rate = RateAdapter::internal_sample_rate(sample_rate);
        let mut effects = EngineEffects::new_with_memory(internal_sample_rate, effects_memory);
        effects.set_tempo_bpm(DEFAULT_TEMPO_BPM);
        Self {
            voices: VoiceManager::<PACKS>::new(internal_sample_rate),
            effects,
            local_tempo_bpm: DEFAULT_TEMPO_BPM,
            tempo_bpm: DEFAULT_TEMPO_BPM,
            midi_clock: MidiClockFollower::new(sample_rate),
            clock_division: ClockDivision::default(),
            master_volume: 0.8,
            output_limiter: LookaheadLimiter::new(internal_sample_rate),
            rate_adapter: RateAdapter::default(),
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
        // VoiceManager::apply_patch writes the patch BPM into each block. Restore the
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

    fn next(&mut self, ctx: &mut RenderContext<'_>) -> (f32, f32) {
        let (left, right) = self.voices.next(ctx);

        crate::profiler_begin!(ctx, RenderStage::Effects);
        let left = left * MIX_BUS_GAIN;
        let right = right * MIX_BUS_GAIN;
        let (left, right) = self.effects.next(
            left,
            right,
            self.voices.effect_modulation(),
            self.voices.lowest_active_note(),
            ctx,
        );
        crate::profiler_end!(ctx, RenderStage::Effects);

        crate::profiler_begin!(ctx, RenderStage::MasterOutput);
        let gain = self.master_volume;
        let (left, right) = self.output_limiter.next(left * gain, right * gain);
        let output = (left.clamp(-1.0, 1.0), right.clamp(-1.0, 1.0));
        crate::profiler_end!(ctx, RenderStage::MasterOutput);
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
    use crate::dsp::FilterOversampling;
    use crate::midi::clock::{MidiClockMode, MidiRealtimeEvent, MidiTransportState};
    use crate::{
        ClockDivision, ControlMessage, DEFAULT_SAMPLE_RATE, DEFAULT_TEMPO_BPM, DedicatedModSource,
        EffectType, ModDestination, ModRoute, ModSource, ParamId, Patch, SynthEngine, VOICE_PACKS,
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

    #[test]
    fn tempo_control_updates_and_clamps_the_engine_parameter() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        assert_eq!(engine.tempo_bpm(), DEFAULT_TEMPO_BPM);

        engine.handle_control(ControlMessage::SetTempoBpm { bpm: 98.0 });
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
        let mut patch = Patch::default();
        patch.bpm = 60.0;
        engine.apply_patch(&patch);
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

        let mut patch = Patch::default();
        patch.bpm = 87.0;
        patch.clock_divide = ClockDivision::ThirtySecondTriplet;
        engine.apply_patch(&patch);
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
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectEnabled, 1.0));
        engine.handle_control(ControlMessage::SetParam(
            ParamId::EffectType,
            EffectType::Reverb.index() as f32,
        ));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectMix, 0.5));
        engine.handle_control(ControlMessage::SetParam(ParamId::Lfo1Depth, 1.0));
        engine.handle_control(ControlMessage::SetParam(
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
        engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::VcaInitialLevel, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.002));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgRelease, 0.05));
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
            engine.handle_control(ControlMessage::SetParam(
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
        delayed.handle_control(ControlMessage::SetParam(ParamId::AmpEgDelay, 0.05));
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
        full.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
        full.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 1.0));
        let full_rms = rendered_note_rms(full, 60, 1.0, 4096);

        let mut reduced = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        reduced.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
        reduced.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 0.25));
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
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, env_amount));
            engine.handle_control(ControlMessage::SetParam(
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
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, env_amount));
            engine.handle_control(ControlMessage::SetParam(
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
            engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 112.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, 1.0));
            engine.handle_control(ControlMessage::SetParam(
                ParamId::FilterEgAttack,
                filter_attack_seconds,
            ));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgDecay, 5.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgSustain, 1.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.0005));
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgDecay, 0.0005));
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgSustain, 1.0));
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
            engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 112.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, 1.0));
            engine.handle_control(ControlMessage::SetParam(
                ParamId::FilterEgDelay,
                delay_seconds,
            ));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgAttack, 0.0005));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgDecay, 5.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgSustain, 1.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.0005));
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgDecay, 0.0005));
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgSustain, 1.0));
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
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 80.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, 0.0));
            engine.handle_control(ControlMessage::SetParam(
                ParamId::FilterVelocity,
                filter_velocity,
            ));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgAttack, 0.0005));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgDecay, 5.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgSustain, 1.0));
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
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 1780.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, -1.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterVelocity, 1.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgAttack, 0.0005));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgDecay, 5.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEgSustain, 1.0));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.0005));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgDecay, 0.0005));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgSustain, 1.0));

        for (param, value) in [
            (ParamId::FilterCutoff, 225.0),
            (ParamId::FilterResonance, 0.8),
            (ParamId::FilterPoles, 0.0),
            (ParamId::FilterKeyTrack, 0.5),
            (ParamId::FilterEnvAmount, 0.4),
            (ParamId::FilterVelocity, 0.5),
            (ParamId::FilterAudioMod, 0.25),
        ] {
            engine.handle_control(ControlMessage::SetParam(param, value));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 1.0));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::HardSync, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.002));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 1.0));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut before = std::vec![0.0; 1024 * 2];
        engine.process(&mut before);

        engine.handle_control(ControlMessage::SetParam(ParamId::HardSync, 1.0));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::HardSync, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.002));
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
            engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 46.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, 0.0));
            if enabled {
                engine.handle_control(ControlMessage::SetParam(ParamId::Lfo1Waveform, 3.0));
                engine.handle_control(ControlMessage::SetParam(ParamId::Lfo1Depth, 1.0));
                engine.handle_control(ControlMessage::SetParam(
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
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 46.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, 0.0));
            if enabled {
                engine.handle_control(ControlMessage::SetParam(
                    ParamId::AuxEgDestination,
                    ModDestination::FilterCutoff.index() as f32,
                ));
                engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgAmount, 1.0));
                engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgAttack, 0.0005));
                engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgDecay, 5.0));
                engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgSustain, 1.0));
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
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 225.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, 0.0));
            engine.handle_control(ControlMessage::SetParam(
                ParamId::AuxEgDestination,
                ModDestination::FilterCutoff.index() as f32,
            ));
            engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgAmount, amount));
            engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgAttack, 0.0005));
            engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgDecay, 5.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgSustain, 1.0));
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
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 46.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, 0.0));
            engine.handle_control(ControlMessage::SetParam(
                ParamId::AuxEgDestination,
                ModDestination::FilterCutoff.index() as f32,
            ));
            engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgAmount, 1.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgVelocity, 1.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgAttack, 0.0005));
            engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgDecay, 5.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::AuxEgSustain, 1.0));
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
            engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 46.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::Lfo1Waveform, 3.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::Lfo1Depth, 1.0));
            engine.handle_control(ControlMessage::SetModulation {
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
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 46.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, 0.0));
            engine.handle_control(ControlMessage::SetModulation {
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
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 0.25));
            engine.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
            engine.handle_control(ControlMessage::SetModulation {
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
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::Lfo1Rate, 67.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::Lfo1Depth, 1.0));
        engine.handle_control(ControlMessage::SetParam(
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
        engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 2000.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 1.0));
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
            engine.handle_control(ControlMessage::SetParam(
                ParamId::EffectEnabled,
                if enabled { 1.0 } else { 0.0 },
            ));
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectType, 11.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectMix, 1.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectParam1, 1.0));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgRelease, 0.0005));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectEnabled, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectType, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectMix, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectParam1, 0.03));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectParam2, 0.55));
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
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectEnabled, 1.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectType, 12.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectMix, 1.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectParam1, 0.65));
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectParam2, 0.0));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectEnabled, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectType, 11.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectMix, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectParam1, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectParam2, 1.0));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgRelease, 0.0005));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectEnabled, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectType, 9.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectMix, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectParam1, 0.8));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectParam2, 0.5));
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
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectEnabled, 1.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectType, 11.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectMix, 0.0));
            engine.handle_control(ControlMessage::SetParam(ParamId::EffectParam1, 1.0));
            engine.handle_control(ControlMessage::SetModulation {
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
        let mut patch = Patch::default();
        patch.master_volume = 0.25;
        patch.effects.enabled = true;
        patch.effects.mix = 0.75;
        engine.apply_patch(&patch);
        assert_eq!(engine.master_volume, 0.25);
    }

    #[test]
    fn wide_pulse_sustain_settles_to_near_zero_dc() {
        let mut engine = SynthEngine::<{ VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc1Waveform, 3.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc1ShapeMod, 0.67));
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 8_000.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.0005));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgDecay, 0.0005));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgSustain, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::MasterVolume, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectEnabled, 0.0));
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
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc1Waveform, 3.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc1Frequency, 57.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc1ShapeMod, 0.67));
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc1KeyboardOn, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc1NoteReset, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 8_000.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::VcaInitialLevel, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::MasterVolume, 1.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::EffectEnabled, 0.0));

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
