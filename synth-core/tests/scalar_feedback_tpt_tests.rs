use synth_core::{Filter, FilterOversampling, FilterType, f32x4};

const SAMPLE_RATE: f32 = 48_000.0;
const CUTOFF_HZ: f32 = 440.0;

fn configured_filter(
    filter_type: FilterType,
    resonance: f32,
    poles: u8,
    oversampling: FilterOversampling,
) -> Filter {
    let mut filter = Filter::new(filter_type);
    filter.set_cutoff(CUTOFF_HZ);
    filter.set_resonance(resonance);
    filter.set_poles(poles);
    filter.set_oversampling(oversampling);
    filter
}

fn process(filter: &mut Filter, input: f32x4, sample_rate: f32) -> f32x4 {
    filter.process(
        input,
        f32x4::splat(69.0),
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
    resonance: f32,
    poles: u8,
) -> f32 {
    sine_gain_with_oversampling(
        filter_type,
        sample_rate,
        frequency,
        resonance,
        poles,
        FilterOversampling::Off,
    )
}

fn sine_gain_with_oversampling(
    filter_type: FilterType,
    sample_rate: f32,
    frequency: f32,
    resonance: f32,
    poles: u8,
    oversampling: FilterOversampling,
) -> f32 {
    sine_gain_at_level(
        filter_type,
        sample_rate,
        frequency,
        resonance,
        poles,
        oversampling,
        1.0e-4,
    )
}

fn sine_gain_at_level(
    filter_type: FilterType,
    sample_rate: f32,
    frequency: f32,
    resonance: f32,
    poles: u8,
    oversampling: FilterOversampling,
    amplitude: f32,
) -> f32 {
    let mut filter = configured_filter(filter_type, resonance, poles, oversampling);
    let phase_step = core::f32::consts::TAU * frequency / sample_rate;
    let frames = (sample_rate * 0.1) as usize;
    let mut phase = 0.0f32;
    for _ in 0..frames {
        let _ = process(
            &mut filter,
            f32x4::splat(phase.sin() * amplitude),
            sample_rate,
        );
        phase += phase_step;
    }

    let mut sin_sum = 0.0;
    let mut cos_sum = 0.0;
    for _ in 0..frames {
        let sine = phase.sin();
        let output =
            process(&mut filter, f32x4::splat(sine * amplitude), sample_rate).to_array()[0];
        sin_sum += output * sine;
        cos_sum += output * phase.cos();
        phase += phase_step;
    }
    2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / frames as f32 / amplitude
}

fn tail_samples(
    filter_type: FilterType,
    resonance: f32,
    oversampling: FilterOversampling,
) -> Vec<f32> {
    let mut filter = configured_filter(filter_type, resonance, 4, oversampling);
    for _ in 0..128 {
        let _ = process(&mut filter, f32x4::splat(0.1), SAMPLE_RATE);
    }
    let mut samples = Vec::with_capacity(48_000);
    for _ in 0..48_000 {
        samples.push(process(&mut filter, f32x4::splat(0.0), SAMPLE_RATE).to_array()[0]);
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
            crossings += 1;
            first.get_or_insert(index);
            last = Some(index);
        }
    }
    match (first, last) {
        (Some(first), Some(last)) if crossings > 1 && last > first => {
            (crossings - 1) as f32 * SAMPLE_RATE / (last - first) as f32
        }
        _ => 0.0,
    }
}

fn projected_amplitude(samples: &[f32], frequency: f32) -> f32 {
    let phase_step = core::f32::consts::TAU * frequency / SAMPLE_RATE;
    let mut phase = 0.0f32;
    let mut sin_sum = 0.0;
    let mut cos_sum = 0.0;
    for &sample in samples {
        sin_sum += sample * phase.sin();
        cos_sum += sample * phase.cos();
        phase += phase_step;
    }
    2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / samples.len() as f32
}

#[test]
fn scalar_feedback_tpt_is_available_and_has_expected_slopes() {
    assert!(FilterType::ScalarFeedbackTpt.is_implemented());
    for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let two_pole_lower = sine_gain(
            FilterType::ScalarFeedbackTpt,
            sample_rate,
            CUTOFF_HZ * 4.0,
            0.0,
            2,
        );
        let two_pole_upper = sine_gain(
            FilterType::ScalarFeedbackTpt,
            sample_rate,
            CUTOFF_HZ * 8.0,
            0.0,
            2,
        );
        let four_pole_lower = sine_gain(
            FilterType::ScalarFeedbackTpt,
            sample_rate,
            CUTOFF_HZ * 4.0,
            0.0,
            4,
        );
        let four_pole_upper = sine_gain(
            FilterType::ScalarFeedbackTpt,
            sample_rate,
            CUTOFF_HZ * 8.0,
            0.0,
            4,
        );
        let two_pole_db = 20.0 * (two_pole_lower / two_pole_upper).log10();
        let four_pole_db = 20.0 * (four_pole_lower / four_pole_upper).log10();
        assert!(
            (11.0..=12.5).contains(&two_pole_db),
            "sr={sample_rate} slope={two_pole_db}"
        );
        assert!(
            (22.0..=24.5).contains(&four_pole_db),
            "sr={sample_rate} slope={four_pole_db}"
        );
    }
}

#[test]
fn scalar_feedback_tpt_linear_response_matches_baseline() {
    for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for poles in [2, 4] {
            for (frequency, resonance) in [
                (CUTOFF_HZ * 0.5, 0.0),
                (CUTOFF_HZ, 0.65),
                (CUTOFF_HZ * 2.0, 0.0),
            ] {
                let baseline = sine_gain(
                    FilterType::DistributedNewtonTpt,
                    sample_rate,
                    frequency,
                    resonance,
                    poles,
                );
                let candidate = sine_gain(
                    FilterType::ScalarFeedbackTpt,
                    sample_rate,
                    frequency,
                    resonance,
                    poles,
                );
                let relative_error = (candidate - baseline).abs() / baseline.max(1.0e-9);
                assert!(
                    relative_error < 2.0e-4,
                    "sr={sample_rate} poles={poles} frequency={frequency} baseline={baseline} candidate={candidate}"
                );
            }
        }
    }
}

#[test]
fn scalar_feedback_tpt_self_oscillation_is_calibrated_to_baseline() {
    let baseline = tail_samples(
        FilterType::DistributedNewtonTpt,
        1.0,
        FilterOversampling::Off,
    );
    let candidate = tail_samples(FilterType::ScalarFeedbackTpt, 1.0, FilterOversampling::Off);
    let baseline = &baseline[24_000..];
    let candidate = &candidate[24_000..];
    let baseline_pitch = positive_crossing_pitch(baseline);
    let candidate_pitch = positive_crossing_pitch(candidate);
    let baseline_rms = rms(baseline);
    let candidate_rms = rms(candidate);
    let baseline_peak = baseline
        .iter()
        .fold(0.0f32, |peak, value| peak.max(value.abs()));
    let candidate_peak = candidate
        .iter()
        .fold(0.0f32, |peak, value| peak.max(value.abs()));

    assert!(
        (candidate_pitch / baseline_pitch - 1.0).abs() < 0.01,
        "baseline={baseline_pitch} candidate={candidate_pitch}"
    );
    assert!(
        (candidate_rms / baseline_rms - 1.0).abs() < 0.06,
        "baseline={baseline_rms} candidate={candidate_rms}"
    );
    assert!(
        (candidate_peak / baseline_peak - 1.0).abs() < 0.06,
        "baseline={baseline_peak} candidate={candidate_peak}"
    );

    for harmonic in 2..=5 {
        let baseline_harmonic = projected_amplitude(baseline, baseline_pitch * harmonic as f32);
        let candidate_harmonic = projected_amplitude(candidate, candidate_pitch * harmonic as f32);
        assert!(
            candidate_harmonic < 0.005,
            "harmonic={harmonic} baseline={baseline_harmonic} candidate={candidate_harmonic}"
        );
    }
}

#[test]
fn scalar_feedback_tpt_self_oscillation_onset_tracks_baseline() {
    for resonance in [0.85, 0.9, 0.95, 1.0] {
        let baseline = tail_samples(
            FilterType::DistributedNewtonTpt,
            resonance,
            FilterOversampling::Off,
        );
        let candidate = tail_samples(
            FilterType::ScalarFeedbackTpt,
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
                (0.65..=1.25).contains(&ratio),
                "resonance={resonance} baseline={baseline_rms} candidate={candidate_rms}"
            );
        }
    }
}

#[test]
fn scalar_feedback_tpt_resonance_boosts_cutoff_smoothly() {
    let gains = [0.0, 0.5, 0.65, 0.7, 0.71, 0.72, 0.8].map(|resonance| {
        sine_gain_with_oversampling(
            FilterType::ScalarFeedbackTpt,
            SAMPLE_RATE,
            CUTOFF_HZ,
            resonance,
            4,
            FilterOversampling::Auto,
        )
    });
    assert!(
        gains.windows(2).all(|pair| pair[1] > pair[0]),
        "gains={gains:?}"
    );
    assert!(gains[4] > gains[0] * 7.0, "gains={gains:?}");
    let threshold_step_db = 20.0 * (gains[5] / gains[4]).log10();
    assert!(
        threshold_step_db < 0.75,
        "gains={gains:?} step={threshold_step_db}dB"
    );

    let musical = [0.73, 0.74, 0.75, 0.76].map(|resonance| {
        sine_gain_at_level(
            FilterType::ScalarFeedbackTpt,
            SAMPLE_RATE,
            CUTOFF_HZ,
            resonance,
            4,
            FilterOversampling::Auto,
            0.1,
        )
    });
    let musical_step_db = 20.0 * (musical[2] / musical[1]).log10();
    assert!(musical.windows(2).all(|pair| pair[1] > pair[0]));
    assert!(
        musical_step_db < 0.8,
        "musical gains={musical:?} step={musical_step_db}dB"
    );
}

#[test]
fn scalar_feedback_tpt_global_oversampling_does_not_switch_at_threshold() {
    let mut filter = configured_filter(
        FilterType::ScalarFeedbackTpt,
        0.7,
        4,
        FilterOversampling::Auto,
    );
    let phase_step = core::f32::consts::TAU * CUTOFF_HZ / SAMPLE_RATE;
    let mut phase = 0.0f32;
    let mut previous = 0.0f32;
    for _ in 0..24_000 {
        previous = process(&mut filter, f32x4::splat(phase.sin() * 0.1), SAMPLE_RATE).to_array()[0];
        phase += phase_step;
    }
    let mut found_peak = previous.abs() >= 0.12;
    for _ in 0..24_000 {
        if found_peak {
            break;
        }
        previous = process(&mut filter, f32x4::splat(phase.sin() * 0.1), SAMPLE_RATE).to_array()[0];
        phase += phase_step;
        found_peak = previous.abs() >= 0.12;
    }
    assert!(
        found_peak,
        "resonant signal never reached the expected level"
    );

    filter.set_resonance(0.72);
    let crossed = process(&mut filter, f32x4::splat(phase.sin() * 0.1), SAMPLE_RATE).to_array()[0];
    assert!(
        (crossed - previous).abs() < 0.04,
        "threshold crossing dropped or jumped: before={previous} after={crossed}"
    );
}

#[test]
fn scalar_feedback_tpt_auto_self_oscillates_from_silence() {
    let mut filter = configured_filter(
        FilterType::ScalarFeedbackTpt,
        1.0,
        4,
        FilterOversampling::Auto,
    );
    let mut energy = 0.0;
    let mut peak = 0.0f32;
    for frame in 0..96_000 {
        let output = process(&mut filter, f32x4::splat(0.0), SAMPLE_RATE).to_array()[0];
        assert!(output.is_finite());
        if frame >= 72_000 {
            energy += output * output;
            peak = peak.max(output.abs());
        }
    }
    let tail_rms = (energy / 24_000.0).sqrt();
    assert!(tail_rms > 0.4, "rms={tail_rms} peak={peak}");
    assert!(peak > 0.6 && peak < 1.0, "rms={tail_rms} peak={peak}");
}

#[test]
fn scalar_feedback_tpt_oversampling_modes_are_stable() {
    for oversampling in [
        FilterOversampling::Off,
        FilterOversampling::X2,
        FilterOversampling::X4,
    ] {
        let samples = tail_samples(FilterType::ScalarFeedbackTpt, 1.0, oversampling);
        let tail = &samples[24_000..];
        let tail_rms = rms(tail);
        let peak = tail
            .iter()
            .fold(0.0f32, |peak, value| peak.max(value.abs()));
        assert!(tail.iter().all(|value| value.is_finite()));
        assert!(
            (0.1..1.5).contains(&tail_rms),
            "mode={oversampling:?} rms={tail_rms}"
        );
        assert!(peak < 2.0, "mode={oversampling:?} peak={peak}");
    }
}

#[test]
fn scalar_feedback_tpt_two_pole_resonance_decays() {
    let mut filter = configured_filter(
        FilterType::ScalarFeedbackTpt,
        1.0,
        2,
        FilterOversampling::X4,
    );
    for _ in 0..128 {
        let _ = process(&mut filter, f32x4::splat(0.1), SAMPLE_RATE);
    }
    let mut first_energy = 0.0;
    let mut last_energy = 0.0;
    for frame in 0..24_000 {
        let output = process(&mut filter, f32x4::splat(0.0), SAMPLE_RATE).to_array()[0];
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
fn scalar_feedback_tpt_remains_finite_across_control_grid() {
    for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for oversampling in [
            FilterOversampling::Off,
            FilterOversampling::X2,
            FilterOversampling::X4,
        ] {
            for poles in [2, 4] {
                for resonance in [0.0, 0.71, 0.9, 1.0] {
                    let mut filter = configured_filter(
                        FilterType::ScalarFeedbackTpt,
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
fn scalar_feedback_tpt_reset_and_simd_lanes_are_independent() {
    let mut filter = configured_filter(
        FilterType::ScalarFeedbackTpt,
        0.6,
        4,
        FilterOversampling::Off,
    );
    let mut fresh = configured_filter(
        FilterType::ScalarFeedbackTpt,
        0.6,
        4,
        FilterOversampling::Off,
    );
    for _ in 0..128 {
        let _ = process(&mut filter, f32x4::new([0.2, -0.1, 0.4, -0.3]), SAMPLE_RATE);
    }
    filter.reset_lane(2);
    let reset_lane = process(&mut filter, f32x4::splat(0.1), SAMPLE_RATE).to_array();
    let fresh_lane = process(&mut fresh, f32x4::splat(0.1), SAMPLE_RATE).to_array();
    assert_eq!(reset_lane[2], fresh_lane[2]);
    assert_ne!(reset_lane[0], fresh_lane[0]);

    filter.reset();
    fresh.reset();
    let reset = process(&mut filter, f32x4::splat(0.1), SAMPLE_RATE);
    let fresh = process(&mut fresh, f32x4::splat(0.1), SAMPLE_RATE);
    assert_eq!(reset, fresh);
    let lanes = reset.to_array();
    assert!(lanes.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn scalar_feedback_tpt_mixed_lane_oversampling_is_independent() {
    let mut mixed = configured_filter(
        FilterType::ScalarFeedbackTpt,
        0.6,
        4,
        FilterOversampling::X4,
    );
    let mut linear = configured_filter(
        FilterType::ScalarFeedbackTpt,
        0.6,
        4,
        FilterOversampling::X4,
    );
    let mut nonlinear = configured_filter(
        FilterType::ScalarFeedbackTpt,
        0.6,
        4,
        FilterOversampling::X4,
    );

    for frame in 0..512 {
        let input = f32x4::splat((frame as f32 * 0.037).sin() * 0.1);
        let render = |filter: &mut Filter, resonance_mod: f32x4| {
            filter.process(
                input,
                f32x4::splat(69.0),
                f32x4::splat(0.0),
                f32x4::splat(1.0),
                f32x4::splat(0.0),
                f32x4::splat(0.0),
                resonance_mod,
                f32x4::splat(0.0),
                SAMPLE_RATE,
            )
        };
        let mixed_output = render(&mut mixed, f32x4::new([0.0, 0.4, 0.0, 0.4])).to_array();
        let linear_output = render(&mut linear, f32x4::splat(0.0)).to_array();
        let nonlinear_output = render(&mut nonlinear, f32x4::splat(0.4)).to_array();
        assert!(
            (mixed_output[0] - linear_output[0]).abs() < 1.0e-6,
            "linear lane frame={frame} mixed={} reference={}",
            mixed_output[0],
            linear_output[0]
        );
        assert!(
            (mixed_output[2] - linear_output[2]).abs() < 1.0e-6,
            "linear lane frame={frame} mixed={} reference={}",
            mixed_output[2],
            linear_output[2]
        );
        assert_eq!(
            mixed_output[1], nonlinear_output[1],
            "nonlinear lane frame={frame}"
        );
        assert_eq!(
            mixed_output[3], nonlinear_output[3],
            "nonlinear lane frame={frame}"
        );
    }
}
