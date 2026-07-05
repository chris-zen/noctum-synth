use wide::f32x4;

use synth_core::{LANES, LadderFilter};

fn process(filter: &mut LadderFilter, input: f32x4, note: f32x4, sample_rate: f32) -> f32x4 {
    process_modulated(
        filter,
        input,
        note,
        f32x4::splat(0.0),
        f32x4::splat(1.0),
        f32x4::splat(0.0),
        sample_rate,
    )
}

fn process_modulated(
    filter: &mut LadderFilter,
    input: f32x4,
    note: f32x4,
    filter_env: f32x4,
    velocity: f32x4,
    osc1_audio: f32x4,
    sample_rate: f32,
) -> f32x4 {
    filter.process(
        input,
        note,
        filter_env,
        velocity,
        osc1_audio,
        f32x4::splat(0.0),
        f32x4::splat(0.0),
        f32x4::splat(0.0),
        sample_rate,
    )
}

/// Measure steady-state RMS response at a given frequency using a small
/// test signal to stay in the linear region.
fn measure_response(
    filter: &mut LadderFilter,
    freq: f32,
    cutoff: f32,
    resonance: f32,
    poles: u8,
    sample_rate: f32,
    amplitude: f32,
) -> f32 {
    let dt = 1.0 / sample_rate;
    let omega = 2.0 * std::f32::consts::PI * freq;
    let settle = (sample_rate * 0.1) as usize;
    let measure = (sample_rate * 0.03) as usize;

    filter.set_cutoff(cutoff);
    filter.set_resonance(resonance);
    filter.set_poles(poles);

    let mut phase = 0.0f32;
    for _ in 0..settle {
        let input = f32x4::splat(phase.sin() * amplitude);
        let _ = process(filter, input, f32x4::splat(60.0), sample_rate);
        phase += omega * dt;
    }

    let mut sum_sq = 0.0f32;
    for _ in 0..measure {
        let input = f32x4::splat(phase.sin() * amplitude);
        let out = process(filter, input, f32x4::splat(60.0), sample_rate);
        sum_sq += out.to_array()[0].powi(2);
        phase += omega * dt;
    }
    (sum_sq / measure as f32).sqrt()
}

fn measure_modulated_response(
    filter: &mut LadderFilter,
    freq: f32,
    note: f32,
    filter_env: f32,
    osc1_audio: f32,
    sample_rate: f32,
    amplitude: f32,
) -> f32 {
    let dt = 1.0 / sample_rate;
    let omega = 2.0 * std::f32::consts::PI * freq;
    let settle = (sample_rate * 0.1) as usize;
    let measure = (sample_rate * 0.03) as usize;
    let note = f32x4::splat(note);
    let filter_env = f32x4::splat(filter_env);
    let osc1_audio = f32x4::splat(osc1_audio);

    let mut phase = 0.0f32;
    for _ in 0..settle {
        let input = f32x4::splat(phase.sin() * amplitude);
        let _ = process_modulated(
            filter,
            input,
            note,
            filter_env,
            f32x4::splat(1.0),
            osc1_audio,
            sample_rate,
        );
        phase += omega * dt;
    }

    let mut sum_sq = 0.0f32;
    for _ in 0..measure {
        let input = f32x4::splat(phase.sin() * amplitude);
        let out = process_modulated(
            filter,
            input,
            note,
            filter_env,
            f32x4::splat(1.0),
            osc1_audio,
            sample_rate,
        );
        sum_sq += out.to_array()[0].powi(2);
        phase += omega * dt;
    }
    (sum_sq / measure as f32).sqrt()
}

fn measure_velocity_lane_response(
    filter: &mut LadderFilter,
    freq: f32,
    velocities: [f32; LANES],
    sample_rate: f32,
    amplitude: f32,
) -> [f32; LANES] {
    let dt = 1.0 / sample_rate;
    let omega = 2.0 * std::f32::consts::PI * freq;
    let settle = (sample_rate * 0.1) as usize;
    let measure = (sample_rate * 0.03) as usize;
    let note = f32x4::splat(60.0);
    let filter_env = f32x4::splat(1.0);
    let velocity = f32x4::new(velocities);
    let osc1_audio = f32x4::splat(0.0);

    let mut phase = 0.0f32;
    for _ in 0..settle {
        let input = f32x4::splat(phase.sin() * amplitude);
        let _ = process_modulated(
            filter,
            input,
            note,
            filter_env,
            velocity,
            osc1_audio,
            sample_rate,
        );
        phase += omega * dt;
    }

    let mut sum_sq = [0.0; LANES];
    for _ in 0..measure {
        let input = f32x4::splat(phase.sin() * amplitude);
        let out = process_modulated(
            filter,
            input,
            note,
            filter_env,
            velocity,
            osc1_audio,
            sample_rate,
        )
        .to_array();
        for lane in 0..LANES {
            sum_sq[lane] += out[lane].powi(2);
        }
        phase += omega * dt;
    }

    sum_sq.map(|sum| (sum / measure as f32).sqrt())
}

#[test]
fn test_dc_gain_is_unity() {
    let mut f = LadderFilter::default();
    for _ in 0..5000 {
        let out = process(&mut f, f32x4::splat(1.0), f32x4::splat(60.0), 44100.0);
        let val = out.to_array()[0];
        assert!(val.is_finite() && val.abs() < 5.0, "DC out of range: {val}");
    }
    let out = process(&mut f, f32x4::splat(1.0), f32x4::splat(60.0), 44100.0);
    let val = out.to_array()[0];
    assert!((val - 1.0).abs() < 0.3, "DC gain should be ~1.0, got {val}");
}

#[test]
fn test_cutoff_control_opens_filter() {
    let sr = 44100.0;
    let mut closed = LadderFilter::default();
    let mut open = LadderFilter::default();
    let closed_amp = measure_response(&mut closed, 2000.0, 500.0, 0.0, 4, sr, 0.1);
    let open_amp = measure_response(&mut open, 2000.0, 5000.0, 0.0, 4, sr, 0.1);

    assert!(
        open_amp > closed_amp * 10.0,
        "higher cutoff should pass more high-frequency energy: closed={closed_amp:.4} open={open_amp:.4}"
    );
}

#[test]
fn test_four_pole_attenuates_more_than_two_pole() {
    let sr = 44100.0;
    let cutoff = 1000.0;
    let mut f4 = LadderFilter::default();
    let mut f2 = LadderFilter::default();
    let amp4 = measure_response(&mut f4, 4000.0, cutoff, 0.0, 4, sr, 0.1);
    let amp2 = measure_response(&mut f2, 4000.0, cutoff, 0.0, 2, sr, 0.1);
    assert!(
        amp4 < amp2,
        "4-pole ({amp4:.4}) should attenuate more than 2-pole ({amp2:.4})"
    );
}

#[test]
fn test_two_pole_rolls_off() {
    let sr = 44100.0;
    let cutoff = 1000.0;
    let mut f = LadderFilter::default();
    let amp1k = measure_response(&mut f, 1000.0, cutoff, 0.0, 2, sr, 0.1);
    let amp4k = measure_response(&mut f, 4000.0, cutoff, 0.0, 2, sr, 0.1);
    assert!(
        amp4k < amp1k,
        "2-pole should attenuate above cutoff: 1k={amp1k:.4} 4k={amp4k:.4}"
    );
}

#[test]
fn test_resonance_creates_peak() {
    let sr = 44100.0;
    let cutoff = 1000.0;
    let mut f_flat = LadderFilter::default();
    let mut f_res = LadderFilter::default();
    let amp_flat = measure_response(&mut f_flat, 1000.0, cutoff, 0.0, 4, sr, 0.1);
    let amp_res = measure_response(&mut f_res, 1000.0, cutoff, 1.0, 4, sr, 0.1);
    assert!(
        amp_res > amp_flat * 4.0,
        "resonance should boost near cutoff: flat={amp_flat:.4} res={amp_res:.4}"
    );
}

#[test]
fn test_key_tracking_opens_cutoff_for_higher_notes() {
    let sr = 44100.0;
    let mut low_note = LadderFilter::default();
    low_note.set_cutoff(700.0);
    low_note.set_key_track(1.0);

    let mut high_note = LadderFilter::default();
    high_note.set_cutoff(700.0);
    high_note.set_key_track(1.0);

    let low_amp = measure_modulated_response(&mut low_note, 2500.0, 48.0, 0.0, 0.0, sr, 0.1);
    let high_amp = measure_modulated_response(&mut high_note, 2500.0, 72.0, 0.0, 0.0, sr, 0.1);

    assert!(
        high_amp > low_amp * 2.0,
        "key tracking should open cutoff for high notes: low={low_amp:.4} high={high_amp:.4}"
    );
}

#[test]
fn test_filter_envelope_amount_modulates_cutoff() {
    let sr = 44100.0;
    let mut closed = LadderFilter::default();
    closed.set_cutoff(400.0);
    closed.set_env_amount(1.0);

    let mut opened = LadderFilter::default();
    opened.set_cutoff(400.0);
    opened.set_env_amount(1.0);

    let closed_amp = measure_modulated_response(&mut closed, 3000.0, 60.0, 0.0, 0.0, sr, 0.1);
    let opened_amp = measure_modulated_response(&mut opened, 3000.0, 60.0, 1.0, 0.0, sr, 0.1);

    assert!(
        opened_amp > closed_amp * 4.0,
        "positive filter EG amount should open cutoff: closed={closed_amp:.4} opened={opened_amp:.4}"
    );
}

#[test]
fn test_filter_velocity_scales_envelope_per_lane() {
    let sr = 44100.0;
    let mut filter = LadderFilter::default();
    filter.set_cutoff(400.0);
    filter.set_env_amount(0.5);
    filter.set_env_velocity_amount(1.0);

    let amps = measure_velocity_lane_response(&mut filter, 3000.0, [0.0, 0.25, 0.5, 1.0], sr, 0.1);

    assert!(
        amps[0] < amps[1] && amps[1] < amps[2] && amps[2] < amps[3],
        "filter velocity should open cutoff monotonically per lane, amps={amps:?}"
    );
    assert!(
        amps[3] > amps[0] * 4.0,
        "full velocity should create substantially deeper filter EG modulation, amps={amps:?}"
    );
}

#[test]
fn test_audio_mod_modulates_cutoff_from_osc1() {
    let sr = 44100.0;
    let mut negative = LadderFilter::default();
    negative.set_cutoff(1000.0);
    negative.set_audio_mod(1.0);

    let mut positive = LadderFilter::default();
    positive.set_cutoff(1000.0);
    positive.set_audio_mod(1.0);

    let negative_amp = measure_modulated_response(&mut negative, 2500.0, 60.0, 0.0, -1.0, sr, 0.1);
    let positive_amp = measure_modulated_response(&mut positive, 2500.0, 60.0, 0.0, 1.0, sr, 0.1);

    assert!(
        positive_amp > negative_amp * 3.0,
        "positive Osc1 audio mod should open cutoff relative to negative mod: negative={negative_amp:.4} positive={positive_amp:.4}"
    );
}

#[test]
fn test_max_resonance_self_oscillates_without_blowing_up() {
    let sr = 44100.0;
    let cutoff = 440.0;
    let resonance = 1.0;
    let mut f = LadderFilter::default();
    f.set_cutoff(cutoff);
    f.set_resonance(resonance);

    // Brief impulse
    for _ in 0..10 {
        let _ = process(&mut f, f32x4::splat(0.5), f32x4::splat(60.0), sr);
    }

    let mut prev = 0.0f32;
    let mut zero_crossings = 0usize;
    let mut max_abs = 0.0f32;
    let mut first_energy = 0.0f32;
    let mut last_energy = 0.0f32;
    for i in 0..12000 {
        let out = process(&mut f, f32x4::splat(0.0), f32x4::splat(60.0), sr);
        let val = out.to_array()[0];
        max_abs = max_abs.max(val.abs());
        if i < 1024 {
            first_energy += val * val;
        } else if i >= 10976 {
            last_energy += val * val;
        }
        if prev < 0.0 && val >= 0.0 {
            zero_crossings += 1;
        }
        prev = val;
    }

    assert!(
        zero_crossings > 5,
        "max resonance should ring after an impulse, got {zero_crossings} crossings"
    );
    assert!(max_abs < 5.0, "self-oscillation blew up: {max_abs:.4}");
    assert!(
        last_energy > first_energy * 0.5,
        "max resonance should sustain instead of decay: first={first_energy:.6} last={last_energy:.6}"
    );
}

#[test]
fn test_max_resonance_self_oscillation_starts_from_silence() {
    let sr = 44100.0;
    let mut f = LadderFilter::default();
    f.set_cutoff(440.0);
    f.set_resonance(1.0);

    let mut first_energy = 0.0f32;
    let mut last_energy = 0.0f32;
    let mut max_abs = 0.0f32;
    for i in 0..60000 {
        let out = process(&mut f, f32x4::splat(0.0), f32x4::splat(60.0), sr);
        let val = out.to_array()[0];
        assert!(val.is_finite(), "NaN/Inf at i={i}");
        max_abs = max_abs.max(val.abs());
        if i < 2048 {
            first_energy += val * val;
        } else if i >= 57952 {
            last_energy += val * val;
        }
    }

    assert!(
        last_energy > first_energy * 100.0 && last_energy > 1.0e-8,
        "self-oscillation should grow from seeded silence: first={first_energy:.12} last={last_energy:.12}"
    );
    assert!(
        max_abs < 5.0,
        "seeded self-oscillation blew up: {max_abs:.4}"
    );
}

#[test]
fn test_below_self_oscillation_threshold_decays() {
    let sr = 44100.0;
    let mut f = LadderFilter::default();
    f.set_cutoff(440.0);
    f.set_resonance(0.95);

    for _ in 0..10 {
        let _ = process(&mut f, f32x4::splat(0.5), f32x4::splat(60.0), sr);
    }

    let mut first_energy = 0.0f32;
    let mut last_energy = 0.0f32;
    for i in 0..12000 {
        let out = process(&mut f, f32x4::splat(0.0), f32x4::splat(60.0), sr);
        let val = out.to_array()[0];
        assert!(val.is_finite(), "NaN/Inf at i={i}");
        if i < 1024 {
            first_energy += val * val;
        } else if i >= 10976 {
            last_energy += val * val;
        }
    }

    assert!(
        last_energy < first_energy,
        "below self-oscillation threshold should decay: first={first_energy:.6} last={last_energy:.6}"
    );
}

#[test]
fn test_simd_lanes_equal() {
    let mut f = LadderFilter::default();
    f.set_cutoff(500.0);
    f.set_resonance(0.3);
    let sr = 44100.0;
    for _ in 0..1000 {
        let input = f32x4::splat(0.5);
        let out = process(&mut f, input, f32x4::splat(60.0), sr);
        let arr = out.to_array();
        for i in 1..4 {
            assert!(
                (arr[i] - arr[0]).abs() < 1e-5,
                "SIMD lane {i} diverged: {} vs {}",
                arr[i],
                arr[0]
            );
        }
    }
}

#[test]
fn test_output_stays_bounded() {
    let mut f = LadderFilter::default();
    let sr = 44100.0;
    let mut phase = 0.0f32;
    for i in 0..10000 {
        let freq = if i < 2500 {
            100.0
        } else if i < 5000 {
            1000.0
        } else if i < 7500 {
            5000.0
        } else {
            10000.0
        };
        let res = if i < 2500 {
            0.0
        } else if i < 5000 {
            0.5
        } else if i < 7500 {
            0.9
        } else {
            1.0
        };
        let input = f32x4::splat(phase.sin());
        f.set_cutoff(2000.0);
        f.set_resonance(res);
        let out = process(&mut f, input, f32x4::splat(60.0), sr);
        let val = out.to_array()[0];
        assert!(val.is_finite(), "NaN/Inf at i={i} res={res} freq={freq}");
        assert!(
            val.abs() < 30.0,
            "output exploded to {val} at i={i} res={res} freq={freq}"
        );
        phase += 2.0 * std::f32::consts::PI * freq / sr;
    }
}

#[test]
fn test_high_cutoff_high_resonance_stays_bounded() {
    let mut f = LadderFilter::default();
    let sr = 44100.0;
    let mut phase = 0.0f32;
    f.set_cutoff(18000.0);
    f.set_resonance(1.0);

    let mut max_abs = 0.0f32;
    for i in 0..20000 {
        let input = f32x4::splat((phase.sin() * 0.5) + ((phase * 0.37).sin() * 0.25));
        let out = process(&mut f, input, f32x4::splat(84.0), sr);
        let val = out.to_array()[0];
        assert!(val.is_finite(), "NaN/Inf at i={i}");
        max_abs = max_abs.max(val.abs());
        phase += 2.0 * std::f32::consts::PI * 8000.0 / sr;
    }

    assert!(
        max_abs < 5.0,
        "high cutoff/high resonance should stay bounded, peak {max_abs:.4}"
    );
}
