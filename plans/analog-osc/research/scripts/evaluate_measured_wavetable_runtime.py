#!/usr/bin/env python3
"""Validate the compiled measured-wavetable runtime on all held-out pitches."""

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
DEFAULT_OFFLINE = REPO_ROOT / (
    "plans/analog-osc/research/reports/korg-monologue-measured-wavetable-v1.json"
)
DEFAULT_BANK_MANIFEST = REPO_ROOT / (
    "plans/analog-osc/research/banks/korg-monologue-measured-bank-v1.json"
)
DEFAULT_DERIVED = REPO_ROOT / "target/analog-osc/reference/korg-monologue-v1/derived"
DEFAULT_OUTPUT = REPO_ROOT / (
    "plans/analog-osc/research/reports/korg-monologue-measured-wavetable-runtime-v1.json"
)
MODEL_ID = "korg-monologue-measured-wavetable-v1"
SAMPLE_RATE_HZ = 48_000
WAVEFORMS = ("saw", "triangle", "square")
OFFLINE_VARIANT = {
    "saw": "canonical_complex_interpolation",
    "triangle": "complex_interpolation",
    "square": "complex_interpolation",
}


def render(
    binary: Path, waveform: str, frequency_hz: float, warmup: int, samples: int
) -> np.ndarray:
    cli_waveform = "pulse" if waveform == "square" else waveform
    with tempfile.TemporaryDirectory(prefix="measured-wavetable-runtime-") as directory:
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
                waveform_rows
                if split == "held_out"
                else [row for row in waveform_rows if row["split"] == split]
            )
            shape = np.asarray(
                [row["runtime_phase_aligned_shape_nrmse"] for row in selected]
            )
            magnitude = np.asarray(
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
                "runtime_phase_aligned_shape_nrmse_median": float(np.median(shape)),
                "runtime_phase_aligned_shape_nrmse_maximum": float(np.max(shape)),
                "runtime_shape_wins_vs_baseline": int(np.sum(shape < baseline_shape)),
                "runtime_magnitude_harmonic_nrmse_median": float(np.median(magnitude)),
                "runtime_magnitude_harmonic_nrmse_maximum": float(np.max(magnitude)),
                "runtime_magnitude_wins_vs_baseline": int(
                    np.sum(magnitude < baseline_magnitude)
                ),
                "maximum_absolute_runtime_offline_shape_delta": float(
                    np.max(
                        np.abs(
                            [row["runtime_minus_offline_shape_nrmse"] for row in selected]
                        )
                    )
                ),
                "maximum_absolute_runtime_offline_magnitude_delta": float(
                    np.max(
                        np.abs(
                            [
                                row["runtime_minus_offline_magnitude_nrmse"]
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
    parser.add_argument("--offline-report", type=Path, default=DEFAULT_OFFLINE)
    parser.add_argument("--bank-manifest", type=Path, default=DEFAULT_BANK_MANIFEST)
    parser.add_argument("--derived-root", type=Path, default=DEFAULT_DERIVED)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    if not args.binary.is_file():
        raise FileNotFoundError(f"build the release research example first: {args.binary}")

    offline = json.loads(args.offline_report.read_text(encoding="utf-8"))
    bank_manifest = json.loads(args.bank_manifest.read_text(encoding="utf-8"))
    results = []
    for waveform in WAVEFORMS:
        with np.load(args.derived_root / f"{waveform}-cycles-v1.npz") as source:
            targets = np.asarray(source["median_cycles"], dtype=np.float64)
        for row in offline["waveforms"][waveform]["cases"]:
            frequency = float(row["frequency_hz"])
            period = int(round(SAMPLE_RATE_HZ / frequency))
            warmup = max(8_192, period * 32)
            samples = max(8_192, period * 16)
            candidate = render(args.binary, waveform, frequency, warmup, samples)
            metrics = cycle_metrics(
                targets[row["pitch_index"]], candidate, frequency, warmup
            )
            variant = OFFLINE_VARIANT[waveform]
            offline_metrics = row["variants"][variant]
            results.append(
                {
                    "waveform": waveform,
                    "pitch_index": row["pitch_index"],
                    "split": row["split"],
                    "frequency_hz": frequency,
                    "endpoint_extrapolation": row["endpoint_extrapolation"],
                    "offline_variant": variant,
                    "baseline_phase_aligned_shape_nrmse": row["baseline"][
                        "phase_aligned_shape_nrmse"
                    ],
                    "baseline_magnitude_harmonic_nrmse": row["baseline"][
                        "magnitude_harmonic_nrmse"
                    ],
                    "offline_phase_aligned_shape_nrmse": offline_metrics[
                        "phase_aligned_shape_nrmse"
                    ],
                    "runtime_phase_aligned_shape_nrmse": metrics[
                        "phase_aligned_time_nrmse"
                    ],
                    "runtime_minus_offline_shape_nrmse": metrics[
                        "phase_aligned_time_nrmse"
                    ]
                    - offline_metrics["phase_aligned_shape_nrmse"],
                    "offline_magnitude_harmonic_nrmse": offline_metrics[
                        "magnitude_harmonic_nrmse"
                    ],
                    "runtime_magnitude_harmonic_nrmse": metrics[
                        "magnitude_harmonic_nrmse"
                    ],
                    "runtime_minus_offline_magnitude_nrmse": metrics[
                        "magnitude_harmonic_nrmse"
                    ]
                    - offline_metrics["magnitude_harmonic_nrmse"],
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

    artifact = {
        "schema_version": 1,
        "model_id": MODEL_ID,
        "profile_id": bank_manifest["profile_id"],
        "profile_content_sha256": bank_manifest["manifest_content_sha256"],
        "bank_binary_sha256": bank_manifest["bank_binary"]["sha256"],
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "scope": "all validation and test pitches",
        "comparison": (
            "compiled native-rate mip/pitch interpolation against measured median cycles"
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
