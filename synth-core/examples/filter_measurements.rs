//! Deterministic per-model response and self-oscillation measurements.

use synth_core::f32x4;
use synth_core::{Filter, FilterOversampling, FilterType};

const CUTOFF_HZ: f32 = 440.0;

fn main() {
    println!("filter_size_bytes={}", core::mem::size_of::<Filter>());
    for filter_type in FilterType::ALL
        .into_iter()
        .filter(|filter_type| filter_type.is_implemented())
    {
        println!("model={}", filter_type.name());
        for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            let low = sine_gain(filter_type, sample_rate, CUTOFF_HZ * 0.5, 0.0, 4);
            let high = sine_gain(filter_type, sample_rate, CUTOFF_HZ * 2.0, 0.0, 4);
            let resonant = sine_gain(filter_type, sample_rate, CUTOFF_HZ, 0.65, 4);
            let two_pole_slope = octave_slope_db(filter_type, sample_rate, 2);
            let four_pole_slope = octave_slope_db(filter_type, sample_rate, 4);
            println!(
                "  sr={sample_rate:.0} low_gain={low:.6} cutoff_peak_gain={resonant:.6} high_gain={high:.6} slope_2p={two_pole_slope:.3}dB/oct slope_4p={four_pole_slope:.3}dB/oct"
            );
        }

        let samples = self_oscillation(filter_type, 48_000.0);
        let pitch_hz = positive_crossing_pitch(&samples, 48_000.0);
        let self_osc_rms = (samples.iter().map(|sample| sample * sample).sum::<f32>()
            / samples.len() as f32)
            .sqrt();
        let peak = samples
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        let harmonics = core::array::from_fn::<_, 6, _>(|index| {
            projected_amplitude(&samples, 48_000.0, pitch_hz * (index + 1) as f32)
        });
        println!(
            "  self_osc_off pitch_hz={pitch_hz:.3} rms={self_osc_rms:.6} peak={peak:.6} harmonics={harmonics:.6?}"
        );
        for cutoff_hz in [110.0, 220.0, 410.0, 440.0, 880.0, 1760.0] {
            let samples = self_oscillation_at_cutoff(filter_type, 48_000.0, cutoff_hz);
            let pitch_hz = positive_crossing_pitch(&samples, 48_000.0);
            let harmonics = core::array::from_fn::<_, 6, _>(|index| {
                projected_amplitude(&samples, 48_000.0, pitch_hz * (index + 1) as f32)
            });
            println!(
                "  calibration cutoff={cutoff_hz:.0} pitch_hz={pitch_hz:.3} rms={:.6} harmonics={harmonics:.6?}",
                rms(&samples),
            );
        }
        let onset_rms = [0.69, 0.73, 0.8, 0.85, 0.9, 0.95, 1.0]
            .map(|resonance| self_oscillation_tail_rms(filter_type, 48_000.0, resonance));
        println!("  onset_tail_rms r=[.69,.73,.80,.85,.90,.95,1.0] values={onset_rms:.6?}");
        for oversampling in [
            FilterOversampling::Off,
            FilterOversampling::Auto,
            FilterOversampling::X2,
            FilterOversampling::X4,
        ] {
            let cutoff_gains = [0.0, 0.5, 0.65, 0.7, 0.71, 0.72, 0.8, 0.9, 1.0].map(|resonance| {
                sine_gain_with_oversampling(
                    filter_type,
                    48_000.0,
                    CUTOFF_HZ,
                    resonance,
                    4,
                    oversampling,
                )
            });
            let samples = self_oscillation_with_oversampling(filter_type, 48_000.0, oversampling);
            println!(
                "  mode={oversampling:?} cutoff_gains r=[0,.5,.65,.70,.71,.72,.80,.90,1]={cutoff_gains:.4?} self_osc_rms={:.6} pitch_hz={:.3}",
                rms(&samples),
                positive_crossing_pitch(&samples, 48_000.0),
            );
            if oversampling == FilterOversampling::Auto {
                let musical_gains = [0.72, 0.73, 0.74, 0.75, 0.76, 0.78, 0.8].map(|resonance| {
                    sine_gain_at_level(
                        filter_type,
                        48_000.0,
                        CUTOFF_HZ,
                        resonance,
                        4,
                        oversampling,
                        0.1,
                    )
                });
                println!(
                    "  auto_musical_cutoff_gains r=[.72,.73,.74,.75,.76,.78,.80] values={musical_gains:.4?}"
                );
            }
        }
    }
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

fn octave_slope_db(filter_type: FilterType, sample_rate: f32, poles: u8) -> f32 {
    let lower = sine_gain(filter_type, sample_rate, CUTOFF_HZ * 4.0, 0.0, poles);
    let upper = sine_gain(filter_type, sample_rate, CUTOFF_HZ * 8.0, 0.0, poles);
    20.0 * (lower / upper.max(1.0e-12)).log10()
}

fn configured_filter(filter_type: FilterType, resonance: f32, poles: u8) -> Filter {
    configured_filter_at_cutoff(filter_type, CUTOFF_HZ, resonance, poles)
}

fn configured_filter_at_cutoff(
    filter_type: FilterType,
    cutoff_hz: f32,
    resonance: f32,
    poles: u8,
) -> Filter {
    let mut filter = Filter::new(filter_type);
    filter.set_cutoff(cutoff_hz);
    filter.set_resonance(resonance);
    filter.set_poles(poles);
    filter.set_oversampling(FilterOversampling::Off);
    filter
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
    let mut filter = configured_filter(filter_type, resonance, poles);
    filter.set_oversampling(oversampling);
    let step = core::f32::consts::TAU * frequency / sample_rate;
    let settle = (sample_rate * 0.1) as usize;
    let measure = (sample_rate * 0.1) as usize;
    let mut phase = 0.0f32;
    for _ in 0..settle {
        let _ = process(&mut filter, phase.sin() * amplitude, sample_rate);
        phase += step;
    }
    let mut sin_sum = 0.0;
    let mut cos_sum = 0.0;
    for _ in 0..measure {
        let sin = phase.sin();
        let output = process(&mut filter, sin * amplitude, sample_rate);
        sin_sum += output * sin;
        cos_sum += output * phase.cos();
        phase += step;
    }
    2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / measure as f32 / amplitude
}

fn self_oscillation(filter_type: FilterType, sample_rate: f32) -> Vec<f32> {
    self_oscillation_with_oversampling(filter_type, sample_rate, FilterOversampling::Off)
}

fn self_oscillation_at_cutoff(
    filter_type: FilterType,
    sample_rate: f32,
    cutoff_hz: f32,
) -> Vec<f32> {
    let mut filter = configured_filter_at_cutoff(filter_type, cutoff_hz, 1.0, 4);
    let frames = (sample_rate * 2.0) as usize;
    let capture_from = frames / 2;
    let mut samples = Vec::with_capacity(frames - capture_from);
    for frame in 0..frames {
        let output = process(&mut filter, 0.0, sample_rate);
        if frame >= capture_from {
            samples.push(output);
        }
    }
    samples
}

fn self_oscillation_with_oversampling(
    filter_type: FilterType,
    sample_rate: f32,
    oversampling: FilterOversampling,
) -> Vec<f32> {
    let mut filter = configured_filter(filter_type, 1.0, 4);
    filter.set_oversampling(oversampling);
    let frames = (sample_rate * 2.0) as usize;
    let capture_from = frames / 2;
    let mut samples = Vec::with_capacity(frames - capture_from);
    for frame in 0..frames {
        let output = process(&mut filter, 0.0, sample_rate);
        if frame >= capture_from {
            samples.push(output);
        }
    }
    samples
}

fn self_oscillation_tail_rms(filter_type: FilterType, sample_rate: f32, resonance: f32) -> f32 {
    let mut filter = configured_filter(filter_type, resonance, 4);
    for _ in 0..128 {
        let _ = process(&mut filter, 0.1, sample_rate);
    }
    let frames = (sample_rate * 0.5) as usize;
    let capture_from = frames * 3 / 4;
    let mut energy = 0.0;
    for frame in 0..frames {
        let output = process(&mut filter, 0.0, sample_rate);
        if frame >= capture_from {
            energy += output * output;
        }
    }
    (energy / (frames - capture_from) as f32).sqrt()
}

fn process(filter: &mut Filter, input: f32, sample_rate: f32) -> f32 {
    filter
        .process(
            f32x4::splat(input),
            f32x4::splat(69.0),
            f32x4::splat(0.0),
            f32x4::splat(1.0),
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            sample_rate,
        )
        .to_array()[0]
}

fn positive_crossing_pitch(samples: &[f32], sample_rate: f32) -> f32 {
    let mut crossings = 0usize;
    for pair in samples.windows(2) {
        crossings += usize::from(pair[0] <= 0.0 && pair[1] > 0.0);
    }
    crossings as f32 * sample_rate / samples.len() as f32
}

fn projected_amplitude(samples: &[f32], sample_rate: f32, frequency: f32) -> f32 {
    let step = core::f32::consts::TAU * frequency / sample_rate;
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
