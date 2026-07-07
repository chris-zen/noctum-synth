use synth_core::{
    ControlMessage, DEFAULT_SAMPLE_RATE, DedicatedModSource, FilterOversampling, ModDestination,
    ModRoute, ModSource, ParamId, Patch, SynthEngine,
};

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

fn rendered_note_rms(mut engine: SynthEngine, note: u8, velocity: f32, frames: usize) -> f32 {
    engine.handle_control(ControlMessage::NoteOn { note, velocity });
    let mut buffer = vec![0.0; frames * 2];
    engine.process(&mut buffer);
    left_rms(&buffer)
}

#[test]
fn default_note_on_renders_oscillator_without_noise() {
    let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
    engine.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });

    let mut buffer = vec![0.0; 16_384 * 2];
    engine.process(&mut buffer);
    let rms = left_rms(&buffer);

    assert!(rms > 0.09, "default osc1 note should be audible, RMS {rms}");
}

#[test]
fn note_off_decays_instead_of_cutting_to_silence() {
    let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.002));
    engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgRelease, 0.05));
    engine.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });

    let mut attack_buffer = vec![0.0; 1024 * 2];
    engine.process(&mut attack_buffer);
    assert!(left_rms(&attack_buffer) > 0.05);

    engine.handle_control(ControlMessage::NoteOff { note: 60 });
    let mut release_start = vec![0.0; 128 * 2];
    engine.process(&mut release_start);
    let release_start_rms = left_rms(&release_start);

    let mut release_tail = vec![0.0; 4096 * 2];
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
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::SetParam(
            ParamId::AmpEgRelease,
            release_seconds,
        ));
        engine.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut attack_buffer = vec![0.0; 4096 * 2];
        engine.process(&mut attack_buffer);
        engine.handle_control(ControlMessage::NoteOff { note: 60 });

        let mut release_buffer = vec![0.0; 2048 * 2];
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
    let mut delayed = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    delayed.handle_control(ControlMessage::SetParam(ParamId::AmpEgDelay, 0.05));
    let delayed_rms = rendered_note_rms(delayed, 60, 1.0, 512);

    let immediate = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    let immediate_rms = rendered_note_rms(immediate, 60, 1.0, 512);

    assert!(
        delayed_rms < immediate_rms * 0.01,
        "amp delay should suppress the initial output window, delayed {delayed_rms}, immediate {immediate_rms}"
    );
}

#[test]
fn amp_env_amount_controls_output_level() {
    let mut full = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    full.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 1.0));
    let full_rms = rendered_note_rms(full, 60, 1.0, 4096);

    let mut reduced = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    reduced.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 0.25));
    let reduced_rms = rendered_note_rms(reduced, 60, 1.0, 4096);

    assert!(
        full_rms > reduced_rms * 3.0,
        "amp env amount should scale output level, full {full_rms}, reduced {reduced_rms}"
    );
}

#[test]
fn amp_velocity_param_controls_velocity_sensitivity() {
    let mut sensitive_low = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    sensitive_low.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 1.0));
    let sensitive_low_rms = rendered_note_rms(sensitive_low, 60, 0.25, 4096);

    let mut sensitive_high = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    sensitive_high.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 1.0));
    let sensitive_high_rms = rendered_note_rms(sensitive_high, 60, 1.0, 4096);

    let mut insensitive_low = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    insensitive_low.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
    let insensitive_low_rms = rendered_note_rms(insensitive_low, 60, 0.25, 4096);

    let mut insensitive_high = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    insensitive_high.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
    let insensitive_high_rms = rendered_note_rms(insensitive_high, 60, 1.0, 4096);

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
fn filter_envelope_params_shape_filter_modulation() {
    fn filtered_attack_rms(filter_attack_seconds: f32) -> f32 {
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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

        let mut buffer = vec![0.0; 2048 * 2];
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
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
        engine.handle_control(ControlMessage::SetParam(ParamId::AmpVelocity, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 80.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 0.0));
        engine.handle_control(ControlMessage::SetParam(ParamId::FilterEnvAmount, 1.0));
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
        "filter velocity should increase filter envelope depth, low {sensitive_low}, high {sensitive_high}"
    );
    assert!(
        (insensitive_high - insensitive_low).abs() < insensitive_high * 0.05,
        "filter velocity 0 should ignore note velocity, low {insensitive_low}, high {insensitive_high}"
    );
}

#[test]
fn filter_velocity_scales_inverted_filter_envelope_depth() {
    fn filtered_velocity_rms(note_velocity: f32) -> f32 {
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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
        low_velocity > high_velocity * 1.2,
        "filter velocity should deepen inverted filter EG modulation, low {low_velocity}, high {high_velocity}"
    );
}

#[test]
fn filter_control_params_remain_wired_and_stable() {
    let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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

    let mut buffer = vec![0.0; 4096 * 2];
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
    let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    for note in [48, 55, 60, 64, 67, 72] {
        engine.handle_control(ControlMessage::NoteOn {
            note,
            velocity: 1.0,
        });
    }

    let mut buffer = vec![0.0; 4096 * 2];
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
    let mut stereo = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    stereo.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });
    let mut stereo_buffer = vec![0.0; 512 * 2];
    stereo.process(&mut stereo_buffer);
    let stereo_left = channel_samples(&stereo_buffer, 2, 0);

    let mut multichannel = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    multichannel.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });
    let mut multichannel_buffer = vec![0.0; 512 * 8];
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
    let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    engine.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 1.0));
    engine.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });
    engine.handle_control(ControlMessage::NoteOn {
        note: 67,
        velocity: 1.0,
    });

    let mut buffer = vec![0.0; 2048 * 4];
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
    let mut single = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    single.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });
    let mut single_buffer = vec![0.0; 4096 * 2];
    single.process(&mut single_buffer);
    let single_rms = left_rms(&single_buffer);

    let mut chord = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    chord.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });
    chord.handle_control(ControlMessage::NoteOn {
        note: 67,
        velocity: 1.0,
    });
    let mut chord_buffer = vec![0.0; 4096 * 2];
    chord.process(&mut chord_buffer);
    let chord_rms = left_rms(&chord_buffer);

    assert!(
        chord_rms > single_rms * 1.05,
        "two voices should add energy, single RMS {single_rms}, chord RMS {chord_rms}"
    );
}

#[test]
fn hard_sync_keeps_osc1_audible_with_osc1_only_mix() {
    let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 1.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::HardSync, 1.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.002));
    engine.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });

    let mut buffer = vec![0.0; 4096 * 2];
    engine.process(&mut buffer);
    let rms = left_rms(&buffer);

    assert!(
        rms > 0.05,
        "hard sync should not mute osc1 with osc1-only mix, RMS {rms}"
    );
}

#[test]
fn enabling_hard_sync_on_active_note_keeps_osc1_audible() {
    let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 1.0));
    engine.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });

    let mut before = vec![0.0; 1024 * 2];
    engine.process(&mut before);

    engine.handle_control(ControlMessage::SetParam(ParamId::HardSync, 1.0));
    let mut after = vec![0.0; 4096 * 2];
    engine.process(&mut after);
    let rms = left_rms(&after);

    assert!(
        rms > 0.05,
        "enabling hard sync on an active note should not mute osc1, RMS {rms}"
    );
}

#[test]
fn hard_sync_with_osc2_off_does_not_mute_or_reset_osc1() {
    let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 0.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::HardSync, 1.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.002));
    engine.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });

    let mut buffer = vec![0.0; 4096 * 2];
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
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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
        let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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
fn old_patch_json_defaults_to_empty_mod_matrix() {
    let mut value = serde_json::to_value(Patch::default()).unwrap();
    value.as_object_mut().unwrap().remove("mod_matrix");

    let patch: Patch = serde_json::from_value(value).unwrap();

    assert!(patch.mod_matrix.free_slots.iter().all(|slot| !slot.enabled));
    assert!(patch.mod_matrix.dedicated.iter().all(|slot| !slot.enabled));
}

#[test]
fn lfo_to_vca_changes_output_level_over_time() {
    let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
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

    let mut first = vec![0.0; 1024 * 2];
    engine.process(&mut first);
    let first_rms = left_rms(&first);

    let mut second = vec![0.0; 1024 * 2];
    engine.process(&mut second);
    let second_rms = left_rms(&second);

    assert!(
        (first_rms - second_rms).abs() > first_rms.min(second_rms) * 0.1,
        "LFO VCA modulation should change level over time, first {first_rms}, second {second_rms}"
    );
}

#[test]
fn filter_oversampling_control_message_can_change_while_rendering() {
    let mut engine = SynthEngine::<{ synth_core::VOICE_PACKS }>::new(DEFAULT_SAMPLE_RATE);
    engine.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::NoiseLevel, 0.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::SubOscLevel, 0.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::FilterCutoff, 2000.0));
    engine.handle_control(ControlMessage::SetParam(ParamId::FilterResonance, 1.0));
    engine.handle_control(ControlMessage::SetFilterOversampling(FilterOversampling::Off));
    engine.handle_control(ControlMessage::NoteOn {
        note: 60,
        velocity: 1.0,
    });

    let mut before = vec![0.0; 1024 * 2];
    engine.process(&mut before);

    engine.handle_control(ControlMessage::SetFilterOversampling(FilterOversampling::X4));
    let mut after = vec![0.0; 1024 * 2];
    engine.process(&mut after);

    let peak = after
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0f32, f32::max);
    assert!(
        peak.is_finite() && peak <= 1.0,
        "dynamic oversampling change should keep output finite and bounded, peak {peak}"
    );
}
