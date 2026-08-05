#!/usr/bin/env python3
"""Evaluate training-only measured wavetable representations for Plan 07."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from pathlib import Path
from typing import Any

import numpy as np


REPO_ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from scripts.evaluate_target_conditioned_ablations import (  # noqa: E402
    best_phase_alignment,
    nrmse,
)
from scripts.fit_target_conditioned_oscillator import (  # noqa: E402
    DEFAULT_DERIVED,
    PHASE_BINS,
    SAMPLE_RATE_HZ,
)
from scripts.fit_target_conditioned_oscillator_v2 import (  # noqa: E402
    production_source,
    target_metrics,
)


DEFAULT_OUTPUT = REPO_ROOT / (
    "plans/analog-osc/research/reports/korg-monologue-measured-wavetable-v1.json"
)
WAVEFORMS = ("saw", "triangle", "square")
VARIANTS = (
    "nearest_table",
    "complex_interpolation",
    "baseline_plus_residual",
    "canonical_complex_interpolation",
    "canonical_baseline_plus_residual",
)
NYQUIST_GUARD = 0.45


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def split_for_index(index: int) -> str:
    if index % 4 == 3:
        return "test"
    if index % 4 == 1:
        return "validation"
    return "train"


def legal_harmonic_mask(size: int, frequency_hz: float) -> np.ndarray:
    harmonics = np.arange(size, dtype=np.float64)
    legal = harmonics * frequency_hz < SAMPLE_RATE_HZ * NYQUIST_GUARD
    legal[0] = True
    return legal


def bandlimit_spectrum(spectrum: np.ndarray, frequency_hz: float) -> np.ndarray:
    result = np.asarray(spectrum, dtype=np.complex128).copy()
    result[~legal_harmonic_mask(result.size, frequency_hz)] = 0.0
    if result.size > 1 and PHASE_BINS % 2 == 0:
        result[-1] = complex(result[-1].real, 0.0)
    return result


def interpolate_complex(
    frequency_hz: float,
    knot_frequencies_hz: np.ndarray,
    knot_spectra: np.ndarray,
) -> np.ndarray:
    coordinate = math.log2(frequency_hz)
    knots = np.log2(knot_frequencies_hz)
    if coordinate <= knots[0]:
        return knot_spectra[0].copy()
    if coordinate >= knots[-1]:
        return knot_spectra[-1].copy()
    upper = int(np.searchsorted(knots, coordinate, side="left"))
    lower = upper - 1
    amount = (coordinate - knots[lower]) / (knots[upper] - knots[lower])
    return knot_spectra[lower] + (knot_spectra[upper] - knot_spectra[lower]) * amount


def reconstruct(spectrum: np.ndarray) -> np.ndarray:
    return np.fft.irfft(spectrum, n=PHASE_BINS)


def render_variants(
    waveform: str,
    frequency_hz: float,
    knot_frequencies_hz: np.ndarray,
    knot_spectra: np.ndarray,
    knot_residual_spectra: np.ndarray,
    canonical_knot_spectra: np.ndarray,
    canonical_knot_residual_spectra: np.ndarray,
) -> dict[str, np.ndarray]:
    log_distance = np.abs(np.log2(knot_frequencies_hz) - math.log2(frequency_hz))
    nearest = knot_spectra[int(np.argmin(log_distance))]
    interpolated = interpolate_complex(frequency_hz, knot_frequencies_hz, knot_spectra)
    residual = interpolate_complex(
        frequency_hz, knot_frequencies_hz, knot_residual_spectra
    )
    canonical = interpolate_complex(
        frequency_hz, knot_frequencies_hz, canonical_knot_spectra
    )
    canonical_residual = interpolate_complex(
        frequency_hz, knot_frequencies_hz, canonical_knot_residual_spectra
    )
    baseline = production_source(waveform, frequency_hz)
    baseline_spectrum = np.fft.rfft(baseline)
    spectra = {
        "nearest_table": nearest,
        "complex_interpolation": interpolated,
        "baseline_plus_residual": baseline_spectrum + residual,
        "canonical_complex_interpolation": canonical,
        "canonical_baseline_plus_residual": baseline_spectrum + canonical_residual,
    }
    return {
        name: reconstruct(bandlimit_spectrum(spectrum, frequency_hz))
        for name, spectrum in spectra.items()
    }


def evaluate_waveform(derived_root: Path, waveform: str) -> dict[str, Any]:
    npz_path = derived_root / f"{waveform}-cycles-v1.npz"
    with np.load(npz_path) as source:
        cycles = np.asarray(source["median_cycles"], dtype=np.float64)
        frequencies = np.asarray(source["measured_frequency_hz"], dtype=np.float64)
    spectra = np.fft.rfft(cycles, axis=1)
    train_indices = np.asarray(
        [index for index in range(cycles.shape[0]) if split_for_index(index) == "train"]
    )
    knot_frequencies = frequencies[train_indices]
    knot_spectra = spectra[train_indices]
    baseline_knots = np.stack(
        [
            np.fft.rfft(production_source(waveform, float(frequency)))
            for frequency in knot_frequencies
        ]
    )
    knot_residuals = knot_spectra - baseline_knots
    canonical_cycles = []
    canonical_shifts = []
    for index, frequency in zip(train_indices, knot_frequencies):
        baseline = production_source(waveform, float(frequency))
        shift, _, aligned = best_phase_alignment(cycles[index], baseline, 0.0)
        canonical_shifts.append(shift)
        canonical_cycles.append(aligned)
    canonical_knot_spectra = np.fft.rfft(np.stack(canonical_cycles), axis=1)
    canonical_knot_residuals = canonical_knot_spectra - baseline_knots

    cases = []
    for index, (target, frequency_hz) in enumerate(zip(cycles, frequencies)):
        if split_for_index(index) == "train":
            continue
        frequency = float(frequency_hz)
        baseline = production_source(waveform, frequency)
        baseline_metrics = target_metrics(target, baseline, frequency)
        candidates = render_variants(
            waveform,
            frequency,
            knot_frequencies,
            knot_spectra,
            knot_residuals,
            canonical_knot_spectra,
            canonical_knot_residuals,
        )
        variant_metrics = {}
        for name, candidate in candidates.items():
            metrics = target_metrics(target, candidate, frequency)
            variant_metrics[name] = {
                "native_time_nrmse": nrmse(target, candidate),
                "fixed_phase_shape_nrmse": metrics["fixed_phase_shape_nrmse"],
                "phase_aligned_shape_nrmse": metrics[
                    "phase_aligned_shape_nrmse"
                ],
                "magnitude_harmonic_nrmse": metrics[
                    "magnitude_harmonic_nrmse"
                ],
                "optimal_target_phase_shift_cycles": metrics[
                    "optimal_target_phase_shift_cycles"
                ],
                "dc_error": float(np.mean(candidate) - np.mean(target)),
            }
        cases.append(
            {
                "pitch_index": index,
                "split": split_for_index(index),
                "frequency_hz": frequency,
                "endpoint_extrapolation": bool(
                    frequency < knot_frequencies[0] or frequency > knot_frequencies[-1]
                ),
                "baseline": {
                    "phase_aligned_shape_nrmse": baseline_metrics[
                        "phase_aligned_shape_nrmse"
                    ],
                    "magnitude_harmonic_nrmse": baseline_metrics[
                        "magnitude_harmonic_nrmse"
                    ],
                },
                "variants": variant_metrics,
            }
        )
    return {
        "source_npz_sha256": sha256_file(npz_path),
        "training_pitch_indices": train_indices.tolist(),
        "training_knot_count": int(train_indices.size),
        "canonical_training_shift_cycles": canonical_shifts,
        "cases": cases,
        "summary": summarize(cases),
    }


def summarize(cases: list[dict[str, Any]]) -> dict[str, Any]:
    result = {}
    for split in ("validation", "test", "held_out"):
        selected = (
            cases
            if split == "held_out"
            else [row for row in cases if row["split"] == split]
        )
        split_result = {}
        baseline_shape = np.asarray(
            [row["baseline"]["phase_aligned_shape_nrmse"] for row in selected]
        )
        baseline_magnitude = np.asarray(
            [row["baseline"]["magnitude_harmonic_nrmse"] for row in selected]
        )
        split_result["baseline"] = {
            "phase_aligned_shape_nrmse_median": float(np.median(baseline_shape)),
            "magnitude_harmonic_nrmse_median": float(np.median(baseline_magnitude)),
        }
        for variant in VARIANTS:
            shape = np.asarray(
                [
                    row["variants"][variant]["phase_aligned_shape_nrmse"]
                    for row in selected
                ]
            )
            magnitude = np.asarray(
                [
                    row["variants"][variant]["magnitude_harmonic_nrmse"]
                    for row in selected
                ]
            )
            fixed = np.asarray(
                [row["variants"][variant]["fixed_phase_shape_nrmse"] for row in selected]
            )
            split_result[variant] = {
                "phase_aligned_shape_nrmse_median": float(np.median(shape)),
                "phase_aligned_shape_nrmse_maximum": float(np.max(shape)),
                "phase_aligned_shape_wins_vs_baseline": int(
                    np.sum(shape < baseline_shape)
                ),
                "magnitude_harmonic_nrmse_median": float(np.median(magnitude)),
                "magnitude_harmonic_nrmse_maximum": float(np.max(magnitude)),
                "magnitude_harmonic_wins_vs_baseline": int(
                    np.sum(magnitude < baseline_magnitude)
                ),
                "fixed_phase_shape_nrmse_median": float(np.median(fixed)),
            }
        result[split] = split_result
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--derived-root", type=Path, default=DEFAULT_DERIVED)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()

    waveforms = {
        waveform: evaluate_waveform(args.derived_root, waveform)
        for waveform in WAVEFORMS
    }
    artifact = {
        "schema_version": 1,
        "experiment_id": "korg-monologue-measured-wavetable-v1",
        "target_id": "korg-monologue-v1",
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "phase_bins": PHASE_BINS,
        "nyquist_guard": NYQUIST_GUARD,
        "fit_policy": {
            "training_split": "pitch index modulo 2 is zero",
            "interpolation_coordinate": "log2 measured frequency",
            "phase_policy": (
                "preserve the importer upward-crossing landmark and interpolate "
                "complex spectra without per-case target alignment"
            ),
            "runtime_bandlimit": (
                "zero harmonics at or above 0.45 * sample rate before reconstruction"
            ),
            "endpoint_policy": "hold the nearest training spectrum",
        },
        "variants": {
            "nearest_table": "nearest training-pitch measured complex spectrum",
            "complex_interpolation": (
                "linear interpolation of complex harmonics in log2 frequency"
            ),
            "baseline_plus_residual": (
                "production oscillator plus interpolated measured-minus-production "
                "complex spectrum"
            ),
            "canonical_complex_interpolation": (
                "complex interpolation after removing one training-cycle global "
                "rotation relative to the production source"
            ),
            "canonical_baseline_plus_residual": (
                "production oscillator plus canonicalized interpolated complex residual"
            ),
        },
        "waveforms": waveforms,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
