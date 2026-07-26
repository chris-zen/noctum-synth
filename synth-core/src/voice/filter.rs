use crate::ParamId;
use crate::dsp::{FilterOversampling, FilterType};
use crate::math::WideF32;
use crate::patch::FilterParams;

pub struct Filter {
    engine: crate::dsp::Filter,
    envelope: crate::dsp::DadsrEnvelope,
}

impl Filter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            engine: crate::dsp::Filter::default(),
            envelope: crate::dsp::DadsrEnvelope::analog(sample_rate),
        }
    }

    pub fn apply_params(&mut self, params: &FilterParams) {
        self.set_cutoff(params.cutoff);
        self.set_resonance(params.resonance);
        self.set_poles(params.poles);
        self.set_key_track(params.key_track);
        self.set_env_amount(params.env_amount);
        self.set_env_velocity_amount(params.velocity);
        self.set_audio_mod(params.audio_mod);
        self.set_delay_seconds(params.eg_delay);
        self.set_attack_seconds(params.eg_attack);
        self.set_decay_seconds(params.eg_decay);
        self.set_sustain_level(params.eg_sustain);
        self.set_release_seconds(params.eg_release);
    }

    pub fn set_param(&mut self, id: ParamId, value: f32) -> bool {
        match id {
            ParamId::FilterCutoff => self.set_cutoff(value),
            ParamId::FilterResonance => self.set_resonance(value),
            ParamId::FilterPoles => self.set_poles(if value < 0.5 { 2 } else { 4 }),
            ParamId::FilterKeyTrack => self.set_key_track(value),
            ParamId::FilterEnvAmount => self.set_env_amount(value),
            ParamId::FilterVelocity => self.set_env_velocity_amount(value),
            ParamId::FilterAudioMod => self.set_audio_mod(value),
            ParamId::FilterEgDelay => self.set_delay_seconds(value),
            ParamId::FilterEgAttack => self.set_attack_seconds(value),
            ParamId::FilterEgDecay => self.set_decay_seconds(value),
            ParamId::FilterEgSustain => self.set_sustain_level(value),
            ParamId::FilterEgRelease => self.set_release_seconds(value),
            _ => return false,
        }
        true
    }

    pub fn next_envelope(&mut self) -> WideF32 {
        self.envelope.next()
    }

    pub fn trigger_lane(&mut self, lane: usize) {
        self.envelope.trigger_lane(lane);
    }

    pub fn release_lane(&mut self, lane: usize) {
        self.envelope.release_lane(lane);
    }

    pub fn release_all(&mut self) {
        self.envelope.release_all();
    }

    pub fn reset_lane(&mut self, lane: usize) {
        self.envelope.reset_lane(lane);
        self.engine.reset_lane(lane);
    }

    pub(crate) fn reset_dsp_lane(&mut self, lane: usize) {
        self.engine.reset_lane(lane);
    }

    pub fn set_oversampling(&mut self, oversampling: FilterOversampling) {
        self.engine.set_oversampling(oversampling);
    }

    pub fn set_filter_type(&mut self, filter_type: FilterType) {
        self.engine.set_filter_type(filter_type);
    }

    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.engine.set_cutoff(cutoff);
    }

    #[cfg(test)]
    pub(crate) fn cutoff(&self) -> f32 {
        self.engine.cutoff()
    }

    pub fn set_resonance(&mut self, resonance: f32) {
        self.engine.set_resonance(resonance);
    }

    #[cfg(test)]
    pub(crate) fn resonance(&self) -> f32 {
        self.engine.resonance()
    }

    pub fn set_poles(&mut self, poles: u8) {
        self.engine.set_poles(poles);
    }

    pub fn set_key_track(&mut self, key_track: f32) {
        self.engine.set_key_track(key_track);
    }

    pub fn set_env_amount(&mut self, env_amount: f32) {
        self.engine.set_env_amount(env_amount);
    }

    #[cfg(test)]
    pub(crate) fn env_amount(&self) -> f32 {
        self.engine.env_amount()
    }

    pub fn set_env_velocity_amount(&mut self, velocity: f32) {
        self.engine.set_env_velocity_amount(velocity);
    }

    pub fn set_audio_mod(&mut self, audio_mod: f32) {
        self.engine.set_audio_mod(audio_mod);
    }

    pub fn set_delay_seconds(&mut self, seconds: f32) {
        self.envelope.set_delay_seconds(seconds);
    }

    pub fn set_attack_seconds(&mut self, seconds: f32) {
        self.envelope.set_attack_seconds(seconds);
    }

    pub fn set_decay_seconds(&mut self, seconds: f32) {
        self.envelope.set_decay_seconds(seconds);
    }

    pub fn set_sustain_level(&mut self, sustain: f32) {
        self.envelope.set_sustain_level(sustain);
    }

    pub fn set_release_seconds(&mut self, seconds: f32) {
        self.envelope.set_release_seconds(seconds);
    }

    pub fn set_self_oscillation_color_enabled(&mut self, enabled: bool) {
        self.engine.set_self_oscillation_color_enabled(enabled);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process_prepared(
        &mut self,
        input: WideF32,
        note: WideF32,
        filter_env: WideF32,
        velocity: WideF32,
        osc1_audio: WideF32,
        cutoff_mod_semitones: WideF32,
        cutoff_mod_uniform_semitones: Option<f32>,
        resonance_mod: WideF32,
        audio_mod: WideF32,
        sample_rate: f32,
    ) -> WideF32 {
        self.engine.process_prepared(
            input,
            note,
            filter_env,
            velocity,
            osc1_audio,
            cutoff_mod_semitones,
            cutoff_mod_uniform_semitones,
            resonance_mod,
            audio_mod,
            sample_rate,
        )
    }
}
