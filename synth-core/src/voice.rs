//! Single voice-block signal chain (four SIMD lanes).

use wide::f32x4;

use crate::analog_oscillators::{OscillatorModulation, Oscillators};
use crate::patch::LfoDestination;
use crate::{
    DadsrEnvelope, FilterOversampling, LANES, LadderFilter, Lfo, LfoWaveform, MIN_LFO_RATE_HZ,
    midi_to_hz,
};

const LFO_PITCH_DEPTH_SEMITONES: f32 = 12.0;
const LFO_CUTOFF_DEPTH_SEMITONES: f32 = 48.0;

/// Four-lane subtractive voice: oscillators → filter → amplifier.
///
/// Each lane can represent a separate note. Envelopes, LFOs, and modulation are
/// evaluated per lane each sample step.
pub struct VoiceBlock {
    pub notes: [u8; LANES],
    pub velocities: [f32; LANES],
    pub gates: [bool; LANES],
    pub ages: [u64; LANES],

    pub oscillators: Oscillators,
    pub amp_env: DadsrEnvelope,
    pub filter_env: DadsrEnvelope,
    pub filter: LadderFilter,
    pub aux_env: DadsrEnvelope,
    pub aux_env_destination: LfoDestination,
    pub aux_env_amount: f32,
    pub aux_env_velocity_amount: f32,
    pub lfos: [Lfo; 4],
    pub lfo_destinations: [LfoDestination; 4],
    pub lfo_clock_sync: [bool; 4],
    pub lfo_base_rates_hz: [f32; 4],
    pub lfo_base_depths: [f32; 4],
    pub last_lfo_outputs: [f32x4; 4],
    pub amp_env_amount: f32,
    pub amp_velocity_amount: f32,
    pub pan_spread: f32,
    pub pan_sides: [f32; LANES],

    pub sample_rate: f32,
}

impl VoiceBlock {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            notes: [60; LANES],
            velocities: [1.0; LANES],
            gates: [false; LANES],
            ages: [0; LANES],
            oscillators: Oscillators::new(sample_rate),
            amp_env: DadsrEnvelope::analog(sample_rate),
            filter_env: DadsrEnvelope::analog(sample_rate),
            filter: LadderFilter::default(),
            aux_env: DadsrEnvelope::analog(sample_rate),
            aux_env_destination: LfoDestination::Off,
            aux_env_amount: 0.0,
            aux_env_velocity_amount: 0.0,
            lfos: core::array::from_fn(|_| Lfo::new(sample_rate)),
            lfo_destinations: [LfoDestination::Off; 4],
            lfo_clock_sync: [false; 4],
            lfo_base_rates_hz: [MIN_LFO_RATE_HZ; 4],
            lfo_base_depths: [0.0; 4],
            last_lfo_outputs: [f32x4::splat(0.0); 4],
            amp_env_amount: 1.0,
            amp_velocity_amount: 1.0,
            pan_spread: 0.0,
            pan_sides: [0.0; LANES],
            sample_rate,
        }
    }

    pub fn note_on(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        pan_side: f32,
        reset_key_synced_lfos: bool,
    ) {
        self.notes[lane] = note;
        self.velocities[lane] = velocity;
        self.gates[lane] = true;
        self.ages[lane] = 0;
        self.pan_sides[lane] = pan_side;
        self.amp_env.trigger_lane(lane);
        self.filter_env.trigger_lane(lane);
        self.aux_env.trigger_lane(lane);

        if reset_key_synced_lfos {
            for lfo in &mut self.lfos {
                if lfo.key_sync() {
                    lfo.reset_all();
                }
            }
        }
        self.oscillators.note_on(lane, self.note_frequencies_hz());
        self.filter.reset_lane(lane);
    }

    pub fn note_off(&mut self, note: u8) {
        for lane in 0..LANES {
            if self.notes[lane] == note && self.gates[lane] {
                self.note_off_lane(lane);
            }
        }
    }

    pub fn note_off_lane(&mut self, lane: usize) {
        self.gates[lane] = false;
        self.amp_env.release_lane(lane);
        self.filter_env.release_lane(lane);
        self.aux_env.release_lane(lane);
    }

    pub fn all_notes_off(&mut self) {
        self.gates = [false; LANES];
        self.amp_env.release_all();
        self.filter_env.release_all();
        self.aux_env.release_all();
    }

    pub fn set_filter_oversampling(&mut self, oversampling: FilterOversampling) {
        self.filter.set_oversampling(oversampling);
    }

    pub fn next(&mut self) -> (f32, f32) {
        self.advance_lfos();
        let velocities = f32x4::new(self.velocities);
        let mut lfo_modulation = self.lfo_modulation();
        let aux_env = self.aux_env.next();
        let aux_velocity_scale = f32x4::splat(1.0 - self.aux_env_velocity_amount)
            + velocities * f32x4::splat(self.aux_env_velocity_amount);
        let aux_signal = aux_env * f32x4::splat(self.aux_env_amount) * aux_velocity_scale;
        apply_destination_modulation(&mut lfo_modulation, self.aux_env_destination, aux_signal);

        let osc = self.oscillators.next(lfo_modulation.oscillators);

        let filter_env = self.filter_env.next();
        let amp = self.amp_env.next();
        let mix = osc.audio;

        let notes = f32x4::new(self.notes.map(|note| note as f32));
        let filtered = self.filter.process(
            mix,
            notes,
            filter_env,
            velocities,
            osc.osc1,
            lfo_modulation.filter_cutoff_semitones,
            lfo_modulation.filter_resonance,
            lfo_modulation.filter_audio_mod,
            self.sample_rate,
        );

        let velocity_gain =
            f32x4::splat(1.0 - self.amp_velocity_amount) + velocities * self.amp_velocity_amount;
        let env_gain = amp * self.amp_env_amount;
        let amp_lfo_gain = (f32x4::splat(1.0) + lfo_modulation.amp_gain)
            .clamp(f32x4::splat(0.0), f32x4::splat(2.0));
        let output = filtered * velocity_gain * env_gain * amp_lfo_gain;

        self.pan_lanes(output, lfo_modulation.pan)
    }

    fn advance_lfos(&mut self) {
        let mut rate_mod = [0.0; 4];
        let mut depth_mod = [0.0; 4];
        for (output, destination) in self.last_lfo_outputs.iter().zip(self.lfo_destinations) {
            let value = average_lanes(*output);
            match destination {
                LfoDestination::Lfo1Frequency => rate_mod[0] += value,
                LfoDestination::Lfo2Frequency => rate_mod[1] += value,
                LfoDestination::Lfo3Frequency => rate_mod[2] += value,
                LfoDestination::Lfo4Frequency => rate_mod[3] += value,
                LfoDestination::LfoAllFrequency => {
                    for rate_mod in &mut rate_mod {
                        *rate_mod += value;
                    }
                }
                LfoDestination::Lfo1Amount => depth_mod[0] += value,
                LfoDestination::Lfo2Amount => depth_mod[1] += value,
                LfoDestination::Lfo3Amount => depth_mod[2] += value,
                LfoDestination::Lfo4Amount => depth_mod[3] += value,
                LfoDestination::LfoAllAmount => {
                    for depth_mod in &mut depth_mod {
                        *depth_mod += value;
                    }
                }
                _ => {}
            }
        }

        let rates = (f32x4::new(self.lfo_base_rates_hz)
            * (f32x4::new(rate_mod) * f32x4::splat(4.0)).exp2())
        .to_array();
        let depths = (f32x4::new(self.lfo_base_depths) + f32x4::new(depth_mod))
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0))
            .to_array();

        for (index, lfo) in self.lfos.iter_mut().enumerate() {
            let rate = rates[index];
            let depth = depths[index];
            lfo.set_rate_hz(rate);
            lfo.set_depth(depth);
            self.last_lfo_outputs[index] = lfo.next();
        }
    }

    fn lfo_modulation(&self) -> LfoModulation {
        let mut modulation = LfoModulation::default();
        for (output, destination) in self.last_lfo_outputs.iter().zip(self.lfo_destinations) {
            apply_destination_modulation(&mut modulation, destination, *output);
        }
        modulation
    }

    fn note_frequencies_hz(&self) -> f32x4 {
        f32x4::new(self.notes.map(midi_to_hz))
    }

    fn pan_lanes(&self, lanes: f32x4, pan_mod: f32x4) -> (f32, f32) {
        let spread =
            (f32x4::splat(self.pan_spread) + pan_mod).clamp(f32x4::splat(-1.0), f32x4::splat(1.0));
        let position = if self.active_lane_count() <= 1 {
            f32x4::splat(0.0)
        } else {
            f32x4::new(self.pan_sides)
        };
        let angle =
            (position * spread + f32x4::splat(1.0)) * f32x4::splat(core::f32::consts::FRAC_PI_4);
        let (sin, cos) = angle.sin_cos();

        ((lanes * cos).reduce_add(), (lanes * sin).reduce_add())
    }

    pub fn is_lane_silent(&self, lane: usize) -> bool {
        !self.gates[lane] && self.amp_env.is_idle_lane(lane)
    }

    pub fn is_lane_released(&self, lane: usize) -> bool {
        !self.gates[lane]
    }

    pub fn for_each_active_note(&self, mut f: impl FnMut(u8)) {
        for lane in 0..LANES {
            if self.gates[lane] {
                f(self.notes[lane]);
            }
        }
    }

    pub fn active_lane_count(&self) -> usize {
        (0..LANES)
            .filter(|&lane| !self.is_lane_silent(lane))
            .count()
    }

    pub fn age_active_lanes(&mut self) {
        for lane in 0..LANES {
            if self.gates[lane] {
                self.ages[lane] += 1;
            }
        }
    }

    pub fn oldest_lane(&self) -> usize {
        self.ages
            .iter()
            .enumerate()
            .max_by_key(|(_, age)| *age)
            .map(|(lane, _)| lane)
            .unwrap_or(0)
    }

    pub fn set_osc1_note_param(&mut self, note_param: f32) {
        self.oscillators.set_osc1_frequency_semitones(note_param);
    }

    pub fn set_osc2_note_param(&mut self, note_param: f32) {
        self.oscillators.set_osc2_frequency_semitones(note_param);
    }

    pub fn set_osc1_fine(&mut self, cents: f32) {
        self.oscillators.set_osc1_fine_tune_cents(cents);
    }

    pub fn set_osc2_fine(&mut self, cents: f32) {
        self.oscillators.set_osc2_fine_tune_cents(cents);
    }

    pub fn set_amp_attack(&mut self, seconds: f32) {
        self.amp_env.set_attack_seconds(seconds);
    }

    pub fn set_amp_delay(&mut self, seconds: f32) {
        self.amp_env.set_delay_seconds(seconds);
    }

    pub fn set_amp_decay(&mut self, seconds: f32) {
        self.amp_env.set_decay_seconds(seconds);
    }

    pub fn set_amp_sustain(&mut self, sustain: f32) {
        self.amp_env.set_sustain_level(sustain);
    }

    pub fn set_amp_release(&mut self, seconds: f32) {
        self.amp_env.set_release_seconds(seconds);
    }

    pub fn set_amp_env_amount(&mut self, amount: f32) {
        self.amp_env_amount = amount.clamp(0.0, 1.0);
    }

    pub fn set_amp_velocity_amount(&mut self, amount: f32) {
        self.amp_velocity_amount = amount.clamp(0.0, 1.0);
    }

    pub fn set_pan_spread(&mut self, spread: f32) {
        self.pan_spread = spread.clamp(0.0, 1.0);
    }

    pub fn set_lfo_rate_hz(&mut self, index: usize, rate_hz: f32) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            self.lfo_base_rates_hz[index] = rate_hz;
            lfo.set_rate_hz(rate_hz);
        }
    }

    pub fn set_lfo_depth(&mut self, index: usize, depth: f32) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            self.lfo_base_depths[index] = depth.clamp(0.0, 1.0);
            lfo.set_depth(depth);
        }
    }

    pub fn set_lfo_waveform(&mut self, index: usize, waveform: LfoWaveform) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_waveform(waveform);
        }
    }

    pub fn set_lfo_destination(&mut self, index: usize, destination: LfoDestination) {
        if let Some(slot) = self.lfo_destinations.get_mut(index) {
            *slot = destination;
        }
    }

    pub fn set_lfo_clock_sync(&mut self, index: usize, clock_sync: bool) {
        if let Some(slot) = self.lfo_clock_sync.get_mut(index) {
            *slot = clock_sync;
        }
    }

    pub fn set_lfo_key_sync(&mut self, index: usize, key_sync: bool) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_key_sync(key_sync);
        }
    }

    pub fn set_filter_delay(&mut self, seconds: f32) {
        self.filter_env.set_delay_seconds(seconds);
    }

    pub fn set_filter_attack(&mut self, seconds: f32) {
        self.filter_env.set_attack_seconds(seconds);
    }

    pub fn set_filter_decay(&mut self, seconds: f32) {
        self.filter_env.set_decay_seconds(seconds);
    }

    pub fn set_filter_sustain(&mut self, sustain: f32) {
        self.filter_env.set_sustain_level(sustain);
    }

    pub fn set_filter_release(&mut self, seconds: f32) {
        self.filter_env.set_release_seconds(seconds);
    }

    pub fn set_aux_destination(&mut self, destination: LfoDestination) {
        self.aux_env_destination = destination;
    }

    pub fn set_aux_amount(&mut self, amount: f32) {
        self.aux_env_amount = amount.clamp(-1.0, 1.0);
    }

    pub fn set_aux_velocity_amount(&mut self, amount: f32) {
        self.aux_env_velocity_amount = amount.clamp(0.0, 1.0);
    }

    pub fn set_aux_delay(&mut self, seconds: f32) {
        self.aux_env.set_delay_seconds(seconds);
    }

    pub fn set_aux_attack(&mut self, seconds: f32) {
        self.aux_env.set_attack_seconds(seconds);
    }

    pub fn set_aux_decay(&mut self, seconds: f32) {
        self.aux_env.set_decay_seconds(seconds);
    }

    pub fn set_aux_sustain(&mut self, sustain: f32) {
        self.aux_env.set_sustain_level(sustain);
    }

    pub fn set_aux_release(&mut self, seconds: f32) {
        self.aux_env.set_release_seconds(seconds);
    }

    pub fn set_aux_repeat(&mut self, repeat: bool) {
        self.aux_env.set_loop_enabled(repeat);
    }
}

#[derive(Default)]
struct LfoModulation {
    oscillators: OscillatorModulation,
    filter_cutoff_semitones: f32x4,
    filter_resonance: f32x4,
    filter_audio_mod: f32x4,
    amp_gain: f32x4,
    pan: f32x4,
}

fn apply_destination_modulation(
    modulation: &mut LfoModulation,
    destination: LfoDestination,
    signal: f32x4,
) {
    match destination {
        LfoDestination::Off => {}
        LfoDestination::Osc1Frequency => {
            modulation.oscillators.osc1_frequency_semitones +=
                signal * f32x4::splat(LFO_PITCH_DEPTH_SEMITONES);
        }
        LfoDestination::Osc2Frequency => {
            modulation.oscillators.osc2_frequency_semitones +=
                signal * f32x4::splat(LFO_PITCH_DEPTH_SEMITONES);
        }
        LfoDestination::OscAllFrequency => {
            let pitch = signal * f32x4::splat(LFO_PITCH_DEPTH_SEMITONES);
            modulation.oscillators.osc1_frequency_semitones += pitch;
            modulation.oscillators.osc2_frequency_semitones += pitch;
        }
        LfoDestination::Osc1Level => modulation.oscillators.osc1_level += signal,
        LfoDestination::OscMix => modulation.oscillators.mix += signal,
        LfoDestination::NoiseLevel => modulation.oscillators.noise_level += signal,
        LfoDestination::SubOscLevel => modulation.oscillators.sub_level += signal,
        LfoDestination::Osc1Shape => modulation.oscillators.osc1_shape += signal,
        LfoDestination::Osc2Shape => modulation.oscillators.osc2_shape += signal,
        LfoDestination::OscAllShape => {
            modulation.oscillators.osc1_shape += signal;
            modulation.oscillators.osc2_shape += signal;
        }
        LfoDestination::FilterCutoff => {
            modulation.filter_cutoff_semitones += signal * f32x4::splat(LFO_CUTOFF_DEPTH_SEMITONES);
        }
        LfoDestination::FilterResonance => modulation.filter_resonance += signal,
        LfoDestination::FilterAudioMod => modulation.filter_audio_mod += signal,
        LfoDestination::Vca => modulation.amp_gain += signal,
        LfoDestination::Pan => modulation.pan += signal,
        _ => {}
    }
}

fn average_lanes(value: f32x4) -> f32 {
    value.reduce_add() / LANES as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voices::Voices;
    use crate::{ControlMessage, ParamId};
    use std::vec::Vec;

    fn stereo_rms(voices: &mut Voices, frames: usize) -> (f32, f32) {
        let mut left_sum = 0.0;
        let mut right_sum = 0.0;
        for _ in 0..frames {
            let (left, right) = voices.next();
            left_sum += left * left;
            right_sum += right * right;
        }
        (
            (left_sum / frames as f32).sqrt(),
            (right_sum / frames as f32).sqrt(),
        )
    }

    fn process_frames(voices: &mut Voices, frames: usize) {
        for _ in 0..frames {
            voices.next();
        }
    }

    #[test]
    fn pan_spread_creates_stereo_separation() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 1.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        let mut left_sum = 0.0;
        let mut right_sum = 0.0;
        let mut diff_sum = 0.0;
        let frames = 4096;
        for _ in 0..frames {
            let (left, right) = voices.next();
            left_sum += left * left;
            right_sum += right * right;
            let diff = left - right;
            diff_sum += diff * diff;
        }
        let left = (left_sum / frames as f32).sqrt();
        let right = (right_sum / frames as f32).sqrt();
        let difference = (diff_sum / frames as f32).sqrt();

        assert!(
            difference > left.max(right) * 0.5,
            "two voices should create stereo difference at full spread, left {left}, right {right}, diff {difference}"
        );
    }

    #[test]
    fn pan_spread_keeps_single_voice_centered() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 1.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut left_sum = 0.0;
        let mut right_sum = 0.0;
        let frames = 4096;
        for _ in 0..frames {
            let (left, right) = voices.next();
            left_sum += left * left;
            right_sum += right * right;
        }
        let left = (left_sum / frames as f32).sqrt();
        let right = (right_sum / frames as f32).sqrt();

        assert!(
            (left - right).abs() < left.max(right) * 0.05,
            "one voice should stay centered even at full spread, left {left}, right {right}"
        );
    }

    #[test]
    fn pan_lfo_modulates_spread_width_instead_of_offset() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 0.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Lfo1Waveform, 3.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Lfo1Depth, 1.0));
        voices.handle_control(ControlMessage::SetParam(
            ParamId::Lfo1Destination,
            LfoDestination::Pan.index() as f32,
        ));

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        let mut left_sum = 0.0;
        let mut right_sum = 0.0;
        let frames = 4096;
        for _ in 0..frames {
            let (left, right) = voices.next();
            left_sum += left * left;
            right_sum += right * right;
        }
        let left = (left_sum / frames as f32).sqrt();
        let right = (right_sum / frames as f32).sqrt();

        assert!(
            (left - right).abs() < left.max(right) * 0.1,
            "positive pan modulation should widen alternating voices symmetrically, left {left}, right {right}"
        );
    }

    #[test]
    fn pan_spread_keeps_repeated_single_notes_centered_after_release() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::AmpEgRelease, 0.0005));

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        let (first_left, first_right) = stereo_rms(&mut voices, 2048);
        voices.handle_control(ControlMessage::NoteOff { note: 60 });
        process_frames(&mut voices, 512);

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        let (second_left, second_right) = stereo_rms(&mut voices, 2048);

        assert!(
            (first_left - first_right).abs() < first_left.max(first_right) * 0.05,
            "first single note should stay centered at full spread, left {first_left}, right {first_right}"
        );
        assert!(
            (second_left - second_right).abs() < second_left.max(second_right) * 0.05,
            "second single note should stay centered at full spread, left {second_left}, right {second_right}"
        );
    }

    #[test]
    fn oscillator_tuning_param_does_not_replace_midi_note() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1Frequency, 72.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });

        let block = voices
            .iter()
            .find(|block| block.gates.iter().any(|gate| *gate))
            .unwrap();
        let lane = block.gates.iter().position(|gate| *gate).unwrap();
        let expected = crate::midi_to_hz(76);
        let osc1_freq = block.oscillators.osc1_frequency_hz().to_array()[lane];
        assert_eq!(block.notes[lane], 64);
        assert!(
            (osc1_freq - expected).abs() < 0.1,
            "osc1 should track MIDI note + tuning offset, got {} expected {expected}",
            osc1_freq
        );
    }

    #[test]
    fn oscillator_frequency_and_fine_tune_use_natural_units_and_clamp() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1Frequency, 240.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1FineTune, 99.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let block = voices
            .iter()
            .find(|block| block.gates.iter().any(|gate| *gate))
            .unwrap();
        let lane = block.gates.iter().position(|gate| *gate).unwrap();
        let expected = crate::midi_to_hz(120) * 2.0f32.powf(50.0 / 1200.0);
        let osc1_freq = block.oscillators.osc1_frequency_hz().to_array()[lane];

        assert!(
            (osc1_freq - expected).abs() < 0.5,
            "osc1 frequency should clamp to 120 semitones and +50 cents, got {osc1_freq}, expected {expected}"
        );
    }

    #[test]
    fn osc_mix_is_canonical_balance_control() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1Enabled, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.25));

        let params = voices[0].oscillators.params();
        assert_eq!(params.osc_mix, 0.25);
    }

    #[test]
    fn osc_slop_zero_is_stable_and_full_slop_offsets_lanes() {
        let mut stable = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        for note in [60, 64, 67, 72] {
            stable.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        let stable_block = &stable[0];
        for lane in 0..LANES {
            let expected = crate::midi_to_hz(stable_block.notes[lane]);
            let freq = stable_block.oscillators.osc1_frequency_hz().to_array()[lane];
            assert!(
                (freq - expected).abs() < 0.1,
                "slop 0 should not detune lane {lane}, got {freq}, expected {expected}"
            );
        }

        let mut sloppy = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        sloppy.handle_control(ControlMessage::SetParam(ParamId::OscSlop, 1.0));
        for note in [60, 64, 67, 72] {
            sloppy.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        let sloppy_block = &sloppy[0];
        let offsets: Vec<f32> = (0..LANES)
            .map(|lane| {
                let expected = crate::midi_to_hz(sloppy_block.notes[lane]);
                sloppy_block.oscillators.osc1_frequency_hz().to_array()[lane] - expected
            })
            .collect();

        assert!(
            offsets.iter().any(|offset| offset.abs() > 0.01),
            "full slop should offset at least one lane, offsets {offsets:?}"
        );
    }

    #[test]
    fn clearing_osc_slop_restores_intended_frequency() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::OscSlop, 1.0));
        for note in [60, 64, 67, 72] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        voices.handle_control(ControlMessage::SetParam(ParamId::OscSlop, 0.0));
        let block = &voices[0];
        for lane in 0..LANES {
            let expected = crate::midi_to_hz(block.notes[lane]);
            let freq = block.oscillators.osc1_frequency_hz().to_array()[lane];
            assert!(
                (freq - expected).abs() < 0.1,
                "clearing slop should restore lane {lane}, got {freq}, expected {expected}"
            );
        }
    }

    #[test]
    fn note_reset_flags_are_routed_to_oscillators() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1NoteReset, 0.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc2NoteReset, 1.0));

        let params = voices[0].oscillators.params();
        assert!(!params.osc1.note_reset);
        assert!(params.osc2.note_reset);
    }

    #[test]
    fn hard_sync_param_is_routed_to_oscillators() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::HardSync, 1.0));

        assert!(voices[0].oscillators.params().sync);
    }

    #[test]
    fn aux_envelope_to_oscillator_frequency_modulates_pitch() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(
            ParamId::AuxEgDestination,
            LfoDestination::Osc1Frequency.index() as f32,
        ));
        voices.handle_control(ControlMessage::SetParam(ParamId::AuxEgAmount, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::AuxEgAttack, 0.0005));
        voices.handle_control(ControlMessage::SetParam(ParamId::AuxEgDecay, 5.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::AuxEgSustain, 1.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        process_frames(&mut voices, 32);

        let block = &voices[0];
        let freq = block.oscillators.osc1_frequency_hz().to_array()[0];
        let expected = crate::midi_to_hz(72);
        assert!(
            (freq - expected).abs() < 1.0,
            "full positive aux pitch modulation should raise osc1 by about one octave, got {freq}, expected {expected}"
        );
    }

    #[test]
    fn aux_repeat_keeps_envelope_cycling_while_held() {
        let mut repeating = Voices::<{ crate::VOICE_PACKS }>::new(1_000.0);
        repeating.handle_control(ControlMessage::SetParam(
            ParamId::AuxEgDestination,
            LfoDestination::FilterCutoff.index() as f32,
        ));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgAmount, 1.0));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgAttack, 0.001));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgDecay, 0.001));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgSustain, 0.5));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgRelease, 0.001));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgLoop, 1.0));
        repeating.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let first = repeating[0].aux_env.next().to_array()[0];
        let reset = repeating[0].aux_env.next().to_array()[0];
        let second = repeating[0].aux_env.next().to_array()[0];

        assert!(first > 0.9);
        assert_eq!(reset, 0.0);
        assert!(second > 0.9);

        repeating.handle_control(ControlMessage::NoteOff { note: 60 });
        assert_eq!(repeating[0].aux_env.next().to_array()[0], 0.0);
        assert!(repeating[0].aux_env.is_idle_lane(0));
    }
}
