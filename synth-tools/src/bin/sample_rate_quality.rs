//! Offline spectral characterization of candidate Daisy DSP sample rates.
//!
//! This is deliberately independent of the hardware performance benchmark. It
//! measures native-rate oscillator/filter/effect output and the exact cheap
//! 24 -> 48 kHz reconstruction currently used by the fallback rate adapter.

use rustfft::{FftPlanner, num_complex::Complex32};
use synth_core::dsp::{
    AnalogOscillator, Filter, FilterOversampling, FilterType, SawMethod, WAVETABLE_BANK_SAMPLES,
    Waveform, WavetableBank, WavetableOscillator, generate_wavetable_bank,
};
use synth_core::math::WideF32;
use synth_core::{EffectModulation, EffectParams, EffectType, Effects};

const RATES: [usize; 4] = [24_000, 32_000, 44_100, 48_000];
const ANALYSIS_SECONDS: usize = 2;
const WARMUP_SAMPLES: usize = 8_192;

#[derive(Clone, Copy)]
struct SpectrumMetrics {
    alias_dbc: f64,
    alias_dbfs: f64,
    fundamental_db: f64,
    image_db: Option<f64>,
}

fn reference_wavetable_bank() -> WavetableBank {
    static BANK: std::sync::OnceLock<WavetableBank> = std::sync::OnceLock::new();
    *BANK.get_or_init(|| {
        let mut samples = vec![0.0; WAVETABLE_BANK_SAMPLES];
        generate_wavetable_bank(&mut samples).expect("generate wavetable quality bank");
        WavetableBank::new(Box::leak(samples.into_boxed_slice()))
            .expect("validate generated wavetable bank")
    })
}

fn main() {
    println!("case,rate_hz,alias_dbc,alias_dbfs,fundamental_db_vs_48k,image_12_24k_dbc");

    for (method_name, method) in [("blep", SawMethod::Blep), ("polyblep", SawMethod::PolyBlep)] {
        for (name, waveform, frequency) in [
            ("saw_997", Waveform::Saw, 997.0),
            ("triangle_997", Waveform::Triangle, 997.0),
            ("pulse_997", Waveform::Pulse, 997.0),
            ("saw_4000", Waveform::Saw, 4_000.0),
            ("triangle_4000", Waveform::Triangle, 4_000.0),
            ("pulse_4000", Waveform::Pulse, 4_000.0),
            ("saw_7001", Waveform::Saw, 7_001.0),
            ("triangle_7001", Waveform::Triangle, 7_001.0),
            ("pulse_7001", Waveform::Pulse, 7_001.0),
            ("saw_10007", Waveform::Saw, 10_007.0),
            ("triangle_10007", Waveform::Triangle, 10_007.0),
            ("pulse_10007", Waveform::Pulse, 10_007.0),
        ] {
            let case_name = format!("{method_name}_{name}");
            let signals: Vec<_> = RATES
                .into_iter()
                .map(|rate| oscillator(rate, waveform, frequency, method))
                .collect();
            report_native_group(&case_name, frequency, &signals);

            let reconstructed = reconstruct_24_to_48(&signals[0]);
            let reference_fundamental =
                analyze(&signals[3], 48_000, frequency, false).fundamental_db;
            let metrics = analyze(&reconstructed, 48_000, frequency, true);
            print_metrics(
                &format!("{case_name}_24to48_halfband15"),
                48_000,
                metrics,
                reference_fundamental,
            );

            let linear = reconstruct_24_to_48_linear(&signals[0]);
            let metrics = analyze(&linear, 48_000, frequency, true);
            print_metrics(
                &format!("{case_name}_24to48_linear_reference"),
                48_000,
                metrics,
                reference_fundamental,
            );
        }
    }

    for (name, waveform, frequency) in [
        ("saw_997", Waveform::Saw, 997.0),
        ("triangle_997", Waveform::Triangle, 997.0),
        ("pulse_997", Waveform::Pulse, 997.0),
        ("saw_4000", Waveform::Saw, 4_000.0),
        ("triangle_4000", Waveform::Triangle, 4_000.0),
        ("pulse_4000", Waveform::Pulse, 4_000.0),
        ("saw_7001", Waveform::Saw, 7_001.0),
        ("triangle_7001", Waveform::Triangle, 7_001.0),
        ("pulse_7001", Waveform::Pulse, 7_001.0),
        ("saw_10007", Waveform::Saw, 10_007.0),
        ("triangle_10007", Waveform::Triangle, 10_007.0),
        ("pulse_10007", Waveform::Pulse, 10_007.0),
    ] {
        let case_name = format!("wavetable_{name}");
        let signals: Vec<_> = RATES
            .into_iter()
            .map(|rate| wavetable_oscillator(rate, waveform, frequency))
            .collect();
        report_native_group(&case_name, frequency, &signals);
    }

    for (name, frequency, generator) in [
        (
            "filter_resonant_saw",
            997.0,
            filter_case as fn(usize, f32) -> Vec<f32>,
        ),
        ("distortion_sine", 997.0, distortion_case),
    ] {
        let signals: Vec<_> = RATES
            .into_iter()
            .map(|rate| generator(rate, frequency))
            .collect();
        report_native_group(name, frequency, &signals);
    }
}

fn wavetable_oscillator(rate: usize, waveform: Waveform, frequency_hz: f32) -> Vec<f32> {
    let mut oscillator =
        WavetableOscillator::new_wavetable(rate as f32, reference_wavetable_bank());
    oscillator.set_waveform(waveform);
    oscillator.set_shape(0.37);
    oscillator.set_frequency(WideF32::splat(frequency_hz));
    let mut ctx = synth_core::create_render_context!();
    collect(rate, || oscillator.next(&mut ctx).output.to_array()[0])
}

fn report_native_group(name: &str, fundamental_hz: f32, signals: &[Vec<f32>]) {
    let reference = analyze(&signals[3], 48_000, fundamental_hz, false).fundamental_db;
    for (rate, signal) in RATES.into_iter().zip(signals) {
        print_metrics(
            name,
            rate,
            analyze(signal, rate, fundamental_hz, false),
            reference,
        );
    }
}

fn print_metrics(name: &str, rate: usize, metrics: SpectrumMetrics, reference_db: f64) {
    let image = metrics
        .image_db
        .map(|value| format!("{value:.2}"))
        .unwrap_or_default();
    println!(
        "{name},{rate},{:.2},{:.2},{:.2},{image}",
        metrics.alias_dbc,
        metrics.alias_dbfs,
        metrics.fundamental_db - reference_db,
    );
}

fn oscillator(rate: usize, waveform: Waveform, frequency_hz: f32, method: SawMethod) -> Vec<f32> {
    let mut oscillator = AnalogOscillator::new(rate as f32);
    oscillator.set_saw_method(method);
    oscillator.set_waveform(waveform);
    oscillator.set_shape(0.37);
    oscillator.set_frequency(WideF32::splat(frequency_hz));
    let mut ctx = synth_core::create_render_context!();
    collect(rate, || oscillator.next(&mut ctx).output.to_array()[0])
}

fn filter_case(rate: usize, frequency_hz: f32) -> Vec<f32> {
    let mut oscillator = AnalogOscillator::new(rate as f32);
    oscillator.set_saw_method(SawMethod::Blep);
    oscillator.set_waveform(Waveform::Saw);
    oscillator.set_frequency(WideF32::splat(frequency_hz));

    let mut filter = Filter::new(FilterType::GainLimitedTpt);
    filter.set_cutoff(3_500.0);
    filter.set_resonance(0.82);
    filter.set_oversampling(FilterOversampling::Off);
    let mut ctx = synth_core::create_render_context!();
    collect(rate, || {
        let input = oscillator.next(&mut ctx).output * WideF32::splat(0.55);
        filter
            .process(
                input,
                WideF32::splat(60.0),
                WideF32::splat(0.0),
                WideF32::splat(1.0),
                input,
                WideF32::splat(0.0),
                WideF32::splat(0.0),
                WideF32::splat(0.0),
                rate as f32,
            )
            .to_array()[0]
    })
}

fn distortion_case(rate: usize, frequency_hz: f32) -> Vec<f32> {
    let mut effects = Effects::<16>::new(rate as f32);
    effects.set_params(EffectParams {
        enabled: true,
        effect_type: EffectType::Distortion,
        mix: 1.0,
        clock_sync: false,
        param1: 0.82,
        param2: 0.35,
    });
    let mut phase = 0.0_f32;
    let increment = frequency_hz / rate as f32;
    let mut ctx = synth_core::create_render_context!();
    collect(rate, || {
        let input = (core::f32::consts::TAU * phase).sin() * 0.45;
        phase = (phase + increment).fract();
        effects
            .next(input, input, EffectModulation::default(), None, &mut ctx)
            .0
    })
}

fn collect(rate: usize, mut next: impl FnMut() -> f32) -> Vec<f32> {
    for _ in 0..WARMUP_SAMPLES {
        let _ = next();
    }
    (0..rate * ANALYSIS_SECONDS).map(|_| next()).collect()
}

/// Reproduces the current 15-tap half-band adapter.
fn reconstruct_24_to_48(input: &[f32]) -> Vec<f32> {
    const COEFFICIENTS: [f32; 8] = [
        -0.003_332_343_2,
        0.034_400_29,
        -0.138_039_95,
        0.606_972,
        0.606_972,
        -0.138_039_95,
        0.034_400_29,
        -0.003_332_343_2,
    ];
    let mut output = Vec::with_capacity(input.len() * 2);
    let mut history = [0.0_f32; 8];
    let mut write_index = 0;
    for &current in input {
        history[write_index] = current;
        write_index = (write_index + 1) & 7;
        let filtered = COEFFICIENTS
            .into_iter()
            .enumerate()
            .map(|(age, coefficient)| history[(write_index + 7 - age) & 7] * coefficient)
            .sum();
        output.push(filtered);
        output.push(history[(write_index + 4) & 7]);
    }
    output
}

fn reconstruct_24_to_48_linear(input: &[f32]) -> Vec<f32> {
    let mut output = Vec::with_capacity(input.len() * 2);
    let mut previous = input[0];
    for &current in input {
        output.push(previous);
        output.push((previous + current) * 0.5);
        previous = current;
    }
    output
}

fn analyze(
    samples: &[f32],
    sample_rate: usize,
    fundamental_hz: f32,
    measure_images: bool,
) -> SpectrumMetrics {
    let length = samples.len();
    let mut window_sum = 0.0_f64;
    let mut input: Vec<_> = samples
        .iter()
        .enumerate()
        .map(|(index, &sample)| {
            // Four-term Blackman-Harris keeps leakage comfortably below the
            // oscillator residuals while still allowing a compact bin mask.
            let phase = core::f64::consts::TAU * index as f64 / (length - 1) as f64;
            let window = 0.35875 - 0.48829 * phase.cos() + 0.14128 * (2.0 * phase).cos()
                - 0.01168 * (3.0 * phase).cos();
            window_sum += window;
            Complex32::new(sample * window as f32, 0.0)
        })
        .collect();
    FftPlanner::<f32>::new()
        .plan_fft_forward(length)
        .process(&mut input);

    let nyquist_bin = length / 2;
    let bin_hz = sample_rate as f64 / length as f64;
    let mut harmonic_mask = vec![false; nyquist_bin + 1];
    for bin in 0..=5.min(nyquist_bin) {
        harmonic_mask[bin] = true;
    }
    let mut harmonic = fundamental_hz as f64;
    while harmonic < sample_rate as f64 * 0.5 {
        let center = (harmonic / bin_hz).round() as usize;
        for bin in center.saturating_sub(5)..=(center + 5).min(nyquist_bin) {
            harmonic_mask[bin] = true;
        }
        harmonic += fundamental_hz as f64;
    }

    let fundamental_bin = (fundamental_hz as f64 / bin_hz).round() as usize;
    let fundamental_power: f64 = input
        [fundamental_bin.saturating_sub(5)..=(fundamental_bin + 5).min(nyquist_bin)]
        .iter()
        .map(|value| value.norm_sqr() as f64)
        .sum();
    let mut alias_power = 0.0;
    let mut image_power = 0.0;
    for bin in 1..=nyquist_bin {
        let power = input[bin].norm_sqr() as f64;
        if !harmonic_mask[bin] {
            alias_power += power;
        }
        let hz = bin as f64 * bin_hz;
        if measure_images && (12_000.0..24_000.0).contains(&hz) {
            image_power += power;
        }
    }

    SpectrumMetrics {
        alias_dbc: power_db(alias_power / fundamental_power),
        // Positive-frequency FFT energy is converted back to the equivalent
        // full-scale peak convention used by the prototype quality gate.
        alias_dbfs: power_db(alias_power / (window_sum * window_sum)) + 6.020_599_913,
        fundamental_db: power_db(fundamental_power / (window_sum * window_sum)),
        image_db: measure_images.then(|| power_db(image_power / fundamental_power)),
    }
}

fn power_db(ratio: f64) -> f64 {
    10.0 * ratio.max(1.0e-30).log10()
}
