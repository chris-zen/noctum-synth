//! Dual-oscillator mixer with sub oscillator, noise, sync, and glide.

use crate::f32x4;

use crate::{AnalogOscillator, AnalogSubOscillator, Waveform, WhiteNoise, midi_to_hz};

const CENTER_FREQUENCY_SEMITONES: f32 = 60.0;

/// Oscillator section for one [`crate::VoiceBlock`]: two analog oscillators, sub, and noise.
pub struct Oscillators {
    osc1: AnalogOscillator,
    osc2: AnalogOscillator,
    sub_osc: AnalogSubOscillator,
    noise: WhiteNoise,
    params: OscillatorsParams,
    note_frequency_hz: f32x4,
    last_frequency_modulation: [f32x4; 2],
    last_shape_modulation: [f32x4; 2],
    sample_rate: f32,
}

impl Oscillators {
    pub fn new(sample_rate: f32) -> Self {
        let mut oscillators = Self {
            osc1: AnalogOscillator::new(sample_rate),
            osc2: AnalogOscillator::new(sample_rate),
            sub_osc: AnalogSubOscillator::default(),
            noise: WhiteNoise::default(),
            params: OscillatorsParams::default(),
            note_frequency_hz: f32x4::splat(0.0),
            last_frequency_modulation: [f32x4::splat(0.0); 2],
            last_shape_modulation: [f32x4::splat(0.0); 2],
            sample_rate,
        };
        oscillators.apply_params_without_frequency();
        oscillators
    }

    pub fn params(&self) -> &OscillatorsParams {
        &self.params
    }

    pub fn osc1_frequency_hz(&self) -> f32x4 {
        self.osc1.frequency_hz()
    }

    pub fn set_osc1_enabled(&mut self, enabled: bool) {
        self.params.osc1.enabled = enabled;
        self.osc1.set_enabled(enabled);
    }

    pub fn set_osc2_enabled(&mut self, enabled: bool) {
        self.params.osc2.enabled = enabled;
        self.osc2.set_enabled(enabled);
    }

    pub fn set_osc1_waveform(&mut self, waveform: Waveform) {
        self.params.osc1.waveform = waveform;
        self.osc1.set_waveform(waveform);
        apply_shape_mod(&mut self.osc1, &self.params.osc1);
        self.last_shape_modulation[0] = f32x4::splat(0.0);
    }

    pub fn set_osc2_waveform(&mut self, waveform: Waveform) {
        self.params.osc2.waveform = waveform;
        self.osc2.set_waveform(waveform);
        apply_shape_mod(&mut self.osc2, &self.params.osc2);
        self.last_shape_modulation[1] = f32x4::splat(0.0);
    }

    pub fn set_osc1_frequency_semitones(&mut self, semitones: f32) {
        self.params.osc1.frequency_semitones = semitones.clamp(0.0, 120.0);
        self.update_frequencies();
    }

    pub fn set_osc2_frequency_semitones(&mut self, semitones: f32) {
        self.params.osc2.frequency_semitones = semitones.clamp(0.0, 120.0);
        self.update_frequencies();
    }

    pub fn set_osc1_fine_tune_cents(&mut self, cents: f32) {
        self.params.osc1.fine_tune_cents = cents.clamp(-50.0, 50.0);
        self.update_frequencies();
    }

    pub fn set_osc2_fine_tune_cents(&mut self, cents: f32) {
        self.params.osc2.fine_tune_cents = cents.clamp(-50.0, 50.0);
        self.update_frequencies();
    }

    pub fn set_osc1_shape_mod(&mut self, shape_mod: f32) {
        self.params.osc1.shape_mod = shape_mod.clamp(0.0, 1.0);
        apply_shape_mod(&mut self.osc1, &self.params.osc1);
        self.last_shape_modulation[0] = f32x4::splat(0.0);
    }

    pub fn set_osc2_shape_mod(&mut self, shape_mod: f32) {
        self.params.osc2.shape_mod = shape_mod.clamp(0.0, 1.0);
        apply_shape_mod(&mut self.osc2, &self.params.osc2);
        self.last_shape_modulation[1] = f32x4::splat(0.0);
    }

    pub fn set_osc1_note_reset(&mut self, note_reset: bool) {
        self.params.osc1.note_reset = note_reset;
    }

    pub fn set_osc2_note_reset(&mut self, note_reset: bool) {
        self.params.osc2.note_reset = note_reset;
    }

    pub fn set_osc1_keyboard_on(&mut self, keyboard_on: bool) {
        self.params.osc1.keyboard_on = keyboard_on;
        self.update_frequencies();
    }

    pub fn set_osc2_keyboard_on(&mut self, keyboard_on: bool) {
        self.params.osc2.keyboard_on = keyboard_on;
        self.update_frequencies();
    }

    pub fn set_sync(&mut self, sync: bool) {
        self.params.sync = sync;
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.params.osc_mix = mix.clamp(0.0, 1.0);
    }

    pub fn set_sub_octave(&mut self, level: f32) {
        self.params.sub_octave = level.clamp(0.0, 1.0);
    }

    pub fn set_noise(&mut self, level: f32) {
        self.params.noise = level.clamp(0.0, 1.0);
    }

    pub fn set_slop(&mut self, slop: f32) {
        self.params.osc_slop = slop.clamp(0.0, 1.0);
        self.osc1.set_slop_amount(self.params.osc_slop);
        self.osc2.set_slop_amount(self.params.osc_slop);
    }

    pub fn set_note_frequency(&mut self, note_frequency_hz: f32x4) {
        self.note_frequency_hz = note_frequency_hz;
        self.update_frequencies();
    }

    pub fn note_on(&mut self, lane: usize, note_frequency_hz: f32x4) {
        self.note_frequency_hz = note_frequency_hz;
        self.update_frequencies();
        self.osc1.trigger_lane(lane, self.params.osc1.note_reset);
        self.osc2.trigger_lane(lane, self.params.osc2.note_reset);
        if self.params.osc1.note_reset {
            self.sub_osc.reset_lane(lane);
        }
    }

    pub fn update_frequencies(&mut self) {
        self.update_frequencies_modulated(f32x4::splat(0.0), f32x4::splat(0.0));
    }

    pub fn next(&mut self, modulation: OscillatorModulation) -> OscillatorsOutput {
        let frequency_modulation = [
            modulation.osc1_frequency_semitones,
            modulation.osc2_frequency_semitones,
        ];
        if frequency_modulation != self.last_frequency_modulation {
            self.update_frequencies_modulated(frequency_modulation[0], frequency_modulation[1]);
        }

        let shape_modulation = [modulation.osc1_shape, modulation.osc2_shape];
        if shape_modulation[0] != self.last_shape_modulation[0] {
            self.osc1.set_shape(modulated_scalar_shape(
                self.params.osc1.shape_mod,
                shape_modulation[0],
            ));
        }
        if shape_modulation[1] != self.last_shape_modulation[1] {
            self.osc2.set_shape(modulated_scalar_shape(
                self.params.osc2.shape_mod,
                shape_modulation[1],
            ));
        }
        self.last_shape_modulation = shape_modulation;

        let osc2_step = if self.params.osc2.enabled {
            self.osc2.next_step()
        } else {
            crate::analog_oscillator::OscillatorStep {
                output: f32x4::splat(0.0),
                wrapped: [false; crate::LANES],
                wrap_phase_fraction: [0.0; crate::LANES],
            }
        };
        if self.params.sync && self.params.osc2.enabled {
            self.osc1
                .sync_reset_lanes_at(osc2_step.wrapped, osc2_step.wrap_phase_fraction);
        }
        let osc1 = self.osc1.next();
        let osc2 = osc2_step.output;
        self.sub_osc
            .set_frequency(self.osc1.frequency_hz(), self.sample_rate);
        let sub_level = (f32x4::splat(self.params.sub_octave) + modulation.sub_level)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let noise_level = (f32x4::splat(self.params.noise) + modulation.noise_level)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let sub = if sub_level == f32x4::ZERO {
            f32x4::splat(0.0)
        } else {
            self.sub_osc.next() * sub_level
        };
        let noise = if noise_level == f32x4::ZERO {
            f32x4::splat(0.0)
        } else {
            self.noise.next() * noise_level
        };
        let mix = (f32x4::splat(self.params.osc_mix) + modulation.mix)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let osc1_gain = (f32x4::splat(1.0) - mix + modulation.osc1_level)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let osc2_gain = (mix + modulation.osc2_level).clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let audio = osc1 * osc1_gain + osc2 * osc2_gain + sub + noise;

        OscillatorsOutput {
            osc1,
            osc2,
            sub,
            noise,
            audio,
        }
    }

    fn apply_params_without_frequency(&mut self) {
        self.osc1.set_enabled(self.params.osc1.enabled);
        self.osc2.set_enabled(self.params.osc2.enabled);
        self.osc1.set_waveform(self.params.osc1.waveform);
        self.osc2.set_waveform(self.params.osc2.waveform);
        apply_shape_mod(&mut self.osc1, &self.params.osc1);
        apply_shape_mod(&mut self.osc2, &self.params.osc2);
    }

    fn update_frequencies_modulated(
        &mut self,
        osc1_frequency_mod_semitones: f32x4,
        osc2_frequency_mod_semitones: f32x4,
    ) {
        self.last_frequency_modulation =
            [osc1_frequency_mod_semitones, osc2_frequency_mod_semitones];
        let osc1 = oscillator_frequency(
            self.note_frequency_hz,
            &self.params.osc1,
            osc1_frequency_mod_semitones,
        );
        let osc2 = oscillator_frequency(
            self.note_frequency_hz,
            &self.params.osc2,
            osc2_frequency_mod_semitones,
        );
        self.osc1.set_frequency(osc1);
        self.osc2.set_frequency(osc2);
    }
}

pub fn osc_mix_to_gains(mix: f32) -> (f32, f32) {
    let osc2 = mix.clamp(0.0, 1.0);
    (1.0 - osc2, osc2)
}

/// Per-lane modulation offsets applied before the oscillator section renders.
#[derive(Clone, Copy)]
pub struct OscillatorModulation {
    pub osc1_frequency_semitones: f32x4,
    pub osc2_frequency_semitones: f32x4,
    pub osc1_shape: f32x4,
    pub osc2_shape: f32x4,
    pub sub_level: f32x4,
    pub noise_level: f32x4,
    pub mix: f32x4,
    pub osc1_level: f32x4,
    pub osc2_level: f32x4,
}

impl Default for OscillatorModulation {
    fn default() -> Self {
        Self {
            osc1_frequency_semitones: f32x4::splat(0.0),
            osc2_frequency_semitones: f32x4::splat(0.0),
            osc1_shape: f32x4::splat(0.0),
            osc2_shape: f32x4::splat(0.0),
            sub_level: f32x4::splat(0.0),
            noise_level: f32x4::splat(0.0),
            mix: f32x4::splat(0.0),
            osc1_level: f32x4::splat(0.0),
            osc2_level: f32x4::splat(0.0),
        }
    }
}

/// Raw per-oscillator outputs and the mixed audio bus for one sample step.
pub struct OscillatorsOutput {
    pub osc1: f32x4,
    pub osc2: f32x4,
    pub sub: f32x4,
    pub noise: f32x4,
    pub audio: f32x4,
}

/// Patch-level settings for the full oscillator section.
#[derive(Debug, Clone)]
pub struct OscillatorsParams {
    pub osc1: OscillatorParams,
    pub osc2: OscillatorParams,
    pub sync: bool,
    pub osc_mix: f32,
    pub sub_octave: f32,
    pub noise: f32,
    pub osc_slop: f32,
}

impl Default for OscillatorsParams {
    fn default() -> Self {
        let mut osc2 = OscillatorParams::default();
        osc2.enabled = false;
        Self {
            osc1: OscillatorParams::default(),
            osc2,
            sync: false,
            osc_mix: 0.0,
            sub_octave: 0.0,
            noise: 0.0,
            osc_slop: 0.0,
        }
    }
}

/// Settings for a single analog oscillator (waveform, tuning, glide, etc.).
#[derive(Debug, Clone)]
pub struct OscillatorParams {
    pub enabled: bool,
    pub waveform: Waveform,
    pub frequency_semitones: f32,
    pub fine_tune_cents: f32,
    pub shape_mod: f32,
    pub keyboard_on: bool,
    pub note_reset: bool,
    pub glide: f32,
}

impl Default for OscillatorParams {
    fn default() -> Self {
        Self {
            enabled: true,
            waveform: Waveform::Saw,
            frequency_semitones: CENTER_FREQUENCY_SEMITONES,
            fine_tune_cents: 0.0,
            shape_mod: 0.0,
            keyboard_on: true,
            note_reset: true,
            glide: 0.0,
        }
    }
}

fn apply_shape_mod(osc: &mut AnalogOscillator, params: &OscillatorParams) {
    let shape_mod = params.shape_mod.clamp(0.0, 1.0);
    osc.set_shape(shape_mod);
}

fn oscillator_frequency(
    note_frequency_hz: f32x4,
    params: &OscillatorParams,
    mod_semitones: f32x4,
) -> f32x4 {
    let keyboard_base = if params.keyboard_on {
        note_frequency_hz
    } else {
        f32x4::splat(midi_to_hz(CENTER_FREQUENCY_SEMITONES as u8))
    };
    let semitone_offset = params.frequency_semitones - CENTER_FREQUENCY_SEMITONES;
    let scalar_semitones = semitone_offset + params.fine_tune_cents / 100.0;
    let total_semitones = f32x4::splat(scalar_semitones) + mod_semitones;
    keyboard_base * (total_semitones * f32x4::splat(1.0 / 12.0)).exp2()
}

fn modulated_scalar_shape(base: f32, modulation: f32x4) -> f32 {
    let average = modulation.reduce_add() * 0.25;
    (base + average).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{OscillatorModulation, Oscillators, Waveform, osc_mix_to_gains};
    use crate::f32x4;
    use crate::midi_to_hz;

    const SAMPLE_RATE: f32 = 44_100.0;

    fn settle(oscillators: &mut Oscillators, frames: usize) {
        for _ in 0..frames {
            oscillators.next(OscillatorModulation::default());
        }
    }

    #[test]
    fn pulse_shape_mod_controls_pwm_from_square() {
        fn positive_ratio(shape_mod: f32) -> f32 {
            let frequency = f32x4::splat(110.0);
            let period = (SAMPLE_RATE / 110.0).round() as usize;
            let mut oscillators = Oscillators::new(SAMPLE_RATE);
            oscillators.set_osc1_waveform(Waveform::Pulse);
            oscillators.set_osc1_shape_mod(shape_mod);
            oscillators.set_note_frequency(frequency);

            let mut positive = 0usize;
            for _ in 0..period {
                let output = oscillators.next(OscillatorModulation::default());
                if output.osc1.to_array()[0] > 0.0 {
                    positive += 1;
                }
            }

            positive as f32 / period as f32
        }

        let square = positive_ratio(0.0);
        let modulated = positive_ratio(1.0);

        assert!(
            (square - 0.5).abs() < 0.05,
            "Pulse shape 0 should be a 50% square, got duty {square:.3}"
        );
        assert!(
            modulated > 0.9,
            "Pulse shape 1 should widen pulse width for PWM, got duty {modulated:.3}"
        );
    }

    #[test]
    fn osc_mix_to_gains_maps_balance_and_clamps() {
        assert_eq!(osc_mix_to_gains(0.0), (1.0, 0.0));
        assert_eq!(osc_mix_to_gains(0.25), (0.75, 0.25));
        assert_eq!(osc_mix_to_gains(1.0), (0.0, 1.0));
        assert_eq!(osc_mix_to_gains(2.0), (0.0, 1.0));
        assert_eq!(osc_mix_to_gains(-1.0), (1.0, 0.0));
    }

    #[test]
    fn default_has_osc2_disabled_and_osc1_only_mix() {
        let oscillators = Oscillators::new(SAMPLE_RATE);
        let params = oscillators.params();

        assert!(params.osc1.enabled);
        assert!(!params.osc2.enabled);
        assert_eq!(params.osc_mix, 0.0);
    }

    #[test]
    fn mix_crossfade_routes_audio_between_oscillators() {
        let frequency = f32x4::splat(440.0);
        let mut oscillators = Oscillators::new(SAMPLE_RATE);
        oscillators.set_note_frequency(frequency);
        oscillators.set_sub_octave(0.0);
        oscillators.set_noise(0.0);
        settle(&mut oscillators, 64);

        oscillators.set_mix(0.0);
        let osc1_only = oscillators.next(OscillatorModulation::default());
        let osc1_audio = osc1_only.audio.to_array()[0];
        let osc1_expected = osc1_only.osc1.to_array()[0];
        assert!(
            (osc1_audio - osc1_expected).abs() < 1e-4,
            "mix 0 should pass only osc1, got {osc1_audio} vs {osc1_expected}"
        );
        assert!(
            osc1_only.osc2.to_array()[0].abs() < 1e-4,
            "default osc2 should be disabled and silent"
        );

        oscillators.set_osc2_enabled(true);
        settle(&mut oscillators, 16);

        oscillators.set_mix(1.0);
        let osc2_only = oscillators.next(OscillatorModulation::default());
        let osc2_audio = osc2_only.audio.to_array()[0];
        let osc2_expected = osc2_only.osc2.to_array()[0];
        assert!(
            (osc2_audio - osc2_expected).abs() < 1e-4,
            "mix 1 should pass only osc2, got {osc2_audio} vs {osc2_expected}"
        );

        oscillators.set_mix(0.35);
        let blended = oscillators.next(OscillatorModulation::default());
        let expected = blended.osc1.to_array()[0] * 0.65 + blended.osc2.to_array()[0] * 0.35;
        assert!(
            (blended.audio.to_array()[0] - expected).abs() < 1e-4,
            "mix 0.35 should crossfade oscillators"
        );
    }

    #[test]
    fn keyboard_off_uses_center_frequency_instead_of_note() {
        let mut oscillators = Oscillators::new(SAMPLE_RATE);
        oscillators.set_note_frequency(f32x4::splat(midi_to_hz(72)));
        oscillators.set_osc1_keyboard_on(false);
        oscillators.update_frequencies();

        let freq = oscillators.osc1_frequency_hz().to_array()[0];
        let expected = midi_to_hz(60);
        assert!(
            (freq - expected).abs() < 0.1,
            "keyboard off should use center pitch, got {freq} expected {expected}"
        );
    }

    #[test]
    fn hard_sync_increases_osc1_wrap_rate_with_faster_osc2() {
        let frequency = f32x4::splat(midi_to_hz(60));
        let samples = 1_200;

        let mut without_sync = Oscillators::new(SAMPLE_RATE);
        without_sync.set_osc2_enabled(true);
        without_sync.set_note_frequency(frequency);
        without_sync.set_osc1_frequency_semitones(48.0);
        without_sync.set_osc2_frequency_semitones(84.0);
        without_sync.set_sync(false);
        let unsynced_gap = max_wrap_gap(&mut without_sync, samples);

        let mut with_sync = Oscillators::new(SAMPLE_RATE);
        with_sync.set_osc2_enabled(true);
        with_sync.set_note_frequency(frequency);
        with_sync.set_osc1_frequency_semitones(48.0);
        with_sync.set_osc2_frequency_semitones(84.0);
        with_sync.set_sync(true);
        let synced_gap = max_wrap_gap(&mut with_sync, samples);

        assert!(
            synced_gap < unsynced_gap / 2,
            "hard sync should cap osc1 wrap spacing to osc2 period, unsynced gap {unsynced_gap}, synced gap {synced_gap}"
        );
    }

    #[test]
    fn hard_sync_does_not_reset_osc1_when_osc2_disabled() {
        let frequency = f32x4::splat(midi_to_hz(60));
        let samples = 400;

        let mut reference = Oscillators::new(SAMPLE_RATE);
        reference.set_osc2_enabled(false);
        reference.set_note_frequency(frequency);
        reference.set_sync(false);
        let reference_gap = max_wrap_gap(&mut reference, samples);

        let mut with_sync = Oscillators::new(SAMPLE_RATE);
        with_sync.set_osc2_enabled(false);
        with_sync.set_note_frequency(frequency);
        with_sync.set_sync(true);
        let synced_gap = max_wrap_gap(&mut with_sync, samples);

        assert_eq!(
            synced_gap, reference_gap,
            "sync with osc2 off should not change osc1 wrap spacing"
        );
    }

    #[test]
    fn modulation_levels_affect_sub_noise_and_mix() {
        let frequency = f32x4::splat(220.0);
        let mut oscillators = Oscillators::new(SAMPLE_RATE);
        oscillators.set_note_frequency(frequency);
        oscillators.set_sub_octave(0.25);
        oscillators.set_noise(0.25);
        oscillators.set_mix(0.0);
        settle(&mut oscillators, 32);

        let base = oscillators.next(OscillatorModulation::default());
        assert!(
            base.sub.to_array()[0].abs() > 0.0,
            "sub octave level should contribute to sub output"
        );
        assert!(
            base.noise.to_array()[0].abs() > 0.0,
            "noise level should contribute to noise output"
        );

        let mut boosted = OscillatorModulation::default();
        boosted.sub_level = f32x4::splat(0.75);
        boosted.noise_level = f32x4::splat(0.75);
        let modulated = oscillators.next(boosted);

        assert!(
            modulated.sub.to_array()[0].abs() > base.sub.to_array()[0].abs(),
            "sub_level modulation should increase sub contribution"
        );
        assert!(
            modulated.noise.to_array()[0].abs() > base.noise.to_array()[0].abs(),
            "noise_level modulation should increase noise contribution"
        );

        oscillators.set_sub_octave(0.0);
        oscillators.set_noise(0.0);
        oscillators.set_osc2_enabled(true);
        settle(&mut oscillators, 16);

        let mut mix_modulation = OscillatorModulation::default();
        mix_modulation.mix = f32x4::splat(1.0);
        let osc2_only = oscillators.next(mix_modulation);
        assert!(
            (osc2_only.audio.to_array()[0] - osc2_only.osc2.to_array()[0]).abs() < 1e-4,
            "mix modulation should push output to osc2 only when sub and noise are off"
        );
    }

    #[test]
    fn note_on_resets_sub_only_when_osc1_note_reset() {
        let frequency = f32x4::splat(220.0);

        let mut reference = Oscillators::new(SAMPLE_RATE);
        reference.set_sub_octave(1.0);
        reference.set_note_frequency(frequency);
        reference.note_on(0, frequency);
        let phase_start = reference
            .next(OscillatorModulation::default())
            .sub
            .to_array()[0];

        let mut with_reset = Oscillators::new(SAMPLE_RATE);
        with_reset.set_sub_octave(1.0);
        with_reset.set_note_frequency(frequency);
        with_reset.set_osc1_note_reset(true);
        with_reset.note_on(0, frequency);
        settle(&mut with_reset, 300);
        with_reset.note_on(0, frequency);
        let after_reset = with_reset
            .next(OscillatorModulation::default())
            .sub
            .to_array()[0];

        let mut without_reset = Oscillators::new(SAMPLE_RATE);
        without_reset.set_sub_octave(1.0);
        without_reset.set_note_frequency(frequency);
        without_reset.set_osc1_note_reset(false);
        without_reset.note_on(0, frequency);
        settle(&mut without_reset, 300);
        without_reset.note_on(0, frequency);
        let after_continue = without_reset
            .next(OscillatorModulation::default())
            .sub
            .to_array()[0];

        assert!(
            (after_reset - phase_start).abs() < 0.05,
            "note reset should restart sub phase, got {after_reset} vs {phase_start}"
        );
        assert!(
            (after_continue - phase_start).abs() > 0.1,
            "without note reset sub phase should continue, got {after_continue} vs {phase_start}"
        );
    }

    #[test]
    fn runtime_shape_modulation_averages_across_lanes() {
        let frequency = f32x4::splat(110.0);
        let period = (SAMPLE_RATE / 110.0).round() as usize;

        let positive_ratio = |shape_mod: f32, shape_modulation: f32| {
            let mut oscillators = Oscillators::new(SAMPLE_RATE);
            oscillators.set_osc1_waveform(Waveform::Pulse);
            oscillators.set_osc1_shape_mod(shape_mod);
            oscillators.set_note_frequency(frequency);

            let mut modulation = OscillatorModulation::default();
            modulation.osc1_shape = f32x4::splat(shape_modulation);

            let mut positive = 0usize;
            for _ in 0..period {
                let output = oscillators.next(modulation);
                if output.osc1.to_array()[0] > 0.0 {
                    positive += 1;
                }
            }

            positive as f32 / period as f32
        };

        let param_only = positive_ratio(0.5, 0.0);
        let modulated = positive_ratio(0.0, 0.5);

        assert!(
            (param_only - modulated).abs() < 0.05,
            "shape param 0.5 and +0.5 lane modulation should match, got {param_only} vs {modulated}"
        );
    }

    fn max_wrap_gap(oscillators: &mut Oscillators, samples: usize) -> usize {
        let mut prev = 0.0f32;
        let mut gap = 0usize;
        let mut max_gap = 0usize;
        for _ in 0..samples {
            let sample = oscillators
                .next(OscillatorModulation::default())
                .osc1
                .to_array()[0];
            if sample < prev - 0.9 {
                max_gap = max_gap.max(gap);
                gap = 0;
            } else {
                gap += 1;
            }
            prev = sample;
        }
        max_gap
    }
}
