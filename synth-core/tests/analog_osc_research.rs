#![cfg(feature = "oscillator-research")]

use std::sync::OnceLock;

use synth_core::dsp::{
    FilterType, MONOLOGUE_WAVETABLE_BANK_PROFILE, MipWavetableBank, WAVETABLE_BANK_SAMPLES,
    Waveform, WavetableBank,
};
use synth_core::{
    BankId, OscillatorEngineType, OscillatorResearchModel, ParamId, ResearchComparisonMetrics,
    ResearchError, ResearchEvent, ResearchModelCapabilities, ResearchModelDescriptor,
    ResearchModelFamily, ResearchModelId, ResearchParameterDescriptor, ResearchParameterScale,
    ResearchRegistry, ResearchRenderCase, ResearchSignalMetrics, SynthEngineWithMemory,
    render_research_case,
};

fn static_case(waveform: Waveform) -> ResearchRenderCase {
    ResearchRenderCase {
        waveform,
        sample_rate_hz: 48_000.0,
        frequency_hz: 223.7,
        shape: 0.37,
        warmup_samples: 512,
        render_samples: 8_192,
        seed: 42,
        reset_phase: true,
    }
}

fn zero_wavetable_bank() -> MipWavetableBank {
    static BANK: OnceLock<MipWavetableBank> = OnceLock::new();
    *BANK.get_or_init(|| {
        MipWavetableBank::new(Box::leak(
            vec![0.0; WAVETABLE_BANK_SAMPLES].into_boxed_slice(),
        ))
        .unwrap()
    })
}

fn generated_measured_bank() -> Option<WavetableBank> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/analog-osc/banks/korg-monologue-measured-bank-v1.f32le");
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() % 4 != 0 {
        return None;
    }
    let samples = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    WavetableBank::new(Box::leak(samples), &MONOLOGUE_WAVETABLE_BANK_PROFILE).ok()
}

#[test]
fn wavetable_live_engine_smoke_test() {
    let effects = vec![0.0; 96_000].into_boxed_slice();
    let mut engine =
        SynthEngineWithMemory::<_, 1>::new_with_effects_memory(48_000.0, effects).unwrap();
    engine.set_wavetable_bank(BankId::Monologue);
    engine.set_oscillator_engine(OscillatorEngineType::Wavetable);
    engine.set_filter_type(FilterType::PassThrough);
    engine.set_param(ParamId::Osc1Waveform, Waveform::Triangle.index() as f32);
    engine.note_on(57, 1.0);
    let mut output = vec![0.0; 16_384];
    engine.process(&mut output);
    assert!(output.iter().all(|sample| sample.is_finite()));
    assert!(output.iter().any(|sample| sample.abs() > 1.0e-4));

    for (waveform, shape) in [
        (Waveform::Saw, 0.8),
        (Waveform::SawTri, 0.65),
        (Waveform::Triangle, 0.8),
        (Waveform::Pulse, 0.0),
        (Waveform::Pulse, 0.9),
    ] {
        engine.set_param(ParamId::Osc1Waveform, waveform.index() as f32);
        engine.set_param(ParamId::Osc1ShapeMod, shape);
        let mut shaped = vec![0.0; 2_048];
        engine.process(&mut shaped);
        assert!(
            shaped.iter().all(|sample| sample.is_finite()),
            "{waveform:?} shape {shape} produced a non-finite sample"
        );
        assert!(
            shaped.iter().any(|sample| sample.abs() > 1.0e-4),
            "{waveform:?} shape {shape} was unexpectedly silent"
        );
    }

    engine.set_oscillator_engine(OscillatorEngineType::Blep);
    let mut after_switch = vec![0.0; 1_024];
    engine.process(&mut after_switch);
    assert!(after_switch.iter().all(|sample| sample.is_finite()));
}

#[test]
fn oscillator_selection_changes_preserve_held_voices() {
    let effects = vec![0.0; 96_000].into_boxed_slice();
    let mut engine =
        SynthEngineWithMemory::<_, 1>::new_with_effects_memory(48_000.0, effects).unwrap();
    engine.set_filter_type(FilterType::PassThrough);
    engine.set_param(ParamId::AmpEgAttack, 0.0);
    engine.set_param(ParamId::AmpEgDecay, 0.0);
    engine.set_param(ParamId::AmpEgSustain, 1.0);
    engine.set_param(ParamId::AmpEgRelease, 1.0);
    engine.note_on(57, 1.0);
    let mut warmup = vec![0.0; 2_048];
    engine.process(&mut warmup);
    assert_eq!(engine.active_voice_count(), 1);

    engine.set_oscillator_engine(OscillatorEngineType::Wavetable);
    assert_eq!(engine.active_voice_count(), 1);
    engine.set_wavetable_bank(BankId::Monologue);
    assert_eq!(engine.active_voice_count(), 1);
    engine.set_oscillator_engine(OscillatorEngineType::Blep);
    assert_eq!(engine.active_voice_count(), 1);
    engine.set_blep_method(synth_core::dsp::SawMethod::PolyBlep);
    assert_eq!(engine.active_voice_count(), 1);

    let mut after_switch = vec![0.0; 2_048];
    engine.process(&mut after_switch);
    assert!(after_switch.iter().all(|sample| sample.is_finite()));
    assert!(after_switch.iter().any(|sample| sample.abs() > 1.0e-4));
}

#[test]
fn measured_banks_survive_initial_patch_apply() {
    fn render(engine_type: OscillatorEngineType) -> Vec<f32> {
        let effects = vec![0.0; 96_000].into_boxed_slice();
        let mut engine =
            SynthEngineWithMemory::<_, 1>::new_with_effects_memory(48_000.0, effects).unwrap();
        engine.set_wavetable_bank(BankId::Monologue);
        engine.apply_patch(&synth_core::Patch::default());
        engine.set_oscillator_engine(engine_type);
        engine.set_filter_type(FilterType::PassThrough);
        engine.set_param(ParamId::Osc1Waveform, Waveform::Saw.index() as f32);
        engine.set_param(ParamId::Osc1Enabled, 1.0);
        engine.set_param(ParamId::Osc2Enabled, 0.0);
        engine.note_on(57, 1.0);
        let mut output = vec![0.0; 8_192];
        engine.process(&mut output);
        output
    }

    let measured = render(OscillatorEngineType::Wavetable);
    let baseline = render(OscillatorEngineType::Blep);
    let maximum_difference = measured
        .iter()
        .zip(&baseline)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        maximum_difference > 1.0e-3,
        "measured output collapsed to baseline after patch apply: {maximum_difference}"
    );
}

#[test]
fn wavetable_live_engine_slop_changes_the_audible_output() {
    fn render(slop: f32) -> Vec<f32> {
        let effects = vec![0.0; 96_000].into_boxed_slice();
        let mut engine =
            SynthEngineWithMemory::<_, 1>::new_with_effects_memory(48_000.0, effects).unwrap();
        engine.set_wavetable_bank(BankId::Monologue);
        engine.set_oscillator_engine(OscillatorEngineType::Wavetable);
        engine.set_filter_type(FilterType::PassThrough);
        engine.set_param(ParamId::Osc1Waveform, Waveform::Saw.index() as f32);
        engine.set_param(ParamId::Osc1Enabled, 1.0);
        engine.set_param(ParamId::Osc2Enabled, 0.0);
        engine.set_param(ParamId::OscSlop, slop);
        engine.note_on(57, 1.0);
        let mut output = vec![0.0; 16_384];
        engine.process(&mut output);
        output
    }

    let stable = render(0.0);
    let sloppy = render(1.0);
    assert!(stable.iter().all(|sample| sample.is_finite()));
    assert!(sloppy.iter().all(|sample| sample.is_finite()));
    let maximum_difference = stable
        .iter()
        .zip(&sloppy)
        .map(|(stable, sloppy)| (stable - sloppy).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        maximum_difference > 1.0e-4,
        "slop did not change the live measured output: {maximum_difference}"
    );
}

#[test]
fn wavetable_live_engine_hard_sync_changes_the_audible_output() {
    fn render(hard_sync: bool) -> Vec<f32> {
        let effects = vec![0.0; 96_000].into_boxed_slice();
        let mut engine =
            SynthEngineWithMemory::<_, 1>::new_with_effects_memory(48_000.0, effects).unwrap();
        engine.set_wavetable_bank(BankId::Monologue);
        engine.set_oscillator_engine(OscillatorEngineType::Wavetable);
        engine.set_filter_type(FilterType::PassThrough);
        engine.set_param(ParamId::Osc1Waveform, Waveform::Saw.index() as f32);
        engine.set_param(ParamId::Osc2Waveform, Waveform::Saw.index() as f32);
        engine.set_param(ParamId::Osc1Frequency, 36.0);
        engine.set_param(ParamId::Osc2Frequency, 60.0);
        engine.set_param(ParamId::Osc1Enabled, 1.0);
        engine.set_param(ParamId::Osc2Enabled, 1.0);
        engine.set_param(ParamId::OscMix, 0.0);
        engine.set_param(ParamId::HardSync, f32::from(hard_sync));
        engine.note_on(57, 1.0);
        let mut output = vec![0.0; 16_384];
        engine.process(&mut output);
        output
    }

    let free = render(false);
    let synced = render(true);
    assert!(free.iter().all(|sample| sample.is_finite()));
    assert!(synced.iter().all(|sample| sample.is_finite()));
    let maximum_difference = free
        .iter()
        .zip(&synced)
        .map(|(free, synced)| (free - synced).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        maximum_difference > 1.0e-4,
        "hard sync did not change the live measured output: {maximum_difference}"
    );
}

#[test]
fn registry_order_and_descriptors_are_stable() {
    let descriptors: Vec<_> = ResearchRegistry::descriptors().collect();
    assert_eq!(descriptors.len(), ResearchModelId::ALL.len());
    assert_eq!(descriptors[0].id, "baseline-v1");
    assert_eq!(descriptors[1].id, "table-blep-v1");
    assert_eq!(descriptors[2].id, "polyblep-v1");
    assert_eq!(descriptors[3].id, "wavetable-prototype-v1");
    assert_eq!(descriptors[4].id, "korg-monologue-measured-wavetable-v1");
    assert_eq!(descriptors[5].id, "prophet5-wavetable-v1");
    assert_eq!(descriptors[6].id, "target-conditioned-phase-filter-v1");
    assert_eq!(descriptors[7].id, "target-conditioned-phase-filter-v2");
    assert!(
        descriptors[..7]
            .iter()
            .all(|descriptor| descriptor.revision == 1)
    );
    assert_eq!(descriptors[7].revision, 2);
    assert!(!descriptors[0].requires_external_asset);
    assert!(descriptors[3].requires_external_asset);
    assert!(!descriptors[3].capabilities.real_time_safe);
    assert!(descriptors[4].capabilities.real_time_safe);
    assert!(descriptors[4].capabilities.saw_triangle);
    assert!(descriptors[4].capabilities.shape);
    assert!(descriptors[4].capabilities.audio_rate_pwm);
    assert!(descriptors[4].capabilities.hard_sync);
    assert!(descriptors[4].capabilities.slop);
    assert!(descriptors[4].requires_external_asset);
    assert!(descriptors[5].capabilities.real_time_safe);
    assert!(descriptors[5].requires_external_asset);
    assert!(!descriptors[6].requires_external_asset);
    assert!(!descriptors[7].requires_external_asset);
    let profile = ResearchRegistry::target_profile_metadata(ResearchModelId::TargetConditioned)
        .expect("fitted model has profile provenance");
    assert_eq!(profile.0, "korg-monologue-phase-filter-v1");
    assert_eq!(profile.1, "korg-monologue-v1");
    assert_eq!(profile.2.len(), 64);
    let v2_profile =
        ResearchRegistry::target_profile_metadata(ResearchModelId::TargetConditionedV2)
            .expect("v2 fitted model has profile provenance");
    assert_eq!(v2_profile.0, "korg-monologue-phase-filter-v2");
    assert_eq!(v2_profile.1, "korg-monologue-v1");
    assert_eq!(v2_profile.2.len(), 64);
    let measured = ResearchRegistry::target_profile_metadata(ResearchModelId::WavetableMonologue)
        .expect("measured bank has target provenance");
    assert_eq!(measured.0, "korg-monologue-measured-bank-v1");
    assert_eq!(measured.1, "korg-monologue-v1");
    assert_eq!(measured.2.len(), 64);
    let arturia = ResearchRegistry::target_profile_metadata(ResearchModelId::WavetableProphet5)
        .expect("prophet5 wavetable bank has target provenance");
    assert_eq!(arturia.0, "prophet5-wavetable-bank-v1");
    assert_eq!(arturia.1, "prophet5-v1");
    assert_eq!(arturia.2.len(), 64);
}

#[test]
fn target_conditioned_v2_renders_all_fitted_waveforms_deterministically() {
    for waveform in [Waveform::Saw, Waveform::Triangle, Waveform::Pulse] {
        let case = static_case(waveform);
        let mut model = ResearchRegistry::create(
            ResearchModelId::TargetConditionedV2,
            case.sample_rate_hz,
            None,
        )
        .unwrap();
        let mut first = vec![0.0; case.render_samples];
        let mut second = vec![0.0; case.render_samples];
        let first_summary = render_research_case(&mut model, case, &mut first).unwrap();
        let second_summary = render_research_case(&mut model, case, &mut second).unwrap();
        assert_eq!(
            first_summary.sample_hash_fnv1a64,
            second_summary.sample_hash_fnv1a64
        );
        assert!(first_summary.signal.rms > 0.01);
        assert!(first.iter().all(|sample| sample.is_finite()));
    }
}

#[test]
fn target_conditioned_v2_zero_character_matches_production_phase_zero() {
    for waveform in [Waveform::Saw, Waveform::Triangle, Waveform::Pulse] {
        let mut case = static_case(waveform);
        case.shape = 0.0;
        case.warmup_samples = 0;

        let mut baseline =
            ResearchRegistry::create(ResearchModelId::Baseline, case.sample_rate_hz, None).unwrap();
        let mut candidate = ResearchRegistry::create(
            ResearchModelId::TargetConditionedV2,
            case.sample_rate_hz,
            None,
        )
        .unwrap();
        candidate.set_parameter("phase-amount", 0.0).unwrap();
        candidate.set_parameter("filter-amount", 0.0).unwrap();

        let mut baseline_samples = vec![0.0; case.render_samples];
        let mut candidate_samples = vec![0.0; case.render_samples];
        render_research_case(&mut baseline, case, &mut baseline_samples).unwrap();
        render_research_case(&mut candidate, case, &mut candidate_samples).unwrap();

        let maximum_error = baseline_samples
            .iter()
            .zip(&candidate_samples)
            .map(|(expected, actual)| (expected - actual).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            maximum_error <= 2.0e-6,
            "{waveform:?} zero-character source differs from production by {maximum_error}"
        );
    }
}

#[test]
fn target_conditioned_model_renders_all_fitted_waveforms_deterministically() {
    for waveform in [Waveform::Saw, Waveform::Triangle, Waveform::Pulse] {
        let case = static_case(waveform);
        let mut model = ResearchRegistry::create(
            ResearchModelId::TargetConditioned,
            case.sample_rate_hz,
            None,
        )
        .unwrap();
        let mut first = vec![0.0; case.render_samples];
        let mut second = vec![0.0; case.render_samples];
        let first_summary = render_research_case(&mut model, case, &mut first).unwrap();
        let second_summary = render_research_case(&mut model, case, &mut second).unwrap();
        assert_eq!(
            first_summary.sample_hash_fnv1a64,
            second_summary.sample_hash_fnv1a64
        );
        assert!(first_summary.signal.rms > 0.01);
        assert!(first.iter().all(|sample| sample.is_finite()));
    }
}

#[test]
fn target_conditioned_ablation_parameters_survive_case_rebuilds() {
    let case = static_case(Waveform::Saw);
    let mut model =
        ResearchRegistry::create(ResearchModelId::TargetConditioned, 48_000.0, None).unwrap();
    assert_eq!(model.parameter_descriptors().len(), 2);
    model.set_parameter("phase-amount", 0.0).unwrap();
    model.set_parameter("filter-amount", 0.0).unwrap();
    let mut ablated = vec![0.0; case.render_samples];
    render_research_case(&mut model, case, &mut ablated).unwrap();
    assert_eq!(model.parameter_value("phase-amount"), Some(0.0));
    assert_eq!(model.parameter_value("filter-amount"), Some(0.0));

    let mut fitted =
        ResearchRegistry::create(ResearchModelId::TargetConditioned, 48_000.0, None).unwrap();
    let mut fitted_output = vec![0.0; case.render_samples];
    render_research_case(&mut fitted, case, &mut fitted_output).unwrap();
    assert_ne!(
        ablated
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        fitted_output
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        model.set_parameter("filter-amount", 1.1),
        Err(ResearchError::InvalidParameterValue)
    );
}

#[test]
fn target_conditioned_model_rejects_unfitted_saw_triangle_morph() {
    let mut model =
        ResearchRegistry::create(ResearchModelId::TargetConditioned, 48_000.0, None).unwrap();
    assert_eq!(
        model.configure(static_case(Waveform::SawTri)),
        Err(ResearchError::UnsupportedEvent)
    );
}

#[test]
fn target_conditioned_triangle_remains_continuous_during_dense_pitch_sweep() {
    let mut case = static_case(Waveform::Triangle);
    case.frequency_hz = 20.7;
    case.warmup_samples = 0;
    let mut model =
        ResearchRegistry::create(ResearchModelId::TargetConditioned, 48_000.0, None).unwrap();
    model.configure(case).unwrap();
    for _ in 0..8_192 {
        let _ = model.next_sample();
    }

    let sample_count = 96_000;
    let frequency_ratio = 1_170.0_f32 / case.frequency_hz;
    let mut previous = model.next_sample();
    let mut maximum_step = 0.0_f32;
    for index in 0..sample_count {
        let amount = index as f32 / (sample_count - 1) as f32;
        let frequency = case.frequency_hz * frequency_ratio.powf(amount);
        model
            .apply_event(ResearchEvent::SetFrequency(frequency))
            .unwrap();
        let sample = model.next_sample();
        assert!(sample.is_finite());
        maximum_step = maximum_step.max((sample - previous).abs());
        previous = sample;
    }
    assert!(
        maximum_step < 0.20,
        "unexpected triangle sweep discontinuity: {maximum_step}"
    );
}

#[test]
fn wavetable_triangle_remains_continuous_during_dense_pitch_sweep() {
    let Some(bank) = generated_measured_bank() else {
        eprintln!("generated measured bank is absent; skipping pitch-continuity test");
        return;
    };
    let mut case = static_case(Waveform::Triangle);
    case.frequency_hz = 20.7;
    case.shape = 0.4;
    case.warmup_samples = 0;
    let mut model = ResearchRegistry::create_wavetable(48_000.0, bank).unwrap();
    model.configure(case).unwrap();
    for _ in 0..8_192 {
        let _ = model.next_sample();
    }

    let sample_count = 96_000;
    let frequency_ratio = 1_200.0_f32 / case.frequency_hz;
    let mut previous = model.next_sample();
    let mut maximum_step = 0.0_f32;
    for index in 0..sample_count {
        let amount = index as f32 / (sample_count - 1) as f32;
        let frequency = case.frequency_hz * frequency_ratio.powf(amount);
        model
            .apply_event(ResearchEvent::SetFrequency(frequency))
            .unwrap();
        let sample = model.next_sample();
        assert!(sample.is_finite());
        maximum_step = maximum_step.max((sample - previous).abs());
        previous = sample;
    }
    assert!(
        maximum_step < 0.20,
        "unexpected measured-table pitch discontinuity: {maximum_step}"
    );
}

#[test]
fn repeated_builtin_render_is_bit_identical() {
    let case = static_case(Waveform::Pulse);
    let mut first_model = ResearchRegistry::create(ResearchModelId::Baseline, 48_000.0, None)
        .expect("create first baseline");
    let mut second_model = ResearchRegistry::create(ResearchModelId::Baseline, 48_000.0, None)
        .expect("create second baseline");
    let mut first = vec![0.0; case.render_samples];
    let mut second = vec![0.0; case.render_samples];
    let first_summary = render_research_case(&mut first_model, case, &mut first).unwrap();
    let second_summary = render_research_case(&mut second_model, case, &mut second).unwrap();
    assert_eq!(
        first_summary.sample_hash_fnv1a64,
        second_summary.sample_hash_fnv1a64
    );
    assert_eq!(
        first
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        second
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    );
    let comparison = ResearchComparisonMetrics::measure(&first, &second).unwrap();
    assert_eq!(comparison.normalized_rms_error, 0.0);
    assert_eq!(comparison.maximum_absolute_error, 0.0);
    assert!((comparison.correlation - 1.0).abs() < 1.0e-12);

    let mut repeated = vec![0.0; case.render_samples];
    let repeated_summary = render_research_case(&mut first_model, case, &mut repeated).unwrap();
    assert_eq!(
        first_summary.sample_hash_fnv1a64,
        repeated_summary.sample_hash_fnv1a64
    );
}

#[test]
fn built_in_hard_sync_event_uses_a_real_lane_mask() {
    let mut case = static_case(Waveform::Saw);
    case.warmup_samples = 0;
    let mut fresh = ResearchRegistry::create(ResearchModelId::TableBlep, 48_000.0, None).unwrap();
    fresh.configure(case).unwrap();
    let expected = fresh.next_sample();

    let mut advanced =
        ResearchRegistry::create(ResearchModelId::TableBlep, 48_000.0, None).unwrap();
    advanced.configure(case).unwrap();
    for _ in 0..137 {
        let _ = advanced.next_sample();
    }
    advanced
        .apply_event(ResearchEvent::HardSync {
            subsample_offset: 1.0,
        })
        .unwrap();
    assert_eq!(advanced.next_sample().to_bits(), expected.to_bits());
}

#[test]
fn wavetable_hard_sync_preserves_subsample_timing() {
    let Some(bank) = generated_measured_bank() else {
        eprintln!("generated measured bank is absent; skipping hard-sync timing test");
        return;
    };
    let mut case = static_case(Waveform::Saw);
    case.frequency_hz = 440.0;
    case.shape = 0.35;
    case.warmup_samples = 0;

    let mut at_start = ResearchRegistry::create_wavetable(48_000.0, bank).unwrap();
    let mut at_end = ResearchRegistry::create_wavetable(48_000.0, bank).unwrap();
    let mut repeated = ResearchRegistry::create_wavetable(48_000.0, bank).unwrap();
    for model in [&mut at_start, &mut at_end, &mut repeated] {
        model.configure(case).unwrap();
        for _ in 0..137 {
            let _ = model.next_sample();
        }
    }
    at_start
        .apply_event(ResearchEvent::HardSync {
            subsample_offset: 0.0,
        })
        .unwrap();
    at_end
        .apply_event(ResearchEvent::HardSync {
            subsample_offset: 1.0,
        })
        .unwrap();
    repeated
        .apply_event(ResearchEvent::HardSync {
            subsample_offset: 0.0,
        })
        .unwrap();

    let start_samples: Vec<_> = (0..16).map(|_| at_start.next_sample()).collect();
    let end_samples: Vec<_> = (0..16).map(|_| at_end.next_sample()).collect();
    let repeated_samples: Vec<_> = (0..16).map(|_| repeated.next_sample()).collect();
    assert!(start_samples.iter().all(|sample| sample.is_finite()));
    assert!(end_samples.iter().all(|sample| sample.is_finite()));
    assert_eq!(
        start_samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        repeated_samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    );
    assert!(
        start_samples
            .iter()
            .zip(&end_samples)
            .any(|(start, end)| (start - end).abs() > 1.0e-5),
        "fractional sync offsets collapsed to the same reset phase"
    );
}

#[test]
fn wavetable_hard_sync_is_bounded_for_every_shape_path() {
    let Some(bank) = generated_measured_bank() else {
        eprintln!("generated measured bank is absent; skipping hard-sync waveform test");
        return;
    };

    for (waveform, shape) in [
        (Waveform::Saw, 0.8),
        (Waveform::SawTri, 0.65),
        (Waveform::Triangle, 0.8),
        (Waveform::Pulse, 0.0),
        (Waveform::Pulse, 0.9),
    ] {
        let mut case = static_case(waveform);
        case.frequency_hz = 440.0;
        case.shape = shape;
        case.warmup_samples = 0;
        let mut model = ResearchRegistry::create_wavetable(48_000.0, bank).unwrap();
        model.configure(case).unwrap();

        for offset in [0.0, 0.25, 0.5, 0.75, 1.0] {
            for _ in 0..37 {
                let sample = model.next_sample();
                assert!(sample.is_finite());
            }
            model
                .apply_event(ResearchEvent::HardSync {
                    subsample_offset: offset,
                })
                .unwrap();
            for _ in 0..16 {
                let sample = model.next_sample();
                assert!(
                    sample.is_finite() && sample.abs() < 4.0,
                    "{waveform:?} shape {shape} offset {offset} produced {sample}"
                );
            }
        }
    }
}

#[test]
fn baseline_and_wavetable_use_the_same_case_runner() {
    let case = static_case(Waveform::Saw);
    let mut baseline = ResearchRegistry::create(ResearchModelId::Baseline, 48_000.0, None).unwrap();
    let mut wavetable = ResearchRegistry::create(
        ResearchModelId::Wavetable,
        48_000.0,
        Some(zero_wavetable_bank()),
    )
    .unwrap();
    let mut baseline_output = vec![0.0; case.render_samples];
    let mut wavetable_output = vec![0.0; case.render_samples];
    let baseline_summary = render_research_case(&mut baseline, case, &mut baseline_output).unwrap();
    let wavetable_summary =
        render_research_case(&mut wavetable, case, &mut wavetable_output).unwrap();
    assert_eq!(baseline_summary.case, wavetable_summary.case);
    assert_eq!(wavetable_summary.descriptor.id, "wavetable-prototype-v1");
    assert!(baseline_summary.signal.rms > 0.1);
    assert_eq!(wavetable_summary.signal.rms, 0.0);
}

struct StatefulProbe {
    phase: f32,
    increment: f32,
    shape: f32,
    shape_bias: f32,
}

const STATEFUL_PARAMETERS: [ResearchParameterDescriptor; 1] = [ResearchParameterDescriptor {
    id: "shape-bias",
    name: "Shape Bias",
    unit: "normalized",
    minimum: 0.0,
    maximum: 1.0,
    default: 0.5,
    scale: ResearchParameterScale::Linear,
}];

impl Default for StatefulProbe {
    fn default() -> Self {
        Self {
            phase: 0.0,
            increment: 0.0,
            shape: 0.5,
            shape_bias: 0.5,
        }
    }
}

impl OscillatorResearchModel for StatefulProbe {
    fn descriptor(&self) -> ResearchModelDescriptor {
        ResearchModelDescriptor {
            id: "stateful-probe-v1",
            name: "Stateful Test Probe",
            revision: 1,
            family: ResearchModelFamily::Stateful,
            capabilities: ResearchModelCapabilities {
                saw: true,
                saw_triangle: false,
                triangle: false,
                pulse: false,
                shape: true,
                audio_rate_pwm: false,
                hard_sync: true,
                note_reset: true,
                slop: false,
                simd_lanes: false,
                real_time_safe: false,
            },
            requires_external_asset: false,
            mutable_state_bytes: std::mem::size_of::<Self>(),
            immutable_asset_bytes: 0,
            latency_samples: 0,
            bounded_render_cost: true,
            no_std_compatible: true,
        }
    }

    fn configure(&mut self, case: ResearchRenderCase) -> Result<(), ResearchError> {
        let case = case.validate()?;
        if case.waveform != Waveform::Saw {
            return Err(ResearchError::UnsupportedEvent);
        }
        self.increment = case.frequency_hz / case.sample_rate_hz;
        self.shape = case.shape;
        if case.reset_phase {
            self.phase = 0.0;
        }
        Ok(())
    }

    fn reset(&mut self, reset_phase: bool) {
        if reset_phase {
            self.phase = 0.0;
        }
    }

    fn apply_event(&mut self, event: ResearchEvent) -> Result<(), ResearchError> {
        match event {
            ResearchEvent::Reset { reset_phase } => self.reset(reset_phase),
            ResearchEvent::SetShape(shape) if (0.0..=1.0).contains(&shape) => self.shape = shape,
            ResearchEvent::HardSync { subsample_offset } => {
                self.phase = self.increment * (1.0 - subsample_offset.clamp(0.0, 1.0));
            }
            _ => return Err(ResearchError::UnsupportedEvent),
        }
        Ok(())
    }

    fn next_sample(&mut self) -> f32 {
        let output = self.phase * 2.0 - 1.0 + self.shape * 0.0 + (self.shape_bias - 0.5) * 0.01;
        self.phase = (self.phase + self.increment).fract();
        output
    }

    fn parameter_descriptors(&self) -> &'static [ResearchParameterDescriptor] {
        &STATEFUL_PARAMETERS
    }

    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), ResearchError> {
        if id != STATEFUL_PARAMETERS[0].id {
            return Err(ResearchError::UnknownParameter);
        }
        if !value.is_finite()
            || value < STATEFUL_PARAMETERS[0].minimum
            || value > STATEFUL_PARAMETERS[0].maximum
        {
            return Err(ResearchError::InvalidParameterValue);
        }
        self.shape_bias = value;
        Ok(())
    }

    fn parameter_value(&self, id: &str) -> Option<f32> {
        (id == STATEFUL_PARAMETERS[0].id).then_some(self.shape_bias)
    }
}

#[test]
fn fully_stateful_model_runs_without_an_analog_oscillator() {
    let case = static_case(Waveform::Saw);
    let mut model = StatefulProbe::default();
    model.set_parameter("shape-bias", 0.72).unwrap();
    assert_eq!(model.parameter_value("shape-bias"), Some(0.72));
    assert_eq!(
        model.set_parameter("missing", 0.0),
        Err(ResearchError::UnknownParameter)
    );
    let mut first = vec![0.0; case.render_samples];
    let mut second = vec![0.0; case.render_samples];
    let summary = render_research_case(&mut model, case, &mut first).unwrap();
    assert_eq!(summary.descriptor.family, ResearchModelFamily::Stateful);
    model
        .apply_event(ResearchEvent::HardSync {
            subsample_offset: 0.25,
        })
        .unwrap();
    model.reset(true);
    let second_summary = render_research_case(&mut model, case, &mut second).unwrap();
    assert_eq!(
        summary.sample_hash_fnv1a64,
        second_summary.sample_hash_fnv1a64
    );
}

#[test]
fn analytic_signal_metrics_recover_level_frequency_and_comparison() {
    let sample_rate = 48_000.0_f32;
    let frequency = 375.0_f32;
    let samples: Vec<_> = (0..48_000)
        .map(|index| (std::f32::consts::TAU * frequency * index as f32 / sample_rate).sin())
        .collect();
    let metrics = ResearchSignalMetrics::measure(&samples, sample_rate).unwrap();
    assert!(metrics.dc.abs() < 1.0e-6);
    assert!((metrics.rms - std::f64::consts::FRAC_1_SQRT_2).abs() < 1.0e-6);
    assert!((metrics.measured_frequency_hz.unwrap() - f64::from(frequency)).abs() < 1.0e-3);

    let delayed: Vec<_> = std::iter::once(0.0)
        .chain(samples.iter().copied().take(samples.len() - 1))
        .collect();
    let comparison = ResearchComparisonMetrics::measure(&samples, &delayed).unwrap();
    assert!(comparison.normalized_rms_error > 0.04);
    assert!(comparison.correlation < 1.0);
}

#[test]
fn invalid_cases_and_assets_fail_before_rendering() {
    let mut invalid = static_case(Waveform::Saw);
    invalid.frequency_hz = invalid.sample_rate_hz;
    assert_eq!(invalid.validate(), Err(ResearchError::InvalidFrequency));
    assert!(matches!(
        ResearchRegistry::create(ResearchModelId::Wavetable, 48_000.0, None),
        Err(ResearchError::MissingMipWavetableBank)
    ));
    assert!(matches!(
        ResearchRegistry::create(ResearchModelId::WavetableMonologue, 48_000.0, None),
        Err(ResearchError::MissingWavetableBank)
    ));
    assert!(matches!(
        ResearchRegistry::create(
            ResearchModelId::Baseline,
            48_000.0,
            Some(zero_wavetable_bank())
        ),
        Err(ResearchError::UnexpectedMipWavetableBank)
    ));
}
