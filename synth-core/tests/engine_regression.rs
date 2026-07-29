#![cfg(all(
    feature = "wide-4",
    feature = "filter-all",
    not(feature = "fast-math"),
    not(feature = "oscillator-polyblep")
))]

use synth_core::{
    ArpMode, ControlMessage, EffectType, GlideMode, LayerPatch, ModDestination, ModRoute,
    ModSource, ParamId, SynthEngine, UnisonMode,
};

#[test]
fn representative_one_layer_render_is_bit_identical() {
    let mut engine = SynthEngine::<4, 4096>::new(48_000.0);
    for (param, value) in [
        (ParamId::Osc1Waveform, 2.0),
        (ParamId::Osc1ShapeMod, 0.37),
        (ParamId::Osc2FineTune, 7.25),
        (ParamId::OscMix, 0.42),
        (ParamId::FilterCutoff, 3_200.0),
        (ParamId::FilterResonance, 0.31),
        (ParamId::AmpEgAttack, 0.004),
        (ParamId::AmpEgDecay, 0.18),
        (ParamId::AmpEgSustain, 0.73),
        (ParamId::Lfo1Rate, 4.7),
        (ParamId::Lfo1Depth, 0.24),
        (
            ParamId::Lfo1Destination,
            ModDestination::OscAllFrequency.index() as f32,
        ),
        (ParamId::GlideEnabled, 1.0),
        (ParamId::GlideMode, GlideMode::FixedRate.index() as f32),
        (ParamId::UnisonMode, UnisonMode::V4.index() as f32),
        (ParamId::UnisonDetune, 5.5),
        (ParamId::UnisonEnabled, 1.0),
        (ParamId::EffectEnabled, 1.0),
        (ParamId::EffectType, EffectType::DelayMono.index() as f32),
        (ParamId::EffectMix, 0.36),
        (ParamId::EffectParam1, 0.08),
        (ParamId::EffectParam2, 0.41),
        (ParamId::MasterVolume, 0.77),
    ] {
        engine.handle_control(ControlMessage::SetParam(param, value));
    }
    engine.handle_control(ControlMessage::SetModulation {
        route: ModRoute::Free(0),
        enabled: true,
        source: ModSource::Velocity,
        destination: ModDestination::FilterCutoff,
        amount: 0.27,
    });
    engine.note_on(48, 0.82);

    let mut rendered = vec![0.0; 2048 * 2];
    engine.process(&mut rendered);
    engine.note_on(55, 0.63);
    engine.sustain_pedal(true);
    engine.note_off(48);
    let start = rendered.len();
    rendered.resize(start + 2048 * 2, 0.0);
    engine.process(&mut rendered[start..]);

    let mut patch = LayerPatch::default();
    patch.osc1.fine_tune = -11.0;
    patch.osc2.fine_tune = 9.0;
    patch.filter.cutoff = 5_400.0;
    patch.filter.resonance = 0.48;
    patch.unison_enabled = true;
    patch.unison_mode = UnisonMode::V4;
    patch.unison_detune = 3.25;
    patch.glide_enabled = true;
    patch.glide_mode = GlideMode::FixedTime;
    patch.effects.enabled = true;
    patch.effects.effect_type = EffectType::Chorus;
    patch.effects.mix = 0.29;
    patch.effects.param1 = 0.53;
    patch.effects.param2 = 0.22;
    patch.master_volume = 0.68;
    engine.apply_patch(&patch);
    engine.note_off(55);
    engine.sustain_pedal(false);
    engine.note_on(67, 0.91);
    let start = rendered.len();
    rendered.resize(start + 3072 * 2, 0.0);
    engine.process(&mut rendered[start..]);

    engine.all_notes_off();
    engine.handle_control(ControlMessage::SetParam(ParamId::UnisonEnabled, 0.0));
    engine.handle_control(ControlMessage::SetParam(
        ParamId::ArpMode,
        ArpMode::UpDown.index() as f32,
    ));
    engine.handle_control(ControlMessage::SetParam(ParamId::ArpEnabled, 1.0));
    engine.note_on(60, 0.72);
    engine.note_on(64, 0.76);
    engine.note_on(67, 0.8);
    let start = rendered.len();
    rendered.resize(start + 12_000 * 2, 0.0);
    engine.process(&mut rendered[start..]);

    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for sample in rendered {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    assert_eq!(hash, 0xca6f_48f9_3f56_3a2f);
}
