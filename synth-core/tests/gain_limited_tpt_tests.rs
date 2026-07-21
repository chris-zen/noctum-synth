use synth_core::{f32x4, Filter, FilterOversampling, FilterType, ParamId, SynthEngine};

const SAMPLE_RATE: f32 = 48_000.0;
const CUTOFF_HZ: f32 = 440.0;

fn configured_filter(
    filter_type: FilterType,
    cutoff_hz: f32,
    resonance: f32,
    poles: u8,
    oversampling: FilterOversampling,
) -> Filter {
    let mut filter = Filter::new(filter_type);
    filter.set_cutoff(cutoff_hz);
    filter.set_resonance(resonance);
    filter.set_poles(poles);
    filter.set_oversampling(oversampling);
    filter
}

fn process(filter: &mut Filter, input: f32x4, note: f32x4, sample_rate: f32) -> f32x4 {
    filter.process(
        input,
        note,
        f32x4::splat(0.0),
        f32x4::splat(1.0),
        f32x4::splat(0.0),
        f32x4::splat(0.0),
        f32x4::splat(0.0),
        f32x4::splat(0.0),
        sample_rate,
    )
}

fn sine_gain(
    filter_type: FilterType,
    sample_rate: f32,
    frequency: f32,
    cutoff_hz: f32,
    resonance: f32,
    poles: u8,
    oversampling: FilterOversampling,
    amplitude: f32,
) -> f32 {
    let mut filter = configured_filter(filter_type, cutoff_hz, resonance, poles, oversampling);
    let phase_step = core::f32::consts::TAU * frequency / sample_rate;
    let frames = (sample_rate * 0.1) as usize;
    let mut phase = 0.0f32;
    for _ in 0..frames {
        let _ = process(
            &mut filter,
            f32x4::splat(phase.sin() * amplitude),
            f32x4::splat(69.0),
            sample_rate,
        );
        phase += phase_step;
    }

    let mut sin_sum = 0.0;
    let mut cos_sum = 0.0;
    for _ in 0..frames {
        let sine = phase.sin();
        let output = process(
            &mut filter,
            f32x4::splat(sine * amplitude),
            f32x4::splat(69.0),
            sample_rate,
        )
        .to_array()[0];
        sin_sum += output * sine;
        cos_sum += output * phase.cos();
        phase += phase_step;
    }
    2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / frames as f32 / amplitude
}

fn self_oscillation_tail(
    filter_type: FilterType,
    cutoff_hz: f32,
    resonance: f32,
    oversampling: FilterOversampling,
) -> Vec<f32> {
    let mut filter = configured_filter(filter_type, cutoff_hz, resonance, 4, oversampling);
    for _ in 0..128 {
        let _ = process(
            &mut filter,
            f32x4::splat(0.1),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        );
    }
    let mut samples = Vec::with_capacity(48_000);
    for _ in 0..48_000 {
        samples.push(
            process(
                &mut filter,
                f32x4::splat(0.0),
                f32x4::splat(69.0),
                SAMPLE_RATE,
            )
            .to_array()[0],
        );
    }
    samples
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

fn positive_crossing_pitch(samples: &[f32]) -> f32 {
    let mut crossings = 0usize;
    let mut first = None;
    let mut last = None;
    for (index, pair) in samples.windows(2).enumerate() {
        if pair[0] <= 0.0 && pair[1] > 0.0 {
            let crossing = index as f32 + (-pair[0] / (pair[1] - pair[0])).clamp(0.0, 1.0);
            crossings += 1;
            first.get_or_insert(crossing);
            last = Some(crossing);
        }
    }
    match (first, last) {
        (Some(first), Some(last)) if crossings > 1 && last > first => {
            (crossings - 1) as f32 * SAMPLE_RATE / (last - first) as f32
        }
        _ => 0.0,
    }
}

fn analyzer_peak_near(samples: &[f32], center_bin: usize, radius: usize) -> (usize, f32) {
    let fft_size = samples.len();
    let mut peak_bin = center_bin;
    let mut peak = 0.0f32;
    for bin in
        center_bin.saturating_sub(radius).max(1)..=(center_bin + radius).min(fft_size / 2 - 1)
    {
        let step = core::f32::consts::TAU * bin as f32 / fft_size as f32;
        let mut sin_sum = 0.0;
        let mut cos_sum = 0.0;
        for (index, sample) in samples.iter().copied().enumerate() {
            let window =
                0.5 * (1.0 - (core::f32::consts::TAU * index as f32 / (fft_size - 1) as f32).cos());
            let phase = step * index as f32;
            sin_sum += sample * window * phase.sin();
            cos_sum += sample * window * phase.cos();
        }
        let magnitude = (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / fft_size as f32;
        if magnitude > peak {
            peak = magnitude;
            peak_bin = bin;
        }
    }
    (peak_bin, 20.0 * peak.max(1.0e-10).log10())
}

fn live_self_oscillation_analyzer_harmonics_db(
    cutoff_hz: f32,
    resonance: f32,
    velocity: f32,
    amp_velocity: f32,
    oscillator_active: bool,
    note: u8,
) -> [f32; 15] {
    const FFT_SIZE: usize = 4096;
    let mut engine = SynthEngine::<1, 48_000>::new(SAMPLE_RATE);
    engine.set_filter_type(FilterType::GainLimitedTpt);
    engine.set_filter_oversampling(FilterOversampling::Off);
    // Mirror the UI's self-oscillation setup. For the silent case oscillator 1
    // stays enabled but the mixer points at disabled oscillator 2.
    engine.set_param(ParamId::Osc1Enabled, 1.0);
    engine.set_param(ParamId::Osc2Enabled, 0.0);
    engine.set_param(ParamId::OscMix, if oscillator_active { 0.0 } else { 1.0 });
    engine.set_param(ParamId::Osc1Waveform, 2.0);
    engine.set_param(ParamId::NoiseLevel, 0.0);
    engine.set_param(ParamId::SubOscLevel, 0.0);
    engine.set_param(ParamId::FilterCutoff, cutoff_hz);
    engine.set_param(ParamId::FilterResonance, resonance);
    engine.set_param(ParamId::FilterPoles, 1.0);
    engine.set_param(ParamId::AmpEgAttack, 0.0005);
    engine.set_param(ParamId::AmpEgDecay, 0.0005);
    engine.set_param(ParamId::AmpEgSustain, 1.0);
    engine.set_param(ParamId::AmpVelocity, amp_velocity);
    engine.set_param(ParamId::MasterVolume, 1.0);
    engine.note_on(note, velocity);

    let mut settle = vec![0.0; 48_000 * 2];
    engine.process_interleaved(&mut settle, 2);
    let mut interleaved = vec![0.0; FFT_SIZE * 2];
    engine.process_interleaved(&mut interleaved, 2);
    let samples: Vec<f32> = interleaved.chunks_exact(2).map(|frame| frame[0]).collect();

    let expected_bin = (cutoff_hz * FFT_SIZE as f32 / SAMPLE_RATE).round() as usize;
    let (fundamental_bin, fundamental_db) = analyzer_peak_near(&samples, expected_bin, 4);
    core::array::from_fn(|index| {
        let harmonic = index + 1;
        if harmonic == 1 {
            fundamental_db
        } else {
            analyzer_peak_near(&samples, fundamental_bin * harmonic, 2).1
        }
    })
}

fn live_key_tracked_fundamental_db(notes: &[u8], measured_hz: f32) -> f32 {
    const FFT_SIZE: usize = 4096;
    let mut engine = SynthEngine::<1, 48_000>::new(SAMPLE_RATE);
    engine.set_filter_type(FilterType::GainLimitedTpt);
    engine.set_filter_oversampling(FilterOversampling::Off);
    engine.set_param(ParamId::Osc1Enabled, 0.0);
    engine.set_param(ParamId::Osc2Enabled, 0.0);
    engine.set_param(ParamId::NoiseLevel, 0.0);
    engine.set_param(ParamId::SubOscLevel, 0.0);
    engine.set_param(ParamId::FilterCutoff, measured_hz);
    engine.set_param(ParamId::FilterResonance, 1.0);
    engine.set_param(ParamId::FilterPoles, 1.0);
    engine.set_param(ParamId::FilterKeyTrack, 1.0);
    engine.set_param(ParamId::AmpEgAttack, 0.0005);
    engine.set_param(ParamId::AmpEgDecay, 0.0005);
    engine.set_param(ParamId::AmpEgSustain, 1.0);
    engine.set_param(ParamId::AmpVelocity, 0.0);
    engine.set_param(ParamId::MasterVolume, 1.0);
    for &note in notes {
        engine.note_on(note, 1.0);
    }

    let mut settle = vec![0.0; 48_000 * 2];
    engine.process_interleaved(&mut settle, 2);
    let mut interleaved = vec![0.0; FFT_SIZE * 2];
    engine.process_interleaved(&mut interleaved, 2);
    let samples: Vec<f32> = interleaved.chunks_exact(2).map(|frame| frame[0]).collect();
    let expected_bin = (measured_hz * FFT_SIZE as f32 / SAMPLE_RATE).round() as usize;
    analyzer_peak_near(&samples, expected_bin, 4).1
}

fn live_driven_chord_stats(notes: &[u8], resonance: f32) -> (f32, f32, usize) {
    let mut engine = SynthEngine::<1, 48_000>::new(SAMPLE_RATE);
    engine.set_filter_type(FilterType::GainLimitedTpt);
    engine.set_filter_oversampling(FilterOversampling::Off);
    engine.set_param(ParamId::Osc1Enabled, 1.0);
    engine.set_param(ParamId::Osc2Enabled, 0.0);
    engine.set_param(ParamId::NoiseLevel, 0.0);
    engine.set_param(ParamId::SubOscLevel, 0.0);
    engine.set_param(ParamId::FilterCutoff, 440.0);
    engine.set_param(ParamId::FilterResonance, resonance);
    engine.set_param(ParamId::AmpVelocity, 0.0);
    engine.set_param(ParamId::AmpEgAttack, 0.0005);
    engine.set_param(ParamId::AmpEgDecay, 0.0005);
    engine.set_param(ParamId::AmpEgSustain, 1.0);
    engine.set_param(ParamId::MasterVolume, 1.0);
    for &note in notes {
        engine.note_on(note, 1.0);
    }
    let mut settle = vec![0.0; 48_000 * 2];
    engine.process_interleaved(&mut settle, 2);
    let mut output = vec![0.0; 4096 * 2];
    engine.process_interleaved(&mut output, 2);
    let left = output.chunks_exact(2).map(|frame| frame[0]);
    let mut peak = 0.0f32;
    let mut energy = 0.0f32;
    let mut clipped = 0usize;
    for sample in left {
        peak = peak.max(sample.abs());
        energy += sample * sample;
        clipped += usize::from(sample.abs() >= 0.999_999);
    }
    (peak, (energy / 4096.0).sqrt(), clipped)
}

#[test]
fn gain_limited_driven_chords_retain_output_headroom() {
    let mut previous_peak = 0.0;
    let mut previous_rms = 0.0;
    for notes in [&[60][..], &[60, 64][..], &[60, 64, 67, 72][..]] {
        let (peak, rms, clipped) = live_driven_chord_stats(notes, 1.0);
        assert_eq!(
            clipped,
            0,
            "{}-note chord reached the final clamp",
            notes.len()
        );
        assert!(
            peak > previous_peak,
            "chord peak should grow with voice count"
        );
        assert!(
            rms > previous_rms,
            "chord intensity should grow with voice count"
        );
        assert!(
            peak < 0.98,
            "{}-note peak left too little headroom: {peak}",
            notes.len()
        );
        assert!(
            rms < 0.45,
            "{}-note RMS is unexpectedly high: {rms}",
            notes.len()
        );
        previous_peak = peak;
        previous_rms = rms;
    }
}

#[test]
fn gain_limited_driven_level_rises_through_max_resonance() {
    let mut previous_rms = 0.0;
    for resonance in [0.90, 0.92, 0.94, 0.95, 0.96, 0.97, 0.98, 0.99, 1.0] {
        let (_, rms, clipped) = live_driven_chord_stats(&[60], resonance);
        assert_eq!(clipped, 0);
        assert!(
            rms >= previous_rms * 0.98,
            "driven level fell at resonance {resonance:.2}: {previous_rms} -> {rms}",
        );
        previous_rms = rms;
    }
}

#[test]
fn adding_a_second_key_does_not_raise_the_first_fundamental() {
    let one_key = live_key_tracked_fundamental_db(&[36], CUTOFF_HZ);
    let two_keys = live_key_tracked_fundamental_db(&[36, 43], CUTOFF_HZ);
    assert!(
        (one_key - two_keys).abs() <= 0.2,
        "first fundamental changed with voice count: one={one_key:.3}dB two={two_keys:.3}dB"
    );
}

#[test]
fn gain_limited_live_analyzer_level_and_harmonics_match_target() {
    for cutoff in [410.0, 110.0, 220.0, 440.0, 739.99, 880.0, 1760.0] {
        let harmonics =
            live_self_oscillation_analyzer_harmonics_db(cutoff, 1.0, 1.0, 1.0, false, 36);
        assert!(
            (-26.0..=-23.0).contains(&harmonics[0]),
            "cutoff={cutoff} harmonics={harmonics:?}"
        );
        for (index, expected) in [
            (1, -69.0..=-63.0),
            (2, -86.0..=-78.0),
            (3, -130.0..=-115.0),
            (4, -141.0..=-123.0),
            (5, -155.0..=-129.0),
        ] {
            assert!(
                expected.contains(&harmonics[index]),
                "cutoff={cutoff} harmonic={} harmonics={harmonics:?}",
                index + 1
            );
        }
        assert!(
            harmonics[0] > harmonics[1]
                && harmonics[1] > harmonics[2]
                && harmonics[2] > harmonics[3]
                && harmonics[3] > harmonics[4]
                && harmonics[4] > harmonics[5],
            "cutoff={cutoff} harmonics={harmonics:?}"
        );
        if cutoff == 410.0 {
            assert!(
                (-66.9..=-64.9).contains(&harmonics[1])
                    && (-82.5..=-80.5).contains(&harmonics[2])
                    && harmonics[3] < -120.0
                    && harmonics[4] < -128.0
                    && harmonics[5] < -138.0,
                "410 Hz Prophet-reference profile missed: {harmonics:?}"
            );
        }
    }

    let soft_sensitive =
        live_self_oscillation_analyzer_harmonics_db(440.0, 1.0, 0.25, 1.0, false, 36)[0];
    let soft_independent =
        live_self_oscillation_analyzer_harmonics_db(440.0, 1.0, 0.25, 0.0, false, 36)[0];
    assert!(soft_sensitive < -34.0, "level={soft_sensitive}");
    assert!(
        (-25.0..=-23.0).contains(&soft_independent),
        "level={soft_independent}"
    );
}

#[test]
fn gain_limited_self_oscillation_is_consistent_with_an_oscillator_active() {
    let autonomous = live_self_oscillation_analyzer_harmonics_db(739.99, 1.0, 1.0, 1.0, false, 36);
    let driven = live_self_oscillation_analyzer_harmonics_db(739.99, 1.0, 1.0, 1.0, true, 36);
    assert!(
        (autonomous[0] - driven[0]).abs() <= 2.0,
        "autonomous={autonomous:?} driven={driven:?}"
    );
    assert!(
        driven[1] < -68.0 && driven[2] < -85.0 && driven[3] < -110.0 && driven[4] < -125.0,
        "oscillator-driven output regained post-filter harmonics: {driven:?}"
    );
}

#[test]
fn gain_limited_does_not_add_post_cutoff_harmonics_at_color_threshold() {
    let below = live_self_oscillation_analyzer_harmonics_db(440.0, 0.93, 1.0, 1.0, true, 69);
    let above = live_self_oscillation_analyzer_harmonics_db(440.0, 0.95, 1.0, 1.0, true, 69);
    for harmonic in 1..5 {
        assert!(
            above[harmonic] <= below[harmonic] + 3.0,
            "harmonic={} below={below:?} above={above:?}",
            harmonic + 1
        );
    }
}

#[test]
fn gain_limited_tpt_is_available_and_has_expected_slopes() {
    assert!(FilterType::GainLimitedTpt.is_implemented());
    for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for (poles, expected) in [(2, 11.0..=12.5), (4, 22.0..=24.5)] {
            let lower = sine_gain(
                FilterType::GainLimitedTpt,
                sample_rate,
                CUTOFF_HZ * 4.0,
                CUTOFF_HZ,
                0.0,
                poles,
                FilterOversampling::Off,
                1.0e-4,
            );
            let upper = sine_gain(
                FilterType::GainLimitedTpt,
                sample_rate,
                CUTOFF_HZ * 8.0,
                CUTOFF_HZ,
                0.0,
                poles,
                FilterOversampling::Off,
                1.0e-4,
            );
            let slope = 20.0 * (lower / upper).log10();
            assert!(
                expected.contains(&slope),
                "sr={sample_rate} poles={poles} slope={slope}"
            );
        }
    }
}

#[test]
fn gain_limited_tpt_linear_response_matches_baseline() {
    const FOUR_POLE_INPUT_GAIN: f32 = 0.40;
    const FOUR_POLE_FEEDBACK: f32 = 3.75;
    const BASELINE_BASS_COMP: f32 = 1.22;
    const CALIBRATED_BASS_COMP: f32 = 0.80;

    for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for poles in [2, 4] {
            for (frequency, resonance) in [
                (CUTOFF_HZ * 0.5, 0.0),
                (CUTOFF_HZ, 0.65),
                (CUTOFF_HZ * 2.0, 0.0),
            ] {
                let gain = |filter_type| {
                    sine_gain(
                        filter_type,
                        sample_rate,
                        frequency,
                        CUTOFF_HZ,
                        resonance,
                        poles,
                        FilterOversampling::Off,
                        1.0e-4,
                    )
                };
                let baseline = gain(FilterType::DistributedNewtonTpt);
                let candidate = gain(FilterType::GainLimitedTpt);
                let expected = if poles == 2 {
                    baseline
                } else {
                    let shaped_resonance = resonance.powf(1.75);
                    let baseline_compensation =
                        1.0 + shaped_resonance * FOUR_POLE_FEEDBACK * BASELINE_BASS_COMP;
                    let calibrated_compensation =
                        1.0 + shaped_resonance * FOUR_POLE_FEEDBACK * CALIBRATED_BASS_COMP;
                    baseline * FOUR_POLE_INPUT_GAIN * calibrated_compensation
                        / baseline_compensation
                };
                let relative_error = (candidate - expected).abs() / expected.max(1.0e-9);
                assert!(
                    relative_error < 2.0e-4,
                    "sr={sample_rate} poles={poles} frequency={frequency} baseline={baseline} expected={expected} candidate={candidate}"
                );
            }
        }
    }
}

#[test]
fn gain_limited_tpt_level_matches_baseline_and_pitch_tracks_five_cutoffs() {
    // Retain the baseline level calibration, but tune pitch to the musical
    // cutoff grid instead of inheriting the baseline model's sharp limit cycle.
    for cutoff_hz in [110.0, 220.0, 440.0, 880.0, 1760.0] {
        let baseline = self_oscillation_tail(
            FilterType::DistributedNewtonTpt,
            cutoff_hz,
            1.0,
            FilterOversampling::Off,
        );
        let candidate = self_oscillation_tail(
            FilterType::GainLimitedTpt,
            cutoff_hz,
            1.0,
            FilterOversampling::Off,
        );
        let baseline = &baseline[24_000..];
        let candidate = &candidate[24_000..];
        let baseline_rms = rms(baseline);
        let candidate_rms = rms(candidate);
        let candidate_pitch = positive_crossing_pitch(candidate);
        let pitch_error_cents = 1200.0 * (candidate_pitch / cutoff_hz).log2();

        assert!(
            (candidate_rms / baseline_rms - 1.0).abs() < 0.08,
            "cutoff={cutoff_hz} baseline_rms={baseline_rms} candidate_rms={candidate_rms}"
        );
        assert!(
            pitch_error_cents.abs() < 20.0,
            "cutoff={cutoff_hz} candidate_pitch={candidate_pitch} error={pitch_error_cents} cents"
        );
    }
}

#[test]
fn gain_limited_tpt_resonance_progression_is_smooth_at_musical_level() {
    let gains = [0.70, 0.71, 0.72, 0.74, 0.75, 0.76, 0.80].map(|resonance| {
        sine_gain(
            FilterType::GainLimitedTpt,
            SAMPLE_RATE,
            CUTOFF_HZ,
            CUTOFF_HZ,
            resonance,
            4,
            FilterOversampling::Auto,
            0.1,
        )
    });
    assert!(
        gains.windows(2).all(|pair| pair[1] > pair[0]),
        "gains={gains:?}"
    );
    let threshold_step_db = 20.0 * (gains[2] / gains[1]).log10();
    let reported_step_db = 20.0 * (gains[4] / gains[3]).log10();
    assert!(
        threshold_step_db < 0.75,
        "gains={gains:?} step={threshold_step_db}dB"
    );
    assert!(
        reported_step_db < 0.8,
        "gains={gains:?} step={reported_step_db}dB"
    );
}

#[test]
fn gain_limited_tpt_global_oversampling_does_not_switch_at_threshold() {
    let mut filter = configured_filter(
        FilterType::GainLimitedTpt,
        CUTOFF_HZ,
        0.70,
        4,
        FilterOversampling::Auto,
    );
    let step = core::f32::consts::TAU * CUTOFF_HZ / SAMPLE_RATE;
    let mut phase = 0.0f32;
    let mut previous = 0.0;
    for _ in 0..24_000 {
        previous = process(
            &mut filter,
            f32x4::splat(phase.sin() * 0.1),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        )
        .to_array()[0];
        phase += step;
    }
    filter.set_resonance(0.72);
    let crossed = process(
        &mut filter,
        f32x4::splat(phase.sin() * 0.1),
        f32x4::splat(69.0),
        SAMPLE_RATE,
    )
    .to_array()[0];
    assert!(
        (crossed - previous).abs() < 0.04,
        "threshold crossing jumped: before={previous} after={crossed}"
    );
}

#[test]
fn gain_limited_tpt_self_oscillation_onset_and_modes_are_stable() {
    for resonance in [0.85, 0.90, 0.95, 1.0] {
        let baseline = self_oscillation_tail(
            FilterType::DistributedNewtonTpt,
            CUTOFF_HZ,
            resonance,
            FilterOversampling::Off,
        );
        let candidate = self_oscillation_tail(
            FilterType::GainLimitedTpt,
            CUTOFF_HZ,
            resonance,
            FilterOversampling::Off,
        );
        let baseline_rms = rms(&baseline[36_000..]);
        let candidate_rms = rms(&candidate[36_000..]);
        if resonance == 0.85 {
            assert!(baseline_rms < 1.0e-3 && candidate_rms < 1.0e-3);
        } else {
            let ratio = candidate_rms / baseline_rms.max(1.0e-9);
            assert!(
                (0.75..=1.2).contains(&ratio),
                "resonance={resonance} baseline={baseline_rms} candidate={candidate_rms}"
            );
        }
    }

    for oversampling in [
        FilterOversampling::Off,
        FilterOversampling::Auto,
        FilterOversampling::X2,
        FilterOversampling::X4,
    ] {
        let samples =
            self_oscillation_tail(FilterType::GainLimitedTpt, CUTOFF_HZ, 1.0, oversampling);
        let tail = &samples[24_000..];
        let tail_rms = rms(tail);
        let peak = tail
            .iter()
            .fold(0.0f32, |peak, value| peak.max(value.abs()));
        assert!(tail.iter().all(|value| value.is_finite()));
        assert!(
            (0.4..0.6).contains(&tail_rms),
            "mode={oversampling:?} rms={tail_rms}"
        );
        assert!(peak < 1.0, "mode={oversampling:?} peak={peak}");
    }
}

#[test]
fn gain_limited_tpt_two_pole_resonance_decays() {
    let mut filter = configured_filter(
        FilterType::GainLimitedTpt,
        CUTOFF_HZ,
        1.0,
        2,
        FilterOversampling::X4,
    );
    for _ in 0..128 {
        let _ = process(
            &mut filter,
            f32x4::splat(0.1),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        );
    }
    let mut first_energy = 0.0;
    let mut last_energy = 0.0;
    for frame in 0..24_000 {
        let output = process(
            &mut filter,
            f32x4::splat(0.0),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        )
        .to_array()[0];
        if frame < 2_000 {
            first_energy += output * output;
        } else if frame >= 22_000 {
            last_energy += output * output;
        }
    }
    assert!(
        last_energy < first_energy * 1.0e-4,
        "first={first_energy} last={last_energy}"
    );
}

#[test]
fn gain_limited_tpt_remains_finite_across_control_grid() {
    for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for oversampling in [
            FilterOversampling::Off,
            FilterOversampling::X2,
            FilterOversampling::X4,
        ] {
            for poles in [2, 4] {
                for resonance in [0.0, 0.71, 0.9, 1.0] {
                    let mut filter = configured_filter(
                        FilterType::GainLimitedTpt,
                        CUTOFF_HZ,
                        resonance,
                        poles,
                        oversampling,
                    );
                    filter.set_key_track(1.0);
                    filter.set_env_amount(1.0);
                    filter.set_env_velocity_amount(1.0);
                    filter.set_audio_mod(1.0);
                    for frame in 0..256 {
                        let phase = frame as f32;
                        let output = filter.process(
                            f32x4::new([0.8, -0.8, 0.25, -0.25]),
                            f32x4::new([24.0, 60.0, 96.0, 120.0]),
                            f32x4::new([0.0, 0.33, 0.66, 1.0]),
                            f32x4::new([0.0, 0.33, 0.66, 1.0]),
                            f32x4::new([phase.sin(), phase.cos(), -phase.sin(), -phase.cos()]),
                            f32x4::new([-48.0, -12.0, 12.0, 48.0]),
                            f32x4::new([-0.2, -0.05, 0.05, 0.2]),
                            f32x4::new([-0.25, 0.0, 0.25, 0.5]),
                            sample_rate,
                        );
                        for value in output.to_array() {
                            assert!(
                                value.is_finite() && value.abs() < 10.0,
                                "sr={sample_rate} mode={oversampling:?} poles={poles} resonance={resonance} frame={frame} output={value}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn gain_limited_tpt_reset_and_lanes_are_independent() {
    let make = || {
        configured_filter(
            FilterType::GainLimitedTpt,
            CUTOFF_HZ,
            0.95,
            4,
            FilterOversampling::X4,
        )
    };
    let mut mixed = make();
    let mut lane_zero = make();
    let mut lane_one = make();
    for frame in 0..512 {
        let input = (frame as f32 * 0.037).sin() * 0.1;
        let mixed_output = mixed
            .process(
                f32x4::splat(input),
                f32x4::splat(69.0),
                f32x4::splat(0.0),
                f32x4::splat(1.0),
                f32x4::splat(0.0),
                f32x4::splat(0.0),
                f32x4::new([-0.25, 0.05, -0.25, 0.05]),
                f32x4::splat(0.0),
                SAMPLE_RATE,
            )
            .to_array();
        let zero = lane_zero
            .process(
                f32x4::splat(input),
                f32x4::splat(69.0),
                f32x4::splat(0.0),
                f32x4::splat(1.0),
                f32x4::splat(0.0),
                f32x4::splat(0.0),
                f32x4::splat(-0.25),
                f32x4::splat(0.0),
                SAMPLE_RATE,
            )
            .to_array()[0];
        let one = lane_one
            .process(
                f32x4::splat(input),
                f32x4::splat(69.0),
                f32x4::splat(0.0),
                f32x4::splat(1.0),
                f32x4::splat(0.0),
                f32x4::splat(0.0),
                f32x4::splat(0.05),
                f32x4::splat(0.0),
                SAMPLE_RATE,
            )
            .to_array()[1];
        assert!((mixed_output[0] - zero).abs() < 1.0e-12);
        assert!((mixed_output[1] - one).abs() < 1.0e-12);
    }

    mixed.reset_lane(2);
    let mut fresh = make();
    let reset_lane = process(
        &mut mixed,
        f32x4::splat(0.1),
        f32x4::splat(69.0),
        SAMPLE_RATE,
    )
    .to_array();
    let fresh_lane = process(
        &mut fresh,
        f32x4::splat(0.1),
        f32x4::splat(69.0),
        SAMPLE_RATE,
    )
    .to_array();
    assert_eq!(reset_lane[2], fresh_lane[2]);
    mixed.reset();
    fresh.reset();
    assert_eq!(
        process(
            &mut mixed,
            f32x4::splat(0.1),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        ),
        process(
            &mut fresh,
            f32x4::splat(0.1),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        )
    );
}

#[test]
fn gain_limited_tpt_key_tracking_and_long_run_are_stable() {
    for note in [36.0, 48.0, 60.0, 72.0, 84.0] {
        let mut filter = configured_filter(
            FilterType::GainLimitedTpt,
            110.0,
            1.0,
            4,
            FilterOversampling::Off,
        );
        filter.set_key_track(1.0);
        let mut samples = Vec::with_capacity(24_000);
        for frame in 0..48_000 {
            let output = process(
                &mut filter,
                f32x4::splat(0.0),
                f32x4::splat(note),
                SAMPLE_RATE,
            )
            .to_array()[0];
            assert!(output.is_finite() && output.abs() < 1.0);
            if frame >= 24_000 {
                samples.push(output);
            }
        }
        let expected = 110.0 * 2.0f32.powf((note - 36.0) / 12.0);
        let pitch = positive_crossing_pitch(&samples);
        assert!(
            (pitch / expected - 1.0).abs() < 0.05,
            "note={note} expected={expected} pitch={pitch}"
        );
    }

    let mut filter = configured_filter(
        FilterType::GainLimitedTpt,
        CUTOFF_HZ,
        1.0,
        4,
        FilterOversampling::Off,
    );
    let mut tail_energy = 0.0;
    for frame in 0..192_000 {
        let input = if frame < 24_000 { 0.1 } else { 0.0 };
        let output = process(
            &mut filter,
            f32x4::splat(input),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        )
        .to_array()[0];
        assert!(output.is_finite() && output.abs() < 1.0);
        if frame >= 168_000 {
            tail_energy += output * output;
        }
    }
    assert!((tail_energy / 24_000.0).sqrt() > 0.4);
}
