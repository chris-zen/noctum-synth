#!/usr/bin/env python3
"""Evaluate dynamic 48 kHz renders against filtered 192 kHz references."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
from scipy.io import wavfile
from scipy.signal import resample_poly


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_INPUT = REPO_ROOT / "target/analog-osc/dynamic-characterization-v1/runtime.json"
DEFAULT_OUTPUT = REPO_ROOT / (
    "plans/analog-osc/research/reports/"
    "korg-monologue-measured-wavetable-dynamic-v1.json"
)
BOUNDARY_SAMPLES = 2048


def load_wav(root: Path, name: str) -> tuple[int, np.ndarray]:
    sample_rate, samples = wavfile.read(root / name)
    if samples.dtype != np.float32 or samples.ndim != 1:
        raise ValueError(f"expected mono float32 WAV: {name}")
    return int(sample_rate), samples.astype(np.float64)


def high_band_dbc(samples: np.ndarray, sample_rate: int) -> float:
    window = np.hanning(samples.size)
    spectrum = np.fft.rfft(samples * window)
    power = np.square(np.abs(spectrum), dtype=np.float64)
    frequencies = np.fft.rfftfreq(samples.size, 1.0 / sample_rate)
    total = float(np.sum(power[1:]))
    high = float(np.sum(power[frequencies >= 18_000.0]))
    tiny = np.finfo(np.float64).tiny
    return float(10.0 * np.log10(max(high / max(total, tiny), tiny)))


def compare_rates(low: np.ndarray, high: np.ndarray) -> dict[str, float]:
    reference = resample_poly(high, 1, 4, window=("kaiser", 8.6))
    count = min(low.size, reference.size)
    if count <= BOUNDARY_SAMPLES * 2:
        raise ValueError("dynamic render is too short for boundary exclusion")
    selected = slice(BOUNDARY_SAMPLES, count - BOUNDARY_SAMPLES)
    low = low[selected]
    reference = reference[selected]
    error = low - reference
    reference_rms = float(np.sqrt(np.mean(np.square(reference))))
    error_rms = float(np.sqrt(np.mean(np.square(error))))
    correlation = float(np.corrcoef(low, reference)[0, 1])
    return {
        "comparison_samples": int(low.size),
        "normalized_rms_error": error_rms
        / max(reference_rms, np.finfo(np.float64).tiny),
        "maximum_absolute_error": float(np.max(np.abs(error))),
        "correlation": correlation,
        "low_48khz_high_band_dbc": high_band_dbc(low, 48_000),
        "reference_high_band_dbc": high_band_dbc(reference, 48_000),
    }


def summarize_runtime(runtime: dict) -> dict:
    indexed = {
        (case["model_id"], case["profile"]): case for case in runtime["engine_cases"]
    }
    profiles = sorted({case["profile"] for case in runtime["engine_cases"]})
    result = {}
    for profile in profiles:
        baseline = indexed[("baseline-v1", profile)]
        measured = indexed[("korg-monologue-measured-wavetable-v1", profile)]
        result[profile] = {
            "baseline_median_ns_per_frame": baseline["nanoseconds_per_frame_median"],
            "measured_median_ns_per_frame": measured["nanoseconds_per_frame_median"],
            "measured_to_baseline_median_ratio": measured[
                "nanoseconds_per_frame_median"
            ]
            / baseline["nanoseconds_per_frame_median"],
            "baseline_p99_ns_per_frame": baseline["nanoseconds_per_frame_p99"],
            "measured_p99_ns_per_frame": measured["nanoseconds_per_frame_p99"],
            "measured_p99_realtime_budget_fraction": measured[
                "realtime_budget_fraction_p99"
            ],
            "measured_peak": measured["peak"],
            "measured_finite": measured["finite"],
        }
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=DEFAULT_INPUT)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    runtime = json.loads(args.input.read_text(encoding="utf-8"))
    root = args.input.parent
    indexed = {
        (case["model_id"], case["scenario"], int(case["sample_rate_hz"])): case
        for case in runtime["source_cases"]
    }
    comparisons = []
    for model_id in ["baseline-v1", "korg-monologue-measured-wavetable-v1"]:
        for scenario in sorted({case["scenario"] for case in runtime["source_cases"]}):
            low_case = indexed[(model_id, scenario, 48_000)]
            high_case = indexed[(model_id, scenario, 192_000)]
            low_rate, low = load_wav(root, low_case["wav"])
            high_rate, high = load_wav(root, high_case["wav"])
            if low_rate != 48_000 or high_rate != 192_000:
                raise ValueError(f"unexpected rates for {model_id} {scenario}")
            comparisons.append(
                {
                    "model_id": model_id,
                    "scenario": scenario,
                    "deterministic_48khz": low_case["deterministic"],
                    "deterministic_192khz": high_case["deterministic"],
                    "peak_48khz": low_case["peak"],
                    "maximum_adjacent_step_48khz": low_case[
                        "maximum_adjacent_step"
                    ],
                    **compare_rates(low, high),
                }
            )

    measured = [
        case
        for case in comparisons
        if case["model_id"] == "korg-monologue-measured-wavetable-v1"
    ]
    artifact = {
        "schema_version": 1,
        "model_id": "korg-monologue-measured-wavetable-v1",
        "method": (
            "48 kHz dynamic renders compared with Kaiser-filtered 192 kHz renders; "
            "disagreement is an alias/implementation proxy, not a pure alias measurement"
        ),
        "boundary_samples_excluded_per_side": BOUNDARY_SAMPLES,
        "summary": {
            "all_source_cases_deterministic": all(
                case["deterministic_48khz"] and case["deterministic_192khz"]
                for case in comparisons
            ),
            "measured_maximum_peak_48khz": max(case["peak_48khz"] for case in measured),
            "measured_maximum_adjacent_step_48khz": max(
                case["maximum_adjacent_step_48khz"] for case in measured
            ),
            "measured_maximum_high_rate_nrmse": max(
                case["normalized_rms_error"] for case in measured
            ),
            "measured_minimum_high_rate_correlation": min(
                case["correlation"] for case in measured
            ),
            "note": (
                "The first run exposed and then corrected the measured PWM DC-compensation "
                "sign; neutral 50% pulse and the prior blind set were unaffected."
            ),
        },
        "sample_rate_comparisons": comparisons,
        "desktop_runtime": summarize_runtime(runtime),
        "raw_runtime_artifact": str(args.input.relative_to(REPO_ROOT)),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
