//! Numerical parity against the WIP Python extractor (`analog_osc_reference.py`).
//!
//! Fixtures under `tests/fixtures/extraction/` were generated once from that
//! algorithm. Tolerances:
//! - frequency: 0.05 cents (or 1e-8 relative for exact-ish cases)
//! - median cycle stride samples: 2e-5 abs
//! - harmonic head: 5e-5 abs (1e-3 on noisy cases)
//! - scalar metrics: 1e-5 relative / abs as noted per field

use std::{fs, path::PathBuf};

use serde::Deserialize;
use synth_capture::extraction::{estimate_frequency, extract_pitch};

#[derive(Deserialize)]
struct FixtureFile {
    cases: std::collections::BTreeMap<String, FixtureCase>,
}

#[derive(Deserialize)]
struct FixtureCase {
    sample_rate_hz: f64,
    search_frequency_hz: f64,
    samples_file: String,
    sample_count: usize,
    python: Option<PythonPitch>,
    python_frequency_hz: Option<f64>,
}

#[derive(Deserialize)]
struct PythonPitch {
    frequency_hz: f64,
    dc: f64,
    rms: f64,
    peak: f64,
    crest_factor: f64,
    duty_above_midpoint: f64,
    period_jitter_ppm: f64,
    cycle_amplitude_cv: f64,
    cycles_used: usize,
    cycles_rejected: usize,
    median_cycle_stride64: Vec<f32>,
    median_cycle_len: usize,
    harmonics_re_head8: Vec<f64>,
    harmonics_im_head8: Vec<f64>,
    harmonics_len: usize,
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extraction")
}

fn load_samples(case: &FixtureCase) -> Vec<f32> {
    let bytes = fs::read(fixture_dir().join(&case.samples_file)).unwrap();
    assert_eq!(bytes.len(), case.sample_count * 4);
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn cents(a: f64, b: f64) -> f64 {
    1200.0 * (a / b).log2()
}

fn assert_close(label: &str, got: f64, expected: f64, abs_tol: f64, rel_tol: f64) {
    let diff = (got - expected).abs();
    let limit = abs_tol.max(rel_tol * expected.abs());
    assert!(
        diff <= limit,
        "{label}: got={got} expected={expected} diff={diff} limit={limit}"
    );
}

#[test]
fn frequency_estimate_matches_python() {
    let meta: FixtureFile = serde_json::from_str(
        &fs::read_to_string(fixture_dir().join("python_parity_v1.json")).unwrap(),
    )
    .unwrap();
    let case = &meta.cases["freq_only"];
    let samples = load_samples(case);
    let got = estimate_frequency(
        &samples,
        case.sample_rate_hz,
        Some(case.search_frequency_hz),
    )
    .unwrap();
    let expected = case.python_frequency_hz.unwrap();
    assert!(
        cents(got, expected).abs() < 0.05,
        "freq cents={} got={} expected={}",
        cents(got, expected),
        got,
        expected
    );
}

#[test]
fn pitch_extraction_matches_python_fixtures() {
    let meta: FixtureFile = serde_json::from_str(
        &fs::read_to_string(fixture_dir().join("python_parity_v1.json")).unwrap(),
    )
    .unwrap();

    for (name, case) in &meta.cases {
        let Some(python) = &case.python else {
            continue;
        };
        let samples = load_samples(case);
        let harmonic_tol = if name == "sine_noise" { 1e-3 } else { 5e-5 };
        let cycle_tol = if name == "sine_noise" { 5e-4 } else { 2e-5 };
        let result = extract_pitch(
            &samples,
            case.sample_rate_hz,
            2048,
            256,
            1024,
            Some(case.search_frequency_hz),
        )
        .unwrap_or_else(|err| panic!("{name}: {err}"));

        assert!(
            cents(result.frequency_hz, python.frequency_hz).abs() < 0.05,
            "{name} frequency cents={}",
            cents(result.frequency_hz, python.frequency_hz)
        );
        assert_close(&format!("{name} dc"), result.raw_dc, python.dc, 1e-6, 1e-5);
        assert_close(
            &format!("{name} rms"),
            result.raw_rms,
            python.rms,
            1e-6,
            1e-5,
        );
        assert_close(
            &format!("{name} peak"),
            result.raw_peak,
            python.peak,
            1e-6,
            1e-5,
        );
        assert_close(
            &format!("{name} crest"),
            result.crest_factor,
            python.crest_factor,
            1e-5,
            1e-5,
        );
        assert_close(
            &format!("{name} duty"),
            result.duty_above_midpoint,
            python.duty_above_midpoint,
            1e-6,
            1e-5,
        );
        assert_close(
            &format!("{name} jitter"),
            result.period_jitter_ppm,
            python.period_jitter_ppm,
            1e-3,
            1e-4,
        );
        assert_close(
            &format!("{name} amp_cv"),
            result.cycle_amplitude_cv,
            python.cycle_amplitude_cv,
            1e-5,
            1e-4,
        );
        assert_eq!(result.cycles_accepted, python.cycles_used, "{name} cycles");
        assert_eq!(
            result.cycles_rejected, python.cycles_rejected,
            "{name} rejected"
        );
        assert_eq!(result.median_cycle_raw.len(), python.median_cycle_len);
        for (index, expected) in python.median_cycle_stride64.iter().enumerate() {
            let got = result.median_cycle_raw[index * 64];
            assert!(
                (got - *expected).abs() <= cycle_tol,
                "{name} cycle[{}]: got={got} expected={expected}",
                index * 64
            );
        }
        assert_eq!(result.complex_harmonics.len(), python.harmonics_len);
        for index in 0..8 {
            assert!(
                (f64::from(result.complex_harmonics[index].re) - python.harmonics_re_head8[index])
                    .abs()
                    <= harmonic_tol,
                "{name} harm_re[{index}]"
            );
            assert!(
                (f64::from(result.complex_harmonics[index].im) - python.harmonics_im_head8[index])
                    .abs()
                    <= harmonic_tol,
                "{name} harm_im[{index}]"
            );
        }
    }
}
