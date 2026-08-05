#!/usr/bin/env python3
"""Compare the compiled target-conditioned oscillator with held-out target cycles."""

from __future__ import annotations

import argparse
import json
import math
import subprocess
import tempfile
from pathlib import Path

import numpy as np
from scipy.io import wavfile


REPO_ROOT = Path(__file__).resolve().parents[4]
DEFAULT_BINARY = REPO_ROOT / "target/release/analog_osc_research"
DEFAULT_PROFILE = REPO_ROOT / "plans/analog-osc/research/profiles/korg-monologue-phase-filter-v1.json"
DEFAULT_DERIVED = REPO_ROOT / "target/analog-osc/reference/korg-monologue-v1/derived"
DEFAULT_OUTPUT = REPO_ROOT / "plans/analog-osc/research/reports/korg-monologue-phase-filter-runtime-v1.json"
SAMPLE_RATE_HZ = 48_000.0


def periodic_interpolate(cycle: np.ndarray, phase: np.ndarray) -> np.ndarray:
    bins = cycle.size
    return np.interp(
        np.mod(phase, 1.0),
        np.arange(bins + 1, dtype=np.float64) / bins,
        np.concatenate((cycle, cycle[:1])),
    )


def normalized_time_error(reference: np.ndarray, candidate: np.ndarray) -> float:
    rms = np.sqrt(np.mean((reference - np.mean(reference)) ** 2))
    return float(np.sqrt(np.mean((candidate - reference) ** 2)) / max(rms, np.finfo(float).tiny))


def checkpoint_rows(waveform_profile: dict, all_held_out: bool) -> list[dict]:
    held_out = [row for row in waveform_profile["evaluation"] if row["split"] != "train"]
    if all_held_out or len(held_out) <= 3:
        return held_out
    return [held_out[0], held_out[len(held_out) // 2], held_out[-1]]


def render(binary: Path, waveform: str, frequency_hz: float, warmup: int, samples: int) -> np.ndarray:
    cli_waveform = "pulse" if waveform == "square" else waveform
    with tempfile.TemporaryDirectory(prefix="analog-osc-runtime-") as directory:
        root = Path(directory)
        subprocess.run(
            [
                str(binary),
                "--model",
                "target-conditioned-phase-filter-v1",
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
        paths = list((root / "renders/target-conditioned-phase-filter-v1").glob("*.wav"))
        if len(paths) != 1:
            raise RuntimeError(f"expected one rendered WAV, found {len(paths)}")
        sample_rate, values = wavfile.read(paths[0])
        if sample_rate != int(SAMPLE_RATE_HZ) or values.dtype != np.float32:
            raise RuntimeError(f"unexpected WAV format: {sample_rate} Hz {values.dtype}")
        return values.astype(np.float64)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--profile", type=Path, default=DEFAULT_PROFILE)
    parser.add_argument("--derived-root", type=Path, default=DEFAULT_DERIVED)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--all-held-out", action="store_true")
    args = parser.parse_args()
    if not args.binary.is_file():
        raise FileNotFoundError(f"build the release research example first: {args.binary}")

    profile = json.loads(args.profile.read_text(encoding="utf-8"))
    results = []
    for waveform in ("saw", "triangle", "square"):
        with np.load(args.derived_root / f"{waveform}-cycles-v1.npz") as source:
            targets = np.asarray(source["median_cycles"], dtype=np.float64)
        for row in checkpoint_rows(profile["waveforms"][waveform], args.all_held_out):
            frequency = float(row["frequency_hz"])
            warmup = max(8_192, math.ceil(SAMPLE_RATE_HZ / frequency) * 32)
            sample_count = max(8_192, math.ceil(SAMPLE_RATE_HZ / frequency) * 16)
            candidate = render(args.binary, waveform, frequency, warmup, sample_count)
            phase = (warmup + np.arange(sample_count)) * frequency / SAMPLE_RATE_HZ
            target = periodic_interpolate(targets[row["pitch_index"]], phase)
            runtime_error = normalized_time_error(target, candidate)
            correlation = float(np.corrcoef(target, candidate)[0, 1])
            results.append(
                {
                    "waveform": waveform,
                    "pitch_index": row["pitch_index"],
                    "split": row["split"],
                    "frequency_hz": frequency,
                    "warmup_samples": warmup,
                    "render_samples": sample_count,
                    "predictor_time_nrmse": row["model_time_nrmse"],
                    "runtime_time_nrmse": runtime_error,
                    "runtime_minus_predictor_nrmse": runtime_error - row["model_time_nrmse"],
                    "runtime_target_correlation": correlation,
                }
            )

    summary = {}
    for waveform in ("saw", "triangle", "square"):
        rows = [row for row in results if row["waveform"] == waveform]
        summary[waveform] = {
            "checkpoint_count": len(rows),
            "median_runtime_time_nrmse": float(np.median([row["runtime_time_nrmse"] for row in rows])),
            "maximum_runtime_time_nrmse": float(np.max([row["runtime_time_nrmse"] for row in rows])),
            "maximum_absolute_runtime_predictor_delta": float(
                np.max([abs(row["runtime_minus_predictor_nrmse"]) for row in rows])
            ),
            "minimum_runtime_target_correlation": float(
                np.min([row["runtime_target_correlation"] for row in rows])
            ),
        }
    artifact = {
        "schema_version": 1,
        "model_id": profile["model_id"],
        "profile_id": profile["profile_id"],
        "profile_content_sha256": profile["profile_content_sha256"],
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "scope": "all held-out pitches" if args.all_held_out else "low/mid/high held-out checkpoints",
        "alignment": "target cycle sampled at the runtime oscillator base phase; no fitted delay",
        "summary": summary,
        "cases": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
