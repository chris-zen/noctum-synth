use wide::f32x4;

use synth_core::{
    filter::{FilterOversampling, SELF_OSC_PITCH_TUNING_CENTS, SELF_OSC_RESONANCE_START},
    midi_to_hz, LANES, LadderFilter,
};

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

fn measure_projected_gain(
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
    let measure = (sample_rate * 0.05) as usize;

    filter.reset();
    filter.set_cutoff(cutoff);
    filter.set_resonance(resonance);
    filter.set_poles(poles);

    let mut phase = 0.0f32;
    for _ in 0..settle {
        let input = f32x4::splat(phase.sin() * amplitude);
        let _ = process(filter, input, f32x4::splat(60.0), sample_rate);
        phase += omega * dt;
    }

    let mut sin_sum = 0.0f32;
    let mut cos_sum = 0.0f32;
    for _ in 0..measure {
        let sin = phase.sin();
        let cos = phase.cos();
        let input = f32x4::splat(sin * amplitude);
        let out = process(filter, input, f32x4::splat(60.0), sample_rate).to_array()[0];
        sin_sum += out * sin;
        cos_sum += out * cos;
        phase += omega * dt;
    }

    let output_amp = 2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / measure as f32;
    output_amp / amplitude
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

#[derive(Debug, Clone, Copy)]
struct ResponseSummary {
    peak_abs: f32,
    energy: f32,
    zero_crossings: usize,
}

fn summarize_response(samples: &[f32]) -> ResponseSummary {
    let mut peak_abs = 0.0f32;
    let mut energy = 0.0f32;
    let mut zero_crossings = 0usize;
    let mut prev = 0.0f32;

    for &sample in samples {
        peak_abs = peak_abs.max(sample.abs());
        energy += sample * sample;
        if prev < 0.0 && sample >= 0.0 {
            zero_crossings += 1;
        }
        prev = sample;
    }

    ResponseSummary {
        peak_abs,
        energy,
        zero_crossings,
    }
}

fn render_impulse_response(
    filter: &mut LadderFilter,
    cutoff: f32,
    resonance: f32,
    poles: u8,
    sample_rate: f32,
    frames: usize,
) -> Vec<f32> {
    filter.reset();
    filter.set_cutoff(cutoff);
    filter.set_resonance(resonance);
    filter.set_poles(poles);

    (0..frames)
        .map(|i| {
            let input = if i == 0 { 1.0 } else { 0.0 };
            process(filter, f32x4::splat(input), f32x4::splat(60.0), sample_rate).to_array()[0]
        })
        .collect()
}

fn render_sine_sweep(
    filter: &mut LadderFilter,
    cutoff: f32,
    resonance: f32,
    poles: u8,
    start_freq: f32,
    end_freq: f32,
    sample_rate: f32,
    frames: usize,
) -> Vec<f32> {
    filter.reset();
    filter.set_cutoff(cutoff);
    filter.set_resonance(resonance);
    filter.set_poles(poles);

    let mut phase = 0.0f32;
    (0..frames)
        .map(|i| {
            let t = i as f32 / frames.max(1) as f32;
            let freq = start_freq + (end_freq - start_freq) * t;
            phase += 2.0 * std::f32::consts::PI * freq / sample_rate;
            let input = phase.sin() * 0.1;
            process(filter, f32x4::splat(input), f32x4::splat(60.0), sample_rate).to_array()[0]
        })
        .collect()
}

fn render_cutoff_sweep(
    filter: &mut LadderFilter,
    input_freq: f32,
    start_cutoff: f32,
    end_cutoff: f32,
    sample_rate: f32,
    frames: usize,
) -> Vec<f32> {
    filter.reset();
    filter.set_resonance(0.2);
    filter.set_poles(4);

    let mut phase = 0.0f32;
    (0..frames)
        .map(|i| {
            let t = i as f32 / frames.max(1) as f32;
            filter.set_cutoff(start_cutoff + (end_cutoff - start_cutoff) * t);
            phase += 2.0 * std::f32::consts::PI * input_freq / sample_rate;
            let input = phase.sin() * 0.1;
            process(filter, f32x4::splat(input), f32x4::splat(60.0), sample_rate).to_array()[0]
        })
        .collect()
}

fn render_resonance_sweep(
    filter: &mut LadderFilter,
    input_freq: f32,
    cutoff: f32,
    sample_rate: f32,
    frames: usize,
) -> Vec<f32> {
    filter.reset();
    filter.set_cutoff(cutoff);
    filter.set_poles(4);

    let mut phase = 0.0f32;
    (0..frames)
        .map(|i| {
            let t = i as f32 / frames.max(1) as f32;
            filter.set_resonance(t);
            phase += 2.0 * std::f32::consts::PI * input_freq / sample_rate;
            let input = phase.sin() * 0.05;
            process(filter, f32x4::splat(input), f32x4::splat(60.0), sample_rate).to_array()[0]
        })
        .collect()
}

fn render_self_oscillation(
    filter: &mut LadderFilter,
    cutoff: f32,
    sample_rate: f32,
    frames: usize,
) -> Vec<f32> {
    render_self_oscillation_with_note(filter, cutoff, 60.0, 0.0, sample_rate, frames)
}

fn render_self_oscillation_with_note(
    filter: &mut LadderFilter,
    cutoff: f32,
    note: f32,
    key_track: f32,
    sample_rate: f32,
    frames: usize,
) -> Vec<f32> {
    render_self_oscillation_with_note_and_tuning(
        filter,
        cutoff,
        note,
        key_track,
        SELF_OSC_PITCH_TUNING_CENTS,
        sample_rate,
        frames,
    )
}

fn render_self_oscillation_with_note_and_tuning(
    filter: &mut LadderFilter,
    cutoff: f32,
    note: f32,
    key_track: f32,
    tuning_cents: f32,
    sample_rate: f32,
    frames: usize,
) -> Vec<f32> {
    filter.reset();
    filter.set_cutoff(cutoff);
    filter.set_resonance(1.0);
    filter.set_poles(4);
    filter.set_key_track(key_track);
    filter.set_self_osc_pitch_tuning_cents(tuning_cents);

    (0..frames)
        .map(|_| process(filter, f32x4::splat(0.0), f32x4::splat(note), sample_rate).to_array()[0])
        .collect()
}

fn measure_projected_component(samples: &[f32], freq: f32, sample_rate: f32) -> f32 {
    let omega = 2.0 * std::f32::consts::PI * freq / sample_rate;
    let mut phase = 0.0f32;
    let mut sin_sum = 0.0f32;
    let mut cos_sum = 0.0f32;

    for &sample in samples {
        let sin = phase.sin();
        let cos = phase.cos();
        sin_sum += sample * sin;
        cos_sum += sample * cos;
        phase += omega;
    }

    2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / samples.len().max(1) as f32
}

fn fold_frequency(freq: f32, sample_rate: f32) -> f32 {
    let nyquist = sample_rate * 0.5;
    let period = sample_rate;
    let folded = freq.rem_euclid(period);
    if folded > nyquist {
        period - folded
    } else {
        folded
    }
}

fn estimate_frequency_from_positive_crossings(samples: &[f32], sample_rate: f32) -> f32 {
    let mut crossings = 0usize;
    let mut first_crossing = None;
    let mut last_crossing = None;
    let mut prev = samples.first().copied().unwrap_or(0.0);

    for (index, &sample) in samples.iter().enumerate().skip(1) {
        if prev < 0.0 && sample >= 0.0 {
            crossings += 1;
            first_crossing.get_or_insert(index);
            last_crossing = Some(index);
        }
        prev = sample;
    }

    let Some(first) = first_crossing else {
        return 0.0;
    };
    let Some(last) = last_crossing else {
        return 0.0;
    };
    if crossings < 2 || last <= first {
        return 0.0;
    }

    (crossings - 1) as f32 * sample_rate / (last - first) as f32
}

fn estimate_self_oscillation_pitch_hz(
    cutoff: f32,
    note: f32,
    key_track: f32,
    tuning_cents: f32,
    sample_rate: f32,
) -> f32 {
    estimate_excited_self_oscillation_pitch_hz(cutoff, note, key_track, tuning_cents, sample_rate)
}

fn estimate_excited_self_oscillation_pitch_hz(
    cutoff: f32,
    note: f32,
    key_track: f32,
    tuning_cents: f32,
    sample_rate: f32,
) -> f32 {
    let mut filter = LadderFilter::default();
    filter.reset();
    filter.set_cutoff(cutoff);
    filter.set_resonance(1.0);
    filter.set_poles(4);
    filter.set_key_track(key_track);
    filter.set_self_osc_pitch_tuning_cents(tuning_cents);

    for _ in 0..128 {
        let _ = process(
            &mut filter,
            f32x4::splat(0.1),
            f32x4::splat(note),
            sample_rate,
        );
    }

    let mut samples = Vec::with_capacity(24_000);
    for _ in 0..24_000 {
        samples.push(
            process(
                &mut filter,
                f32x4::splat(0.0),
                f32x4::splat(note),
                sample_rate,
            )
            .to_array()[0],
        );
    }

    estimate_frequency_from_positive_crossings(&samples[8_000..], sample_rate)
}

fn measure_self_oscillation_tail_energy(resonance: f32, cutoff: f32, sample_rate: f32) -> f32 {
    let mut filter = LadderFilter::default();
    filter.set_cutoff(cutoff);
    filter.set_resonance(resonance);
    filter.set_poles(4);

    for _ in 0..128 {
        let _ = process(
            &mut filter,
            f32x4::splat(0.1),
            f32x4::splat(60.0),
            sample_rate,
        );
    }

    let mut energy = 0.0;
    for i in 0..18_000 {
        let out = process(
            &mut filter,
            f32x4::splat(0.0),
            f32x4::splat(60.0),
            sample_rate,
        )
        .to_array()[0];
        assert!(out.is_finite(), "NaN/Inf at i={i}");
        if i >= 14_000 {
            energy += out * out;
        }
    }

    energy
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
fn test_neutral_filter_predicate_matches_open_unmodulated_filter() {
    let mut f = LadderFilter::default();
    assert!(f.is_neutral(), "default filter should be neutral/open");

    f.set_resonance(0.009);
    assert!(
        f.is_neutral(),
        "very low resonance should keep open/unmodulated state"
    );

    f.set_resonance(0.02);
    assert!(
        !f.is_neutral(),
        "audible resonance should change the filter state"
    );

    f.set_resonance(0.0);
    f.set_cutoff(1000.0);
    assert!(
        !f.is_neutral(),
        "closed cutoff should change the filter state"
    );

    f.set_cutoff(18_000.0);
    f.set_env_amount(0.1);
    assert!(
        !f.is_neutral(),
        "filter modulation should change the filter state"
    );
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
fn test_two_pole_resonates_without_self_oscillation() {
    let sr = 44100.0;
    let mut flat_filter = LadderFilter::default();
    let mut resonant_filter = LadderFilter::default();
    let flat = render_impulse_response(&mut flat_filter, 1000.0, 0.0, 2, sr, 4096);
    let resonant = render_impulse_response(&mut resonant_filter, 1000.0, 1.0, 2, sr, 4096);
    let flat_summary = summarize_response(&flat);
    let resonant_summary = summarize_response(&resonant);
    assert!(
        resonant_summary.zero_crossings > flat_summary.zero_crossings
            && resonant_summary.peak_abs < 0.1,
        "2-pole resonance should add ringing without self-oscillation: flat={flat_summary:?} res={resonant_summary:?}"
    );

    let mut f = LadderFilter::default();
    f.set_cutoff(440.0);
    f.set_resonance(1.0);
    f.set_poles(2);
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
        last_energy < first_energy * 0.1,
        "2-pole mode should decay rather than self-oscillate: first={first_energy:.6} last={last_energy:.6}"
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
fn test_four_pole_resonance_compensates_low_frequency_loss() {
    let sr = 44100.0;
    let cutoff = 1000.0;
    let mut flat = LadderFilter::default();
    let mut resonant = LadderFilter::default();
    let flat_amp = measure_response(&mut flat, 100.0, cutoff, 0.0, 4, sr, 0.02);
    let resonant_amp = measure_response(&mut resonant, 100.0, cutoff, 0.8, 4, sr, 0.02);
    let ratio = resonant_amp / flat_amp;

    assert!(
        (0.95..=1.25).contains(&ratio),
        "4-pole bass compensation should keep low-frequency response near unity: flat={flat_amp:.4} resonant={resonant_amp:.4} ratio={ratio:.3}"
    );
}

#[test]
fn test_four_pole_resonance_compensation_survives_musical_level() {
    let sr = 44100.0;
    let cutoff = 1000.0;
    let amplitude = 0.35;
    let mut flat = LadderFilter::default();
    let mut resonant = LadderFilter::default();
    let flat_amp = measure_response(&mut flat, 100.0, cutoff, 0.0, 4, sr, amplitude);
    let resonant_amp = measure_response(&mut resonant, 100.0, cutoff, 0.8, 4, sr, amplitude);
    let ratio = resonant_amp / flat_amp;

    assert!(
        (0.9..=1.25).contains(&ratio),
        "4-pole bass compensation should stay near unity at musical signal levels: flat={flat_amp:.4} resonant={resonant_amp:.4} ratio={ratio:.3}"
    );
    assert!(
        resonant_amp.is_finite() && resonant_amp < amplitude * 4.0,
        "4-pole bass compensation should stay bounded at musical signal levels: resonant={resonant_amp:.4}"
    );
}

#[test]
fn test_max_resonance_open_filter_passband_stays_near_unity() {
    let sr = 44100.0;
    let cutoff = 18000.0;
    let freq = 1000.0;
    let mut flat = LadderFilter::default();
    let mut resonant = LadderFilter::default();
    for amplitude in [0.02, 0.35] {
        let flat_gain = measure_projected_gain(&mut flat, freq, cutoff, 0.0, 4, sr, amplitude);
        let resonant_gain =
            measure_projected_gain(&mut resonant, freq, cutoff, 1.0, 4, sr, amplitude);
        let ratio = resonant_gain / flat_gain;
        let db = 20.0 * ratio.log10();

        assert!(
            (-1.0..=1.0).contains(&db),
            "max-resonance open filter passband should stay near unity at amplitude {amplitude}: flat={flat_gain:.4} resonant={resonant_gain:.4} ratio={ratio:.3} db={db:.2}"
        );
    }
}

#[test]
fn test_resonance_peak_survives_self_oscillation_threshold() {
    let sr = 44100.0;
    let cutoff = 1000.0;
    let freq = cutoff;
    let amplitude = 0.05;
    for (below_resonance, above_resonance) in [(0.89, 0.91), (0.955, 0.975)] {
        let mut below = LadderFilter::default();
        let mut above = LadderFilter::default();
        let below_gain =
            measure_projected_gain(&mut below, freq, cutoff, below_resonance, 4, sr, amplitude);
        let above_gain =
            measure_projected_gain(&mut above, freq, cutoff, above_resonance, 4, sr, amplitude);
        let ratio = above_gain / below_gain;

        assert!(
            ratio > 0.75,
            "resonance peak should not collapse across resonance {below_resonance:.3}->{above_resonance:.3}: below={below_gain:.4} above={above_gain:.4} ratio={ratio:.3}"
        );
    }
}

#[test]
fn test_max_resonance_keeps_cutoff_peak() {
    let sr = 44100.0;
    let cutoff = 1000.0;
    let max_resonance_freq = cutoff * 2.0f32.powf(SELF_OSC_PITCH_TUNING_CENTS / 1200.0);
    let amplitude = 0.02;
    let mut pre_self_osc = LadderFilter::default();
    let mut max_resonance = LadderFilter::default();
    let pre_gain =
        measure_projected_gain(&mut pre_self_osc, cutoff, cutoff, SELF_OSC_RESONANCE_START - 0.02, 4, sr, amplitude);
    let max_gain = measure_projected_gain(
        &mut max_resonance,
        max_resonance_freq,
        cutoff,
        1.0,
        4,
        sr,
        amplitude,
    );
    let ratio = max_gain / pre_gain;

    assert!(
        ratio > 0.55 && max_gain > 5.5,
        "max resonance should keep a strong calibrated cutoff peak: pre={pre_gain:.4} max={max_gain:.4} ratio={ratio:.3}"
    );
}

#[test]
fn test_filter_measurement_helpers_generate_finite_summaries() {
    let sr = 44100.0;
    let mut impulse_filter = LadderFilter::default();
    let impulse = render_impulse_response(&mut impulse_filter, 1000.0, 0.4, 4, sr, 2048);
    let impulse_summary = summarize_response(&impulse);
    assert!(impulse_summary.peak_abs > 0.0 && impulse_summary.peak_abs < 2.0);
    assert!(impulse_summary.energy.is_finite() && impulse_summary.energy > 0.0);

    let mut sine_filter = LadderFilter::default();
    let sine_sweep = render_sine_sweep(&mut sine_filter, 1200.0, 0.35, 4, 100.0, 4000.0, sr, 2048);
    let sine_summary = summarize_response(&sine_sweep);
    assert!(sine_summary.energy.is_finite() && sine_summary.energy > 0.0);

    let mut cutoff_filter = LadderFilter::default();
    let cutoff_sweep = render_cutoff_sweep(&mut cutoff_filter, 2500.0, 300.0, 6000.0, sr, 2048);
    let cutoff_summary = summarize_response(&cutoff_sweep);
    assert!(cutoff_summary.energy.is_finite() && cutoff_summary.energy > 0.0);

    let mut resonance_filter = LadderFilter::default();
    let resonance_sweep = render_resonance_sweep(&mut resonance_filter, 1000.0, 1000.0, sr, 2048);
    let resonance_summary = summarize_response(&resonance_sweep);
    assert!(resonance_summary.energy.is_finite() && resonance_summary.peak_abs < 5.0);

    let mut self_osc_filter = LadderFilter::default();
    let self_osc = render_self_oscillation(&mut self_osc_filter, 440.0, sr, 60_000);
    let self_osc_summary = summarize_response(&self_osc[58_000..]);
    assert!(self_osc_summary.peak_abs.is_finite() && self_osc_summary.peak_abs < 5.0);
    assert!(
        self_osc_summary.zero_crossings > 5,
        "self-oscillation helper should capture an oscillating tail: {self_osc_summary:?}"
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
fn test_prophet_reference_self_oscillation_pitch_without_key_tracking() {
    let sr = 44100.0;
    let pitch_hz =
        estimate_self_oscillation_pitch_hz(444.0, 69.0, 0.0, SELF_OSC_PITCH_TUNING_CENTS, sr);

    assert!(
        (440.0..=490.0).contains(&pitch_hz),
        "max-resonance self-oscillation at cutoff 444 Hz without key tracking should stay in the measured high-400 Hz region: pitch={pitch_hz:.2}Hz tuning={SELF_OSC_PITCH_TUNING_CENTS:.1}c"
    );
}

#[test]
fn test_key_tracked_self_oscillation_is_close_to_c4_seventh_harmonic() {
    let sr = 44100.0;
    let c4 = midi_to_hz(60);
    let target_hz = c4 * 7.0;
    let pitch_hz =
        estimate_self_oscillation_pitch_hz(444.0, 60.0, 1.0, SELF_OSC_PITCH_TUNING_CENTS, sr);
    let beat_hz = (pitch_hz - target_hz).abs();

    assert!(
        beat_hz <= 8.0,
        "key-tracked cutoff 444 Hz self-oscillation should beat slowly against C4's 7th harmonic near the audible mix point: pitch={pitch_hz:.2}Hz target={target_hz:.2}Hz beat={beat_hz:.2}Hz tuning={SELF_OSC_PITCH_TUNING_CENTS:.1}c"
    );
}

#[test]
#[ignore = "prints the best cents trim for the C4/key-tracked/cutoff 444 Hz self-oscillation beat-rate target"]
fn calibrate_self_oscillation_pitch_tuning_cents_for_key_tracked_c4() {
    let sr = 44100.0;
    let c4 = midi_to_hz(60);
    let target_hz = c4 * 7.0;
    let mut best_cents = 0.0;
    let mut best_pitch_hz = 0.0;
    let mut best_beat_hz = f32::INFINITY;

    for cents in 110..=150 {
        let cents = cents as f32;
        let pitch_hz = estimate_self_oscillation_pitch_hz(
            444.0,
            60.0,
            1.0,
            cents,
            sr,
        );
        let beat_hz = (pitch_hz - target_hz).abs();

        if beat_hz < best_beat_hz {
            best_cents = cents;
            best_pitch_hz = pitch_hz;
            best_beat_hz = beat_hz;
        }
    }

    println!(
        "best SELF_OSC_PITCH_TUNING_CENTS={best_cents:.1}, key_tracked_c4_pitch={best_pitch_hz:.3}Hz, c4_7th_harmonic={target_hz:.3}Hz, beat={best_beat_hz:.3}Hz"
    );

    assert!(
        best_beat_hz <= 3.0,
        "best self-oscillation tuning should get within the estimator resolution of C4's 7th harmonic"
    );
}

#[test]
fn test_prophet_reference_key_tracking_pushes_c4_self_oscillation_high() {
    let sr = 44100.0;
    let mut filter = LadderFilter::default();
    let samples = render_self_oscillation_with_note(&mut filter, 444.0, 60.0, 1.0, sr, 70_000);
    let pitch_hz = estimate_frequency_from_positive_crossings(&samples[50_000..], sr);

    assert!(
        (1750.0..=2050.0).contains(&pitch_hz),
        "max key tracking at C4 should move cutoff 444 Hz self-oscillation near the measured Prophet 1.9 kHz region, got {pitch_hz:.2} Hz"
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
    assert!(
        max_abs > 0.25,
        "seeded self-oscillation should reach an audible level: {max_abs:.4}"
    );
}

#[test]
fn test_max_resonance_self_oscillation_harmonics_stay_subtle() {
    let sr = 44100.0;
    let mut f = LadderFilter::default();
    f.set_cutoff(440.0);
    f.set_resonance(1.0);

    let mut samples = Vec::with_capacity(70_000);
    for _ in 0..70_000 {
        samples.push(process(&mut f, f32x4::splat(0.0), f32x4::splat(60.0), sr).to_array()[0]);
    }
    let tail = &samples[50_000..];
    let fundamental_hz = estimate_frequency_from_positive_crossings(tail, sr);
    let fundamental = measure_projected_component(tail, fundamental_hz, sr);
    let second = measure_projected_component(tail, fundamental_hz * 2.0, sr);
    let third = measure_projected_component(tail, fundamental_hz * 3.0, sr);
    let strongest_harmonic = second.max(third);
    let ratio = strongest_harmonic / fundamental.max(1.0e-9);

    assert!(
        ratio < 0.02,
        "self-oscillation harmonics should stay subtle: fundamental_hz={fundamental_hz:.2} fundamental={fundamental:.4} second={second:.4} third={third:.4} ratio={ratio:.3}"
    );
}

#[test]
fn test_below_self_oscillation_threshold_decays() {
    let sr = 44100.0;
    let mut f = LadderFilter::default();
    f.set_cutoff(440.0);
    f.set_resonance(SELF_OSC_RESONANCE_START - 0.02);

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
fn test_self_oscillation_spans_wide_resonance_range_and_level_rises() {
    let sr = 44100.0;
    let cutoff = 440.0;
    let below = measure_self_oscillation_tail_energy(SELF_OSC_RESONANCE_START - 0.02, cutoff, sr);
    let onset = measure_self_oscillation_tail_energy(SELF_OSC_RESONANCE_START + 0.02, cutoff, sr);
    let mid = measure_self_oscillation_tail_energy(0.85, cutoff, sr);
    let max = measure_self_oscillation_tail_energy(1.0, cutoff, sr);

    assert!(
        onset > below * 2.0,
        "self-oscillation should begin soon after resonance start {SELF_OSC_RESONANCE_START:.2}: below={below:.6} onset={onset:.6}"
    );
    assert!(
        mid > onset * 1.5 && max > mid * 1.5,
        "self-oscillation level should rise across the resonance range: onset={onset:.6} mid={mid:.6} max={max:.6}"
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

#[test]
fn test_filter_oversampling_auto_resolution() {
    assert_eq!(FilterOversampling::Off.factor(44_100.0), 1);
    assert_eq!(FilterOversampling::Auto.factor(44_100.0), 4);
    assert_eq!(FilterOversampling::Auto.factor(48_000.0), 4);
    assert_eq!(FilterOversampling::Auto.factor(96_000.0), 2);
    assert_eq!(FilterOversampling::Auto.factor(192_000.0), 1);
    assert_eq!(FilterOversampling::X2.factor(192_000.0), 2);
    assert_eq!(FilterOversampling::X4.factor(192_000.0), 4);
}

#[test]
fn test_filter_oversampling_does_not_affect_two_pole_mode() {
    let sr = 44100.0;
    let mut off = LadderFilter::default();
    let mut x4 = LadderFilter::default();
    off.set_oversampling(FilterOversampling::Off);
    x4.set_oversampling(FilterOversampling::X4);
    off.set_poles(2);
    x4.set_poles(2);
    off.set_cutoff(1800.0);
    x4.set_cutoff(1800.0);
    off.set_resonance(1.0);
    x4.set_resonance(1.0);

    let mut phase = 0.0f32;
    let mut max_diff = 0.0f32;
    for _ in 0..5000 {
        let input = f32x4::splat(phase.sin() * 0.25);
        let off_out = process(&mut off, input, f32x4::splat(60.0), sr).to_array()[0];
        let x4_out = process(&mut x4, input, f32x4::splat(60.0), sr).to_array()[0];
        max_diff = max_diff.max((off_out - x4_out).abs());
        phase += std::f32::consts::TAU * 330.0 / sr;
    }

    assert!(
        max_diff < 1.0e-6,
        "oversampling should not alter two-pole processing: max_diff={max_diff:.8}"
    );
}

#[test]
fn test_filter_oversampling_does_not_affect_four_pole_below_self_oscillation() {
    let sr = 44100.0;
    let mut off = LadderFilter::default();
    let mut x4 = LadderFilter::default();
    off.set_oversampling(FilterOversampling::Off);
    x4.set_oversampling(FilterOversampling::X4);
    off.set_cutoff(1800.0);
    x4.set_cutoff(1800.0);
    off.set_resonance(SELF_OSC_RESONANCE_START - 0.02);
    x4.set_resonance(SELF_OSC_RESONANCE_START - 0.02);

    let mut phase = 0.0f32;
    let mut max_diff = 0.0f32;
    for _ in 0..5000 {
        let input = f32x4::splat(phase.sin() * 0.25);
        let off_out = process(&mut off, input, f32x4::splat(60.0), sr).to_array()[0];
        let x4_out = process(&mut x4, input, f32x4::splat(60.0), sr).to_array()[0];
        max_diff = max_diff.max((off_out - x4_out).abs());
        phase += std::f32::consts::TAU * 330.0 / sr;
    }

    assert!(
        max_diff < 1.0e-6,
        "oversampling should not alter four-pole processing below self-oscillation: max_diff={max_diff:.8}"
    );
}

#[test]
fn test_filter_oversampling_reduces_high_cutoff_foldback() {
    let sr = 44100.0;
    let cutoff = 9000.0;
    let render = |mode: FilterOversampling| {
        let mut f = LadderFilter::default();
        f.set_oversampling(mode);
        f.set_cutoff(cutoff);
        f.set_resonance(1.0);
        let mut samples = Vec::with_capacity(90_000);
        for _ in 0..90_000 {
            samples.push(
                process(&mut f, f32x4::splat(0.0), f32x4::splat(60.0), sr).to_array()[0],
            );
        }
        samples
    };

    let off = render(FilterOversampling::Off);
    let x4 = render(FilterOversampling::X4);
    let off_tail = &off[50_000..];
    let x4_tail = &x4[50_000..];
    let off_fundamental_hz = estimate_frequency_from_positive_crossings(off_tail, sr);
    let x4_fundamental_hz = estimate_frequency_from_positive_crossings(x4_tail, sr);
    let off_folded_third_hz = fold_frequency(off_fundamental_hz * 3.0, sr);
    let x4_folded_third_hz = fold_frequency(x4_fundamental_hz * 3.0, sr);
    let off_fundamental = measure_projected_component(off_tail, off_fundamental_hz, sr);
    let x4_fundamental = measure_projected_component(x4_tail, x4_fundamental_hz, sr);
    let off_alias = measure_projected_component(off_tail, off_folded_third_hz, sr);
    let x4_alias = measure_projected_component(x4_tail, x4_folded_third_hz, sr);
    let off_ratio = off_alias / off_fundamental.max(1.0e-9);
    let x4_ratio = x4_alias / x4_fundamental.max(1.0e-9);

    assert!(
        x4_fundamental > off_fundamental * 0.25,
        "4x oversampling should preserve the main self-oscillation component: off={off_fundamental:.4} x4={x4_fundamental:.4}"
    );
    assert!(
        x4_ratio < off_ratio * 0.75,
        "4x oversampling should reduce folded third-harmonic energy: off_ratio={off_ratio:.4} x4_ratio={x4_ratio:.4} off_f={off_fundamental_hz:.1}Hz x4_f={x4_fundamental_hz:.1}Hz"
    );
}
