use synth_core::{
    DEFAULT_SAMPLE_RATE, EffectType,
    effects::{EffectModulation, Effects, EffectsWithMemory},
};

type TestEffects = Effects<48_000>;

#[test]
fn caller_provided_storage_matches_inline_storage() {
    let mut borrowed_memory = [0.0; 128];
    let mut inline = Effects::<128>::new(DEFAULT_SAMPLE_RATE);
    let mut borrowed =
        EffectsWithMemory::new_with_memory(DEFAULT_SAMPLE_RATE, borrowed_memory.as_mut_slice());

    for effect in [
        &mut inline as &mut dyn EffectSetup,
        &mut borrowed as &mut dyn EffectSetup,
    ] {
        effect.configure_delay();
    }

    for frame in 0..256 {
        let input = if frame == 0 { (0.5, -0.25) } else { (0.0, 0.0) };
        assert_eq!(
            inline.next(input.0, input.1, EffectModulation::default(), None),
            borrowed.next(input.0, input.1, EffectModulation::default(), None),
        );
    }
}

trait EffectSetup {
    fn configure_delay(&mut self);
}

impl<Memory> EffectSetup for EffectsWithMemory<Memory>
where
    Memory: AsRef<[f32]> + AsMut<[f32]>,
{
    fn configure_delay(&mut self) {
        self.set_enabled(true);
        self.set_type(EffectType::DelayMono);
        self.set_mix(0.7);
        self.set_param1(0.2);
        self.set_param2(0.3);
    }
}

#[test]
fn set_params_updates_the_selected_effect_as_one_value() {
    let mut effects = Effects::<64>::new(DEFAULT_SAMPLE_RATE);
    let params = synth_core::EffectParams {
        enabled: true,
        effect_type: EffectType::BucketBrigadeDelay,
        mix: 0.6,
        clock_sync: true,
        param1: 0.25,
        param2: 0.75,
    };

    effects.set_params(params);

    let applied = effects.params();
    assert_eq!(applied.enabled, params.enabled);
    assert_eq!(applied.effect_type, params.effect_type);
    assert_eq!(applied.mix, params.mix);
    assert_eq!(applied.clock_sync, params.clock_sync);
    assert_eq!(applied.param1, params.param1);
    assert_eq!(applied.param2, params.param2);
}

#[test]
fn tempo_is_initialized_and_clamped() {
    let mut effects = Effects::<8>::new(DEFAULT_SAMPLE_RATE);
    assert_eq!(effects.tempo_bpm(), synth_core::DEFAULT_TEMPO_BPM);

    effects.set_tempo_bpm(20.0);
    assert_eq!(effects.tempo_bpm(), 30.0);
    effects.set_tempo_bpm(300.0);
    assert_eq!(effects.tempo_bpm(), 250.0);
}

fn render_silence_peak(effect: &mut TestEffects, frames: usize) -> f32 {
    let mut peak = 0.0f32;
    for _ in 0..frames {
        let (left, right) = effect.next(0.0, 0.0, EffectModulation::default(), None);
        peak = peak.max(left.abs()).max(right.abs());
    }
    peak
}

fn stereo_energy_and_difference(effect: &mut TestEffects, frames: usize) -> (f32, f32) {
    let mut energy = 0.0;
    let mut difference = 0.0;
    for _ in 0..frames {
        let (left, right) = effect.next(0.0, 0.0, EffectModulation::default(), None);
        energy += left * left + right * right;
        let delta = left - right;
        difference += delta * delta;
    }
    (energy.sqrt(), difference.sqrt())
}

#[test]
fn delay_time_changes_do_not_jump_to_the_new_read_head_immediately() {
    let mut effect = TestEffects::new(DEFAULT_SAMPLE_RATE);
    effect.set_enabled(true);
    effect.set_type(EffectType::DelayMono);
    effect.set_mix(1.0);
    effect.set_param1(0.1);
    effect.set_param2(0.0);

    let mut previous = 0.0;
    for index in 0..DEFAULT_SAMPLE_RATE as usize {
        let input = -0.8 + 1.6 * (index as f32 / (DEFAULT_SAMPLE_RATE - 1.0));
        previous = effect
            .next(input, input, EffectModulation::default(), None)
            .0;
    }

    effect.set_param1(0.95);
    let next = effect.next(0.8, 0.8, EffectModulation::default(), None).0;

    assert!(
        (next - previous).abs() < 0.2,
        "delay time changes should be smoothed, previous {previous}, next {next}"
    );
}

#[test]
fn delay_time_can_be_moved_repeatedly_without_large_spikes() {
    let mut effect = TestEffects::new(DEFAULT_SAMPLE_RATE);
    effect.set_enabled(true);
    effect.set_type(EffectType::DdlStereo);
    effect.set_mix(1.0);
    effect.set_param1(0.1);
    effect.set_param2(0.65);

    let mut peak = 0.0f32;
    for index in 0..24_000 {
        if index % 256 == 0 {
            let value = if (index / 256) % 2 == 0 { 0.08 } else { 0.92 };
            effect.set_param1(value);
        }
        let input = if index < 4096 { 0.35 } else { 0.0 };
        let (left, right) = effect.next(input, -input, EffectModulation::default(), None);
        peak = peak.max(left.abs()).max(right.abs());
    }

    assert!(
        peak.is_finite() && peak < 1.0,
        "moving delay time repeatedly should stay bounded, peak {peak}"
    );
}

#[test]
fn changing_effect_type_clears_delay_memory() {
    let mut effect = TestEffects::new(DEFAULT_SAMPLE_RATE);
    effect.set_enabled(true);
    effect.set_type(EffectType::DelayMono);
    effect.set_mix(1.0);
    effect.set_param1(0.02);
    effect.set_param2(0.75);

    for index in 0..4096 {
        let input = if index < 256 { 0.7 } else { 0.0 };
        effect.next(input, input, EffectModulation::default(), None);
    }

    let tail_peak = render_silence_peak(&mut effect, 4096);
    assert!(
        tail_peak > 0.001,
        "test setup should leave an audible delay tail, peak {tail_peak}"
    );

    effect.set_type(EffectType::Distortion);
    effect.set_type(EffectType::DelayMono);

    let stale_peak = render_silence_peak(&mut effect, 4096);
    assert!(
        stale_peak < 1.0e-6,
        "switching away and back should clear stale delay memory, peak {stale_peak}"
    );
}

#[test]
fn reverb_tail_is_diffuse_stereo_and_decays() {
    let mut effect = TestEffects::new(DEFAULT_SAMPLE_RATE);
    effect.set_enabled(true);
    effect.set_type(EffectType::Reverb);
    effect.set_mix(1.0);
    effect.set_param1(0.75);
    effect.set_param2(0.6);

    for index in 0..2048 {
        let input = if index < 128 { 0.8 } else { 0.0 };
        effect.next(input, input, EffectModulation::default(), None);
    }

    let (early_energy, stereo_difference) = stereo_energy_and_difference(&mut effect, 8192);
    let (late_energy, _) = stereo_energy_and_difference(&mut effect, 24_000);

    assert!(
        early_energy > 0.01,
        "reverb should produce an audible tail, energy {early_energy}"
    );
    assert!(
        stereo_difference > early_energy * 0.01,
        "reverb tail should be stereo-diffused, energy {early_energy}, difference {stereo_difference}"
    );
    assert!(
        late_energy < early_energy * 2.5,
        "reverb tail should decay rather than build up, early {early_energy}, late {late_energy}"
    );
}

#[test]
fn switching_effects_restores_runtime_params_without_changing_patch_shape() {
    let mut effects = Effects::<64>::new(DEFAULT_SAMPLE_RATE);
    effects.set_type(EffectType::DelayMono);
    effects.set_mix(0.2);
    effects.set_clock_sync(true);
    effects.set_param1(0.3);
    effects.set_param2(0.4);

    effects.set_type(EffectType::Distortion);
    effects.set_mix(0.8);
    effects.set_param1(0.9);
    effects.set_param2(0.7);
    effects.set_type(EffectType::DelayMono);

    let params = effects.params();
    assert_eq!(params.effect_type, EffectType::DelayMono);
    assert_eq!(params.mix, 0.2);
    assert!(params.clock_sync);
    assert_eq!(params.param1, 0.3);
    assert_eq!(params.param2, 0.4);
}

#[test]
fn small_delay_memory_clamps_delay_time_and_remains_bounded() {
    let mut effects = Effects::<8>::new(DEFAULT_SAMPLE_RATE);
    effects.set_enabled(true);
    effects.set_type(EffectType::DdlStereo);
    effects.set_mix(1.0);
    effects.set_param1(1.0);
    effects.set_param2(0.8);

    let mut peak = 0.0f32;
    for frame in 0..128 {
        let input = if frame == 0 { 0.5 } else { 0.0 };
        let (left, right) = effects.next(input, -input, EffectModulation::default(), None);
        peak = peak.max(left.abs()).max(right.abs());
    }

    assert!(peak.is_finite() && peak <= 1.0);
    assert!(peak > 0.01, "clamped delay should still process audio");
}

#[test]
fn undersized_buffered_effects_bypass_without_panicking() {
    let mut effects = Effects::<3>::new(DEFAULT_SAMPLE_RATE);
    effects.set_enabled(true);
    effects.set_mix(1.0);

    for effect_type in [
        EffectType::DelayMono,
        EffectType::Chorus,
        EffectType::Reverb,
    ] {
        effects.set_type(effect_type);
        assert_eq!(
            effects.next(0.25, -0.5, EffectModulation::default(), None),
            (0.25, -0.5)
        );
    }
}
