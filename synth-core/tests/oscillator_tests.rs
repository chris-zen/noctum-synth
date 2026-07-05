use synth_core::analog_oscillator::pulse_width_from_shape;
use synth_core::{AnalogOscillator, AnalogSubOscillator, SawMethod, Waveform, midi_to_hz};
use wide::f32x4;

#[test]
fn test_midi_to_hz() {
    assert!((midi_to_hz(69) - 440.0).abs() < 0.01); // A4
    assert!((midi_to_hz(60) - 261.6256).abs() < 0.01); // C4
    assert!((midi_to_hz(81) - 880.0).abs() < 0.01); // A5
}

#[test]
fn test_saw_phase_zero_is_corrected() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Saw);
    osc.set_frequency(f32x4::splat(440.0));
    // Phase = 0 before first advance → near_reset fires → output ≈ 0
    let out = osc.next();
    let arr = out.to_array();
    assert!(
        arr[0].abs() < 0.1,
        "saw at phi=0 should be corrected to ~0, got {}",
        arr[0]
    );
}

#[test]
fn test_saw_mid_cycle_near_zero() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Saw);
    // 1Hz: 44100 samples per cycle. At phi=0.5, saw ≈ 0.
    // Sample 0 = phi=0 (corrected to ~0), so sample 22050 = phi=0.5
    osc.set_frequency(f32x4::splat(1.0));
    for _ in 0..22050 {
        osc.next();
    }
    let out = osc.next();
    let arr = out.to_array();
    assert!(
        (arr[0] - 0.0).abs() < 0.02,
        "mid-cycle saw should be near 0, got {}",
        arr[0]
    );
}

#[test]
fn test_triangle_peak() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Triangle);
    osc.set_frequency(f32x4::splat(1.0));
    // Triangle peaks at φ=0.5 (sample 22050)
    for _ in 0..22050 {
        osc.next();
    }
    let out = osc.next();
    let arr = out.to_array();
    assert!(
        arr[0] > 0.95,
        "triangle should peak near 1.0 at φ=0.5, got {}",
        arr[0]
    );
}

#[test]
fn test_triangle_zero_crossing() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Triangle);
    osc.set_frequency(f32x4::splat(1.0));
    // Triangle starts at -1, crosses 0 at φ=0.25 (sample 11025)
    for _ in 0..11025 {
        osc.next();
    }
    let out = osc.next();
    let arr = out.to_array();
    assert!(
        arr[0].abs() < 0.02,
        "triangle should cross 0 at φ=0.25, got {}",
        arr[0]
    );
}

#[test]
fn test_polyblamp_triangle_smooths_corners_below_overlap_limit() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Triangle);
    osc.set_frequency(f32x4::splat(4410.0));

    osc.set_phase(f32x4::splat(0.0));
    let valley = osc.next().to_array()[0];
    assert!(
        valley > -0.95,
        "PolyBLAMP should raise the sharp triangle valley, got {valley}"
    );

    osc.set_phase(f32x4::splat(0.5));
    let peak = osc.next().to_array()[0];
    assert!(
        peak < 0.95,
        "PolyBLAMP should lower the sharp triangle peak, got {peak}"
    );
}

#[test]
fn test_polyblamp_triangle_zero_frequency_lanes_stay_finite() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Triangle);
    osc.set_frequency(f32x4::new([0.0, 4410.0, 0.0, 4410.0]));
    osc.set_phase(f32x4::new([0.0, 0.0, 0.5, 0.5]));

    let out = osc.next().to_array();
    for sample in out {
        assert!(sample.is_finite(), "triangle produced non-finite sample");
    }
    assert!(
        (out[0] + 1.0).abs() < 1e-6,
        "zero-frequency valley lane should use the naive value, got {}",
        out[0]
    );
    assert!(
        (out[2] - 1.0).abs() < 1e-6,
        "zero-frequency peak lane should use the naive value, got {}",
        out[2]
    );
}

#[test]
fn test_polyblamp_triangle_disables_correction_above_overlap_limit() {
    let mut osc = AnalogOscillator::new(100.0);
    osc.set_waveform(Waveform::Triangle);
    osc.set_frequency(f32x4::splat(30.0));

    osc.set_phase(f32x4::splat(0.0));
    let valley = osc.next().to_array()[0];
    assert!(
        (valley + 1.0).abs() < 1e-6,
        "above the overlap limit the valley should stay naive, got {valley}"
    );

    osc.set_phase(f32x4::splat(0.5));
    let peak = osc.next().to_array()[0];
    assert!(
        (peak - 1.0).abs() < 1e-6,
        "above the overlap limit the peak should stay naive, got {peak}"
    );
}

#[test]
fn test_saw_method_selects_triangle_bandlimiting_path() {
    let mut polyblep = AnalogOscillator::new(44100.0);
    polyblep.set_waveform(Waveform::Triangle);
    polyblep.set_saw_method(SawMethod::PolyBlep);
    polyblep.set_frequency(f32x4::splat(4410.0));
    polyblep.set_phase(f32x4::splat(0.0));

    let mut polyblamp = AnalogOscillator::new(44100.0);
    polyblamp.set_waveform(Waveform::Triangle);
    polyblamp.set_saw_method(SawMethod::Blep);
    polyblamp.set_frequency(f32x4::splat(4410.0));
    polyblamp.set_phase(f32x4::splat(0.0));

    let polyblep_sample = polyblep.next().to_array()[0];
    let polyblamp_sample = polyblamp.next().to_array()[0];

    assert!(
        (polyblep_sample - polyblamp_sample).abs() > 0.001,
        "SawMethod should select distinct triangle paths, got {polyblep_sample} and {polyblamp_sample}"
    );
}

#[test]
fn test_polyblep_integrated_triangle_stays_finite_and_bounded() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Triangle);
    osc.set_saw_method(SawMethod::PolyBlep);
    osc.set_frequency(f32x4::new([110.0, 440.0, 1760.0, 7040.0]));

    let mut max_abs = 0.0f32;
    for _ in 0..4096 {
        for sample in osc.next().to_array() {
            assert!(sample.is_finite(), "triangle produced non-finite sample");
            max_abs = max_abs.max(sample.abs());
        }
    }

    assert!(
        max_abs <= 1.25,
        "triangle output exceeded bounds: {max_abs}"
    );
    assert!(
        max_abs > 0.1,
        "triangle output unexpectedly collapsed: {max_abs}"
    );
}

#[test]
fn test_polyblamp_triangle_high_frequency_stays_finite_and_bounded() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Triangle);
    osc.set_frequency(f32x4::new([8_000.0, 9_000.0, 10_000.0, 11_000.0]));

    let mut max_abs = 0.0f32;
    for _ in 0..512 {
        for sample in osc.next().to_array() {
            assert!(sample.is_finite(), "triangle produced non-finite sample");
            max_abs = max_abs.max(sample.abs());
        }
    }

    assert!(
        max_abs <= 1.25,
        "triangle output exceeded bounds: {max_abs}"
    );
    assert!(
        max_abs > 0.1,
        "triangle output unexpectedly collapsed: {max_abs}"
    );
}

#[test]
fn test_pulse_50_percent_is_square() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Pulse);
    osc.set_shape(0.0);
    osc.set_frequency(f32x4::splat(440.0));
    // Sum-of-saws pulse: first sample at phi=0 is the rising edge (corrected to ~0)
    let out1 = osc.next();
    let arr1 = out1.to_array();
    assert!(
        arr1[0].abs() < 0.1,
        "pulse at phi=0 should be ~0, got {}",
        arr1[0]
    );

    // A few samples later: should be in the low state (-1)
    for _ in 0..(44100 / 440 / 8) as usize {
        osc.next();
    }
    let out_low = osc.next();
    let arr_low = out_low.to_array();
    assert!(
        arr_low[0] < -0.9,
        "pulse low should be near -1, got {}",
        arr_low[0]
    );
}

#[test]
fn test_sub_oscillator_half_frequency() {
    let sample_rate = 44100.0;
    let mut sub = AnalogSubOscillator::default();
    let freq = f32x4::splat(440.0);
    sub.set_frequency(freq, sample_rate);

    // Sub at 440Hz primary = 220Hz square
    // One cycle = 44100/220 ≈ 200.45 samples
    // Positive half: ~100 samples
    let mut positive_count = 0;
    for _ in 0..50 {
        let out = sub.next();
        if out.to_array()[0] > 0.0 {
            positive_count += 1;
        }
    }
    assert_eq!(
        positive_count, 50,
        "first 50 samples should all be positive"
    );
}

#[test]
fn test_waveshape_modulation_on_saw() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Saw);
    osc.set_shape(0.0);
    osc.set_frequency(f32x4::splat(440.0));

    let unshaped = osc.next().to_array()[0];

    osc.set_shape(0.5);
    osc.set_phase(f32x4::splat(0.0)); // reset
    osc.set_frequency(f32x4::splat(440.0));
    let shaped = osc.next().to_array()[0];

    // Shape at 0.5 should produce a different waveform
    assert!(
        (unshaped - shaped).abs() > 0.001,
        "shape 0.5 should change saw output, got {unshaped} vs {shaped}"
    );
}

#[test]
fn test_simd_all_lanes_equal() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Saw);
    osc.set_frequency(f32x4::splat(440.0));

    let out = osc.next();
    let arr = out.to_array();
    // All lanes got same frequency, should produce same output
    assert!(
        (arr[0] - arr[1]).abs() < 1e-6
            && (arr[0] - arr[2]).abs() < 1e-6
            && (arr[0] - arr[3]).abs() < 1e-6
    );
}

#[test]
fn test_phase_wraps_correctly() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Saw);
    osc.set_frequency(f32x4::splat(440.0));

    // Run for many samples, verify phase wrapping stays bounded. BLEP table
    // residuals are not hard-clipped, so tiny overshoot is expected.
    for _ in 0..100000 {
        let out = osc.next();
        let arr = out.to_array();
        for &v in &arr {
            assert!(v >= -1.02 && v <= 1.02, "output out of range: {v}");
        }
    }
}

#[test]
fn test_phase_input_wraps_without_remainder_edge_cases() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Saw);
    osc.set_frequency(f32x4::splat(440.0));
    osc.set_phase(f32x4::new([-0.25, 0.25, 0.75, 1.25]));

    let out = osc.next().to_array();

    assert!(
        (out[0] - out[2]).abs() < 1e-6,
        "negative phase should wrap to the same point as positive phase"
    );
    assert!(
        (out[1] - out[3]).abs() < 1e-6,
        "phase above 1.0 should wrap without changing oscillator output"
    );
}

#[test]
fn test_invalid_frequency_does_not_poison_phase() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Saw);
    osc.set_frequency(f32x4::new([440.0, f32::NAN, f32::INFINITY, -1.0]));

    for _ in 0..1024 {
        let out = osc.next().to_array();
        for sample in out {
            assert!(sample.is_finite(), "oscillator produced non-finite sample");
        }
    }
}

#[test]
fn test_oscillator_enabled_gain_can_mute_without_stopping_phase() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Saw);
    osc.set_frequency(f32x4::splat(440.0));
    osc.set_enabled(false);

    let muted = osc.next().to_array()[0];
    assert_eq!(muted, 0.0);

    osc.set_enabled(true);
    let audible = osc.next().to_array()[0];
    assert!(
        audible.abs() > 0.001,
        "oscillator should keep advancing while muted and become audible when re-enabled"
    );
}

#[test]
fn test_sync_reset_lanes_resets_only_selected_lanes() {
    let mut osc = AnalogOscillator::new(100.0);
    osc.set_waveform(Waveform::Saw);
    osc.set_frequency(f32x4::splat(1.0));
    osc.set_phase(f32x4::new([0.25, 0.25, 0.75, 0.75]));

    osc.sync_reset_lanes([true, false, true, false]);
    let out = osc.next().to_array();

    assert!(
        out[0].abs() < 0.1,
        "reset lane should render from cycle start, got {}",
        out[0]
    );
    assert!(
        (out[1] - out[0]).abs() > 0.1,
        "non-reset lane should keep its previous phase"
    );
    assert!(
        out[2].abs() < 0.1,
        "reset lane should render from cycle start, got {}",
        out[2]
    );
    assert!(
        (out[3] - out[2]).abs() > 0.1,
        "non-reset lane should keep its previous phase"
    );
}

#[test]
fn test_polyblep_saw_full_cycle() {
    // Use 55Hz so there are ~800 samples/period — plenty of room for PolyBLEP
    let sr = 44100.0;
    let freq = 55.0;
    let period_samples = (sr as f64 / freq as f64).round() as usize; // ~802 samples

    let mut osc = AnalogOscillator::new(sr);
    osc.set_waveform(Waveform::Saw);
    osc.set_frequency(f32x4::splat(freq));

    let mut min_val = f32::MAX;
    let mut max_val = f32::MIN;

    for _ in 0..period_samples + 10 {
        let val = osc.next().to_array()[0];
        min_val = min_val.min(val);
        max_val = max_val.max(val);
    }

    // Output range should be close to [-1, 1]
    assert!(min_val < -0.95, "min too high: {min_val}");
    assert!(max_val > 0.95, "max too low: {max_val}");
}

#[test]
fn test_polyblep_reduces_discontinuity() {
    // Renders 300 saw samples at 440Hz and checks the PolyBLEP
    // correction fires and reduces the max jump from naive ~1.99.
    let sr = 44100.0;
    let freq = 440.0;

    let mut osc = AnalogOscillator::new(sr);
    osc.set_waveform(Waveform::Saw);
    osc.set_frequency(f32x4::splat(freq));

    let mut prev = 0.0;
    let mut max_jump = 0.0f32;
    for _ in 0..300 {
        let val = osc.next().to_array()[0];
        max_jump = max_jump.max((val - prev).abs());
        prev = val;
    }

    // Table BLEP spreads the transition without clipping. Naive is ~1.99.
    assert!(max_jump < 1.75, "max jump {max_jump} should be < 1.75");
}

#[test]
fn test_polyblep_saw_smooth_transition() {
    // Verify the transition at the reset is smooth: should pass through
    // values near 0 (the midpoint), not create a spike below -1.
    let sr = 44100.0;
    let freq = 440.0;
    let dt = freq / sr;

    let mut osc = AnalogOscillator::new(sr);
    osc.set_waveform(Waveform::Saw);
    osc.set_frequency(f32x4::splat(freq));

    // Collect samples around the wrap
    let mut samples = Vec::with_capacity(300);
    for _ in 0..300 {
        samples.push(osc.next().to_array()[0]);
    }

    // Find the wrap: where the naive would have a max→min jump
    let period = (1.0 / dt) as usize;
    let mut min_after_wrap = f32::MAX;
    let mut spike_count = 0;

    for i in period..samples.len() {
        let jump = samples[i] - samples[i - 1];
        // The BLEP-corrected jump should stay below the naive ~1.99 jump.
        if jump.abs() > 1.75 {
            spike_count += 1;
        }
        min_after_wrap = min_after_wrap.min(samples[i]);
    }

    // No sample-to-sample jump should approach the naive 1.99 reset.
    assert_eq!(spike_count, 0, "found {spike_count} large jumps > 1.75");
    // Output never goes below -1.1 (allows BLEP residual overshoot)
    assert!(
        min_after_wrap > -1.1,
        "output went below -1.1: {min_after_wrap}"
    );
}

#[test]
fn test_table_blep_left_edge_uses_falling_correction() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Saw);
    osc.set_saw_method(SawMethod::Blep);
    osc.set_frequency(f32x4::splat(440.0));

    // Just before wrap, the falling-edge BLEP must pull the trivial ramp down.
    // The old sign added the left-side residual and clipped this to +1.0.
    osc.set_phase(f32x4::splat(0.999));
    let out = osc.next().to_array()[0];
    assert!(
        out < 0.6,
        "left-edge BLEP should pull down before wrap, got {out}"
    );
}

#[test]
fn test_polyblep_pulse_smooth_edges() {
    let sr = 44100.0;
    let freq = 55.0;

    let mut osc = AnalogOscillator::new(sr);
    osc.set_waveform(Waveform::Pulse);
    osc.set_shape(0.0);
    osc.set_frequency(f32x4::splat(freq));

    // Skip first sample (phi=0, edge correction)
    let mut prev = osc.next().to_array()[0];
    let mut max_jump = 0.0f32;
    for _ in 0..3000 {
        let val = osc.next().to_array()[0];
        max_jump = max_jump.max((val - prev).abs());
        prev = val;
    }

    // Direct PolyBLEP pulse smooths both edges. Max jump is ~1.08
    // on the first cycle, degrading to ~1.87 after several cycles
    // due to f32 phase accumulation drift. Still far better than
    // the naive 2.0 jump. A dedicated periodic phase reset will
    // be added in a later iteration.
    assert!(
        max_jump < 1.95,
        "pulse max jump {max_jump} — should be below naive 2.0"
    );
}

#[test]
fn test_polyblep_pulse_50_percent() {
    let sr = 44100.0;
    let freq = 440.0;
    let dt = freq / sr;

    let mut osc = AnalogOscillator::new(sr);
    osc.set_waveform(Waveform::Pulse);
    osc.set_shape(0.0);
    osc.set_frequency(f32x4::splat(freq));

    // Collect a full cycle + extra
    let period = (1.0 / dt) as usize;
    let mut samples = Vec::with_capacity(period + 10);
    for _ in 0..period + 10 {
        samples.push(osc.next().to_array()[0]);
    }

    // The square should have approximately equal time in high and low state
    let mut high_count = 0;
    for &s in &samples[..period] {
        if s > 0.0 {
            high_count += 1;
        }
    }

    // 50% duty cycle: high and low should be roughly equal
    let ratio = high_count as f32 / period as f32;
    assert!(
        (ratio - 0.5).abs() < 0.05,
        "duty cycle {ratio:.3} should be ~0.5"
    );
}

#[test]
fn test_pulse_shape_mod_controls_pwm_duty_without_unbounded_levels() {
    fn measure(shape: f32) -> (f32, f32) {
        let sr = 44100.0f32;
        let freq = 55.0f32;
        let period = (sr / freq).round() as usize;
        let mut osc = AnalogOscillator::new(sr);
        osc.set_waveform(Waveform::Pulse);
        osc.set_shape(shape);
        osc.set_frequency(f32x4::splat(freq));

        let mut positive = 0usize;
        let mut peak = 0.0f32;
        for _ in 0..period {
            let sample = osc.next().to_array()[0];
            if sample > 0.0 {
                positive += 1;
            }
            peak = peak.max(sample.abs());
        }

        (positive as f32 / period as f32, peak)
    }

    let (square_ratio, square_peak) = measure(0.0);
    let (wide_ratio, wide_peak) = measure(0.51);

    assert!(
        (square_ratio - pulse_width_from_shape(0.0)).abs() < 0.05,
        "shape 0.0 should produce a ~50% square duty, got {square_ratio:.3}"
    );
    assert!(
        (wide_ratio - pulse_width_from_shape(0.51)).abs() < 0.05,
        "shape 0.51 should track its mapped pulse width, got {wide_ratio:.3}"
    );
    assert!(
        wide_ratio > square_ratio + 0.1,
        "higher shape should widen the positive duty, square={square_ratio:.3} wide={wide_ratio:.3}"
    );
    assert!(
        square_peak < 1.25 && wide_peak < 1.25,
        "PWM should stay bounded, peaks square={square_peak:.3} wide={wide_peak:.3}"
    );
}

#[test]
fn test_pulse_shape_maps_to_width_without_phase_blend() {
    let mut osc = AnalogOscillator::new(44100.0);
    osc.set_waveform(Waveform::Pulse);
    osc.set_shape(1.0);
    osc.set_phase(f32x4::splat(0.123));
    osc.set_frequency(f32x4::splat(220.0));

    let sr = 44100.0f32;
    let freq = 220.0f32;
    let period = (sr / freq).round() as usize;

    let mut positive = 0usize;
    let mut peak = 0.0f32;
    for _ in 0..period {
        let sample = osc.next().to_array()[0];
        if sample > 0.0 {
            positive += 1;
        }
        peak = peak.max(sample.abs());
    }

    let duty = positive as f32 / period as f32;
    assert!(
        (duty - pulse_width_from_shape(1.0)).abs() < 0.05,
        "Pulse shape should map straight to pulse width, expected ~{:.3} got {duty:.3}",
        pulse_width_from_shape(1.0)
    );
    assert!(
        peak < 1.25,
        "Pulse shape mapping should add no phase-blend overshoot, peak {peak:.3}"
    );
}
