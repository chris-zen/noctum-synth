use synth_core::{Filter, FilterOversampling, FilterType, f32x4};

const SAMPLE_RATE: f32 = 48_000.0;
const CUTOFF_HZ: f32 = 440.0;

fn filter(
    filter_type: FilterType,
    cutoff: f32,
    resonance: f32,
    poles: u8,
    oversampling: FilterOversampling,
) -> Filter {
    let mut filter = Filter::new(filter_type);
    filter.set_cutoff(cutoff);
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
    cutoff: f32,
    resonance: f32,
    poles: u8,
    oversampling: FilterOversampling,
    amplitude: f32,
) -> f32 {
    let mut filter = filter(filter_type, cutoff, resonance, poles, oversampling);
    let step = core::f32::consts::TAU * frequency / sample_rate;
    let frames = (sample_rate * 0.1) as usize;
    let mut phase = 0.0f32;
    for _ in 0..frames {
        let _ = process(
            &mut filter,
            f32x4::splat(phase.sin() * amplitude),
            f32x4::splat(69.0),
            sample_rate,
        );
        phase += step;
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
        phase += step;
    }
    2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / frames as f32 / amplitude
}

fn tail(
    filter_type: FilterType,
    cutoff: f32,
    resonance: f32,
    oversampling: FilterOversampling,
    kick: bool,
) -> Vec<f32> {
    let mut filter = filter(filter_type, cutoff, resonance, 4, oversampling);
    if kick {
        for _ in 0..128 {
            let _ = process(
                &mut filter,
                f32x4::splat(0.1),
                f32x4::splat(69.0),
                SAMPLE_RATE,
            );
        }
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

fn pitch(samples: &[f32]) -> f32 {
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
        (Some(first), Some(last)) if crossings > 1 => {
            (crossings - 1) as f32 * SAMPLE_RATE / (last - first) as f32
        }
        _ => 0.0,
    }
}

fn projected_amplitude(samples: &[f32], frequency: f32) -> f32 {
    let step = core::f32::consts::TAU * frequency / SAMPLE_RATE;
    let mut phase = 0.0f32;
    let mut sin_sum = 0.0;
    let mut cos_sum = 0.0;
    for &sample in samples {
        sin_sum += sample * phase.sin();
        cos_sum += sample * phase.cos();
        phase += step;
    }
    2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / samples.len() as f32
}

#[test]
fn cascaded_tpt_svf_is_available_with_butterworth_response_and_slopes() {
    assert!(FilterType::CascadedTptSvf.is_implemented());
    for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for (poles, expected) in [(2, 11.5..=12.6), (4, 22.5..=25.0)] {
            let lower = sine_gain(
                FilterType::CascadedTptSvf,
                sample_rate,
                CUTOFF_HZ * 4.0,
                CUTOFF_HZ,
                0.0,
                poles,
                FilterOversampling::Off,
                1.0e-4,
            );
            let upper = sine_gain(
                FilterType::CascadedTptSvf,
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

        let low = sine_gain(
            FilterType::CascadedTptSvf,
            sample_rate,
            CUTOFF_HZ * 0.5,
            CUTOFF_HZ,
            0.0,
            4,
            FilterOversampling::Off,
            1.0e-4,
        );
        let cutoff = sine_gain(
            FilterType::CascadedTptSvf,
            sample_rate,
            CUTOFF_HZ,
            CUTOFF_HZ,
            0.0,
            4,
            FilterOversampling::Off,
            1.0e-4,
        );
        let high = sine_gain(
            FilterType::CascadedTptSvf,
            sample_rate,
            CUTOFF_HZ * 2.0,
            CUTOFF_HZ,
            0.0,
            4,
            FilterOversampling::Off,
            1.0e-4,
        );
        let two_pole_cutoff = sine_gain(
            FilterType::CascadedTptSvf,
            sample_rate,
            CUTOFF_HZ,
            CUTOFF_HZ,
            0.0,
            2,
            FilterOversampling::Off,
            1.0e-4,
        );
        assert!((0.98..1.02).contains(&low), "sr={sample_rate} low={low}");
        assert!(
            (0.69..0.72).contains(&cutoff),
            "sr={sample_rate} cutoff={cutoff}"
        );
        assert!(
            (0.69..0.72).contains(&two_pole_cutoff),
            "sr={sample_rate} two_pole_cutoff={two_pole_cutoff}"
        );
        assert!(
            (0.058..0.066).contains(&high),
            "sr={sample_rate} high={high}"
        );
    }
}

#[test]
fn cascaded_tpt_svf_self_oscillation_is_tuned_and_harmonically_bounded() {
    for cutoff in [110.0, 220.0, 440.0, 880.0, 1760.0] {
        let baseline = tail(
            FilterType::DistributedNewtonTpt,
            cutoff,
            1.0,
            FilterOversampling::Off,
            true,
        );
        let candidate = tail(
            FilterType::CascadedTptSvf,
            cutoff,
            1.0,
            FilterOversampling::Off,
            true,
        );
        let baseline = &baseline[24_000..];
        let candidate = &candidate[24_000..];
        let baseline_rms = rms(baseline);
        let candidate_rms = rms(candidate);
        let baseline_pitch = pitch(baseline);
        let candidate_pitch = pitch(candidate);
        assert!(
            (candidate_rms / baseline_rms - 1.0).abs() < 0.08,
            "cutoff={cutoff} baseline={baseline_rms} candidate={candidate_rms}"
        );
        assert!(
            (candidate_pitch / baseline_pitch - 1.0).abs() < 0.02,
            "cutoff={cutoff} baseline={baseline_pitch} candidate={candidate_pitch}"
        );
        for harmonic in 2..=5 {
            let amplitude = projected_amplitude(candidate, candidate_pitch * harmonic as f32);
            assert!(
                amplitude < 0.005,
                "cutoff={cutoff} harmonic={harmonic} amplitude={amplitude}"
            );
        }
    }
}

#[test]
fn cascaded_tpt_svf_resonance_onset_and_global_oversampling_are_smooth() {
    let gains = [0.70, 0.71, 0.72, 0.74, 0.75, 0.76, 0.80].map(|resonance| {
        sine_gain(
            FilterType::CascadedTptSvf,
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
    let threshold_db = 20.0 * (gains[2] / gains[1]).log10();
    let reported_db = 20.0 * (gains[4] / gains[3]).log10();
    assert!(threshold_db < 0.75, "gains={gains:?} step={threshold_db}");
    assert!(reported_db < 0.8, "gains={gains:?} step={reported_db}");

    for (resonance, range) in [
        (0.85, 0.0..0.001),
        (0.90, 0.24..0.36),
        (0.95, 0.38..0.47),
        (1.00, 0.44..0.52),
    ] {
        let samples = tail(
            FilterType::CascadedTptSvf,
            CUTOFF_HZ,
            resonance,
            FilterOversampling::Off,
            true,
        );
        let level = rms(&samples[36_000..]);
        assert!(range.contains(&level), "resonance={resonance} rms={level}");
    }

    for mode in [
        FilterOversampling::Off,
        FilterOversampling::Auto,
        FilterOversampling::X2,
        FilterOversampling::X4,
    ] {
        let samples = tail(FilterType::CascadedTptSvf, CUTOFF_HZ, 1.0, mode, false);
        let level = rms(&samples[24_000..]);
        assert!((0.44..0.52).contains(&level), "mode={mode:?} rms={level}");
        assert!(
            samples
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() < 1.0)
        );
    }
}

#[test]
fn cascaded_tpt_svf_two_pole_decays_and_control_grid_stays_finite() {
    let mut two_pole = filter(
        FilterType::CascadedTptSvf,
        CUTOFF_HZ,
        1.0,
        2,
        FilterOversampling::X4,
    );
    for _ in 0..128 {
        let _ = process(
            &mut two_pole,
            f32x4::splat(0.1),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        );
    }
    let mut first = 0.0;
    let mut last = 0.0;
    for frame in 0..24_000 {
        let output = process(
            &mut two_pole,
            f32x4::splat(0.0),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        )
        .to_array()[0];
        if frame < 2_000 {
            first += output * output;
        } else if frame >= 22_000 {
            last += output * output;
        }
    }
    assert!(last < first * 1.0e-4, "first={first} last={last}");

    for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for mode in [
            FilterOversampling::Off,
            FilterOversampling::X2,
            FilterOversampling::X4,
        ] {
            for poles in [2, 4] {
                for resonance in [0.0, 0.71, 0.9, 1.0] {
                    let mut filter = filter(
                        FilterType::CascadedTptSvf,
                        CUTOFF_HZ,
                        resonance,
                        poles,
                        mode,
                    );
                    filter.set_key_track(1.0);
                    filter.set_env_amount(1.0);
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
                        assert!(
                            output
                                .to_array()
                                .iter()
                                .all(|value| value.is_finite() && value.abs() < 10.0)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn cascaded_tpt_svf_reset_key_tracking_and_lanes_are_independent() {
    let make = || {
        filter(
            FilterType::CascadedTptSvf,
            CUTOFF_HZ,
            0.95,
            4,
            FilterOversampling::X2,
        )
    };
    let mut mixed = make();
    let mut low = make();
    let mut high = make();
    for frame in 0..512 {
        let input = f32x4::splat((frame as f32 * 0.037).sin() * 0.1);
        let render = |filter: &mut Filter, resonance_mod| {
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
        let mixed_output = render(&mut mixed, f32x4::new([-0.25, 0.05, -0.25, 0.05])).to_array();
        let low_output = render(&mut low, f32x4::splat(-0.25)).to_array();
        let high_output = render(&mut high, f32x4::splat(0.05)).to_array();
        assert!(
            (mixed_output[0] - low_output[0]).abs() < 1.0e-6,
            "frame={frame} mixed={} low={} delta={}",
            mixed_output[0],
            low_output[0],
            mixed_output[0] - low_output[0]
        );
        assert!(
            (mixed_output[1] - high_output[1]).abs() < 1.0e-6,
            "frame={frame} mixed={} high={} delta={}",
            mixed_output[1],
            high_output[1],
            mixed_output[1] - high_output[1]
        );
    }
    mixed.reset_lane(2);
    let mut fresh = make();
    let reset = process(
        &mut mixed,
        f32x4::splat(0.1),
        f32x4::splat(69.0),
        SAMPLE_RATE,
    )
    .to_array();
    let fresh_output = process(
        &mut fresh,
        f32x4::splat(0.1),
        f32x4::splat(69.0),
        SAMPLE_RATE,
    )
    .to_array();
    assert_eq!(reset[2], fresh_output[2]);
    mixed.reset();
    fresh.reset();
    let reset = process(
        &mut mixed,
        f32x4::splat(0.1),
        f32x4::splat(69.0),
        SAMPLE_RATE,
    );
    let fresh_output = process(
        &mut fresh,
        f32x4::splat(0.1),
        f32x4::splat(69.0),
        SAMPLE_RATE,
    );
    for (reset, fresh) in reset.to_array().into_iter().zip(fresh_output.to_array()) {
        assert!((reset - fresh).abs() < 1.0e-12);
    }

    for note in [36.0, 48.0, 60.0, 72.0, 84.0] {
        let mut tracked = filter(
            FilterType::CascadedTptSvf,
            110.0,
            1.0,
            4,
            FilterOversampling::Off,
        );
        tracked.set_key_track(1.0);
        let pitch_trim = tracked.self_osc_pitch_tuning_cents();
        let mut samples = Vec::with_capacity(24_000);
        for frame in 0..48_000 {
            let output = process(
                &mut tracked,
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
        let expected = 110.0 * 2.0f32.powf((note - 36.0) / 12.0 + pitch_trim / 1200.0);
        let measured = pitch(&samples);
        assert!(
            (measured / expected - 1.0).abs() < 0.05,
            "note={note} expected={expected} measured={measured}"
        );
    }
}

#[test]
fn cascaded_tpt_svf_long_running_self_oscillation_stays_bounded() {
    let mut filter = filter(
        FilterType::CascadedTptSvf,
        CUTOFF_HZ,
        1.0,
        4,
        FilterOversampling::Off,
    );
    let mut energy = 0.0;
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
            energy += output * output;
        }
    }
    assert!((energy / 24_000.0).sqrt() > 0.44);
}

#[test]
fn cascaded_tpt_svf_high_cutoff_feedback_stays_bounded() {
    for cutoff in [6_600.0, 12_000.0, 18_000.0, 20_000.0] {
        let samples = tail(
            FilterType::CascadedTptSvf,
            cutoff,
            1.0,
            FilterOversampling::Off,
            true,
        );
        let level = rms(&samples[36_000..]);
        assert!(
            samples
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() < 1.7),
            "cutoff={cutoff} level={level}"
        );
        assert!(
            (0.05..1.3).contains(&level),
            "cutoff={cutoff} level={level}"
        );
    }
}
