#!/usr/bin/env python3
"""Validate the compiled phase-invariant v2 oscillator against its fit profile."""

from __future__ import annotations

import argparse
import json
import math
import subprocess
import tempfile
from pathlib import Path

import numpy as np
from scipy.io import wavfile

from evaluate_target_conditioned_ablations import cycle_metrics


REPO_ROOT = Path(__file__).resolve().parents[4]
DEFAULT_BINARY = REPO_ROOT / "target/release/analog_osc_research"
DEFAULT_PROFILE = REPO_ROOT / (
    "plans/analog-osc/research/profiles/korg-monologue-phase-filter-v2.json"
)
DEFAULT_DERIVED = REPO_ROOT / "target/analog-osc/reference/korg-monologue-v1/derived"
DEFAULT_OUTPUT = REPO_ROOT / (
    "plans/analog-osc/research/reports/korg-monologue-phase-filter-runtime-v2.json"
)
MODEL_ID = "target-conditioned-phase-filter-v2"
SAMPLE_RATE_HZ = 48_000
WAVEFORMS = ("saw", "triangle", "square")


def render(
    binary: Path, waveform: str, frequency_hz: float, warmup: int, samples: int
) -> np.ndarray:
    cli_waveform = "pulse" if waveform == "square" else waveform
    with tempfile.TemporaryDirectory(prefix="analog-osc-runtime-v2-") as directory:
        root = Path(directory)
        subprocess.run(
            [
                str(binary),
                "--model",
                MODEL_ID,
                "--waveform",
                cli_waveform,
                "--frequency",
                repr(frequency_hz),
                "--shape",
                "0",
                "--warmup",
                str(warmup),
                "--samples",
                str(samples),
                "--output-root",
                str(root),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        paths = list((root / "renders" / MODEL_ID).glob("*.wav"))
        if len(paths) != 1:
            raise RuntimeError(f"expected one rendered WAV, found {len(paths)}")
        actual_rate, values = wavfile.read(paths[0])
        if actual_rate != SAMPLE_RATE_HZ or values.dtype != np.float32:
            raise RuntimeError(f"unexpected WAV format: {actual_rate} Hz {values.dtype}")
        return values.astype(np.float64)


def summarize(rows: list[dict]) -> dict:
    result = {}
    for waveform in WAVEFORMS:
        waveform_rows = [row for row in rows if row["waveform"] == waveform]
        result[waveform] = {}
        for split in ("validation", "test", "held_out"):
            selected = (
                [row for row in waveform_rows if row["split"] != "train"]
                if split == "held_out"
                else [row for row in waveform_rows if row["split"] == split]
            )
            runtime_shape = np.asarray(
                [row["runtime_phase_aligned_shape_nrmse"] for row in selected]
            )
            runtime_magnitude = np.asarray(
                [row["runtime_magnitude_harmonic_nrmse"] for row in selected]
            )
            baseline_shape = np.asarray(
                [row["baseline_phase_aligned_shape_nrmse"] for row in selected]
            )
            baseline_magnitude = np.asarray(
                [row["baseline_magnitude_harmonic_nrmse"] for row in selected]
            )
            result[waveform][split] = {
                "case_count": len(selected),
                "runtime_phase_aligned_shape_nrmse_median": float(
                    np.median(runtime_shape)
                ),
                "runtime_magnitude_harmonic_nrmse_median": float(
                    np.median(runtime_magnitude)
                ),
                "phase_aligned_shape_wins_vs_baseline": int(
                    np.sum(runtime_shape < baseline_shape)
                ),
                "magnitude_harmonic_wins_vs_baseline": int(
                    np.sum(runtime_magnitude < baseline_magnitude)
                ),
                "maximum_absolute_runtime_predictor_shape_delta": float(
                    np.max(
                        np.abs(
                            [
                                row["runtime_minus_predictor_shape_nrmse"]
                                for row in selected
                            ]
                        )
                    )
                ),
                "maximum_absolute_runtime_predictor_magnitude_delta": float(
                    np.max(
                        np.abs(
                            [
                                row["runtime_minus_predictor_magnitude_nrmse"]
                                for row in selected
                            ]
                        )
                    )
                ),
            }
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--profile", type=Path, default=DEFAULT_PROFILE)
    parser.add_argument("--derived-root", type=Path, default=DEFAULT_DERIVED)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    if not args.binary.is_file():
        raise FileNotFoundError(f"build the release research example first: {args.binary}")

    profile = json.loads(args.profile.read_text(encoding="utf-8"))
    if profile["model_id"] != MODEL_ID:
        raise ValueError(f"expected {MODEL_ID}, got {profile['model_id']}")

    results = []
    for waveform in WAVEFORMS:
        with np.load(args.derived_root / f"{waveform}-cycles-v1.npz") as source:
            targets = np.asarray(source["median_cycles"], dtype=np.float64)
        evaluation = profile["waveforms"][waveform]["evaluation"]
        for row in evaluation:
            if row["split"] == "train":
                continue
            frequency = float(row["frequency_hz"])
            period = int(round(SAMPLE_RATE_HZ / frequency))
            warmup = max(8_192, period * 32)
            samples = max(8_192, period * 16)
            candidate = render(args.binary, waveform, frequency, warmup, samples)
            metrics = cycle_metrics(
                targets[row["pitch_index"]], candidate, frequency, warmup
            )
            results.append(
                {
                    "waveform": waveform,
                    "pitch_index": row["pitch_index"],
                    "split": row["split"],
                    "frequency_hz": frequency,
                    "period_samples": period,
                    "warmup_samples": warmup,
                    "render_samples": samples,
                    "baseline_phase_aligned_shape_nrmse": row[
                        "baseline_phase_aligned_shape_nrmse"
                    ],
                    "predictor_phase_aligned_shape_nrmse": row[
                        "model_phase_aligned_shape_nrmse"
                    ],
                    "runtime_phase_aligned_shape_nrmse": metrics[
                        "phase_aligned_time_nrmse"
                    ],
                    "runtime_minus_predictor_shape_nrmse": metrics[
                        "phase_aligned_time_nrmse"
                    ]
                    - row["model_phase_aligned_shape_nrmse"],
                    "baseline_magnitude_harmonic_nrmse": row[
                        "baseline_magnitude_harmonic_nrmse"
                    ],
                    "predictor_magnitude_harmonic_nrmse": row[
                        "model_magnitude_harmonic_nrmse"
                    ],
                    "runtime_magnitude_harmonic_nrmse": metrics[
                        "magnitude_harmonic_nrmse"
                    ],
                    "runtime_minus_predictor_magnitude_nrmse": metrics[
                        "magnitude_harmonic_nrmse"
                    ]
                    - row["model_magnitude_harmonic_nrmse"],
                    "runtime_optimal_target_phase_shift_cycles": metrics[
                        "optimal_target_phase_shift_cycles"
                    ],
                    "runtime_candidate_to_target_ac_rms_ratio": metrics[
                        "candidate_to_target_ac_rms_ratio"
                    ],
                    "runtime_candidate_minus_target_dc": metrics[
                        "candidate_minus_target_dc"
                    ],
                }
            )
            print(
                f"{waveform:8s} {row['split']:10s} {frequency:9.3f} Hz "
                f"shape={metrics['phase_aligned_time_nrmse']:.5f} "
                f"mag={metrics['magnitude_harmonic_nrmse']:.5f}",
                flush=True,
            )

    artifact = {
        "schema_version": 2,
        "model_id": profile["model_id"],
        "profile_id": profile["profile_id"],
        "profile_content_sha256": profile["profile_content_sha256"],
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "scope": "all validation and untouched test pitches",
        "initial_phase_policy": (
            "runtime resets to phase 0.0; whole-cycle target phase is removed only "
            "inside the comparison metric"
        ),
        "comparison": (
            "averaged exact-period runtime cycles against measured median cycles; "
            "phase-aligned centered shape and harmonic magnitudes"
        ),
        "summary": summarize(results),
        "cases": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
