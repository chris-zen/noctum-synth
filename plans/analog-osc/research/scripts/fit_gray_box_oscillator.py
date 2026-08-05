#!/usr/bin/env python3
"""Fit and qualify the bounded Plan 12 saw-core against Monologue cycles."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from pathlib import Path
from typing import Any

import numpy as np
from scipy.optimize import least_squares

REPO_ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from scripts.evaluate_target_conditioned_ablations import (
    best_phase_alignment,
    centered_unit_rms,
)
from scripts.fit_target_conditioned_oscillator import DEFAULT_DERIVED, load_dataset
from scripts.fit_target_conditioned_oscillator_v2 import production_source


DEFAULT_OUTPUT = REPO_ROOT / (
    "plans/analog-osc/research/profiles/korg-monologue-gray-box-saw-core-v1.json"
)
SAMPLE_RATE_HZ = 48_000.0
TRAIN_INDEX = 0
HELD_OUT_INDICES = (5, 11, 17, 23, 35, 47, 59, 71)
WAVEFORMS = ("saw", "triangle", "square")


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    digest.update(path.read_bytes())
    return digest.hexdigest()


def capacitor_cycle(curvature: float, reset_cycles: float, bins: int) -> np.ndarray:
    phase = np.arange(bins, dtype=np.float64) / bins
    charge_cycles = 1.0 - reset_cycles
    charging = phase < charge_cycles
    result = np.empty(bins, dtype=np.float64)
    charge_phase = phase[charging]
    if abs(curvature) < 1.0e-7:
        result[charging] = charge_phase / charge_cycles
    else:
        travel = math.log((1.0 + curvature) / (1.0 - curvature)) / (2.0 * curvature)
        scale = travel / charge_cycles
        a = 1.0 - curvature
        b = 2.0 * curvature
        result[charging] = ((a * np.exp(scale * b * charge_phase)) - a) / b
    if reset_cycles > 1.0e-9:
        result[~charging] = 1.0 - (phase[~charging] - charge_cycles) / reset_cycles
    else:
        result[~charging] = 0.0
    return np.clip(result, 0.0, 1.0)


def one_pole_cycle(source: np.ndarray, frequency_hz: float, cutoff_hz: float) -> np.ndarray:
    harmonics = np.arange(source.size // 2 + 1, dtype=np.float64)
    omega = 2.0 * np.pi * harmonics * frequency_hz / SAMPLE_RATE_HZ
    inverse_z = np.exp(-1j * omega)
    pole = math.exp(-2.0 * math.pi * cutoff_hz / SAMPLE_RATE_HZ)
    response = (1.0 - pole) / (1.0 - pole * inverse_z)
    return np.fft.irfft(np.fft.rfft(source) * response, n=source.size)


def periodic_shift(cycle: np.ndarray, shift_cycles: float) -> np.ndarray:
    harmonics = np.fft.rfft(cycle)
    indices = np.arange(harmonics.size, dtype=np.float64)
    return np.fft.irfft(harmonics * np.exp(2j * np.pi * indices * shift_cycles), n=cycle.size)


def source_cycle(waveform: str, capacitor: np.ndarray) -> np.ndarray:
    if waveform == "saw":
        return 2.0 * capacitor - 1.0
    if waveform == "triangle":
        return 1.0 - 4.0 * np.abs(capacitor - 0.5)
    return np.where(capacitor < 0.5, 1.0, -1.0)


def candidate_cycle(
    waveform: str,
    frequency_hz: float,
    parameters: np.ndarray,
    bins: int,
) -> np.ndarray:
    curvature, reset = parameters[:2]
    waveform_index = WAVEFORMS.index(waveform)
    cutoff = math.exp(parameters[2 + waveform_index])
    shift = parameters[5 + waveform_index]
    capacitor = capacitor_cycle(curvature, reset, bins)
    source = source_cycle(waveform, capacitor)
    return periodic_shift(one_pole_cycle(source, frequency_hz, cutoff), shift)


def optimal_gain_dc(target: np.ndarray, candidate: np.ndarray) -> tuple[float, float]:
    target_centered = target - np.mean(target)
    candidate_centered = candidate - np.mean(candidate)
    gain = float(np.dot(target_centered, candidate_centered) / np.dot(candidate_centered, candidate_centered))
    return gain, float(np.mean(target) - gain * np.mean(candidate))


def fit(datasets: dict[str, Any]) -> Any:
    initial = np.asarray(
        [-0.08, 0.002, math.log(18_000.0), math.log(15_000.0), math.log(18_000.0), 0.0, 0.0, 0.0]
    )
    lower = np.asarray(
        [-0.85, 0.0, math.log(800.0), math.log(800.0), math.log(800.0), -0.25, -0.25, -0.25]
    )
    upper = np.asarray(
        [0.85, 0.08, math.log(60_000.0), math.log(60_000.0), math.log(60_000.0), 0.25, 0.25, 0.25]
    )

    def residual(parameters: np.ndarray) -> np.ndarray:
        values = []
        for waveform in WAVEFORMS:
            dataset = datasets[waveform]
            target = dataset.cycles[TRAIN_INDEX]
            candidate = candidate_cycle(
                waveform, dataset.frequencies_hz[TRAIN_INDEX], parameters, target.size
            )
            values.append((centered_unit_rms(candidate) - centered_unit_rms(target)) / math.sqrt(target.size))
        return np.concatenate(values)

    return least_squares(
        residual,
        initial,
        bounds=(lower, upper),
        max_nfev=800,
        xtol=1.0e-10,
        ftol=1.0e-10,
        gtol=1.0e-10,
    )


def metrics(target: np.ndarray, candidate: np.ndarray) -> dict[str, float]:
    shift, error, aligned_target = best_phase_alignment(target, candidate, 0.0)
    correlation = np.corrcoef(centered_unit_rms(aligned_target), centered_unit_rms(candidate))[0, 1]
    return {
        "phase_aligned_shape_nrmse": float(error),
        "correlation": float(correlation),
        "optimal_target_shift_cycles": float(shift),
    }


def summarize(datasets: dict[str, Any], parameters: np.ndarray) -> dict[str, Any]:
    cases = []
    outputs = {}
    for waveform_index, waveform in enumerate(WAVEFORMS):
        dataset = datasets[waveform]
        train_target = dataset.cycles[TRAIN_INDEX]
        train_candidate = candidate_cycle(
            waveform, dataset.frequencies_hz[TRAIN_INDEX], parameters, train_target.size
        )
        gain, dc = optimal_gain_dc(train_target, train_candidate)
        outputs[waveform] = {
            "lowpass_hz": float(math.exp(parameters[2 + waveform_index])),
            "gain": gain,
            "dc": dc,
        }
        for index in HELD_OUT_INDICES:
            target = dataset.cycles[index]
            candidate = candidate_cycle(waveform, dataset.frequencies_hz[index], parameters, target.size)
            baseline = production_source(waveform, dataset.frequencies_hz[index], target.size)
            cases.append(
                {
                    "waveform": waveform,
                    "index": index,
                    "frequency_hz": float(dataset.frequencies_hz[index]),
                    "gray_box": metrics(target, candidate),
                    "baseline": metrics(target, baseline),
                }
            )
    gray = np.asarray([case["gray_box"]["phase_aligned_shape_nrmse"] for case in cases])
    baseline = np.asarray([case["baseline"]["phase_aligned_shape_nrmse"] for case in cases])
    return {
        "outputs": outputs,
        "held_out_summary": {
            "case_count": len(cases),
            "gray_box_nrmse_median": float(np.median(gray)),
            "baseline_nrmse_median": float(np.median(baseline)),
            "relative_improvement_median": float(np.median((baseline - gray) / baseline)),
            "gray_box_wins": int(np.count_nonzero(gray < baseline)),
        },
        "held_out_cases": cases,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--derived", type=Path, default=DEFAULT_DERIVED)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    options = parser.parse_args()
    datasets = {waveform: load_dataset(options.derived, waveform) for waveform in WAVEFORMS}
    optimizer = fit(datasets)
    parameters = optimizer.x
    runtime_parameters = parameters.copy()
    runtime_parameters[1] = 0.0
    result = {
        "schema_version": 1,
        "profile_id": "korg-monologue-gray-box-saw-core-v1",
        "target_id": "korg-monologue-v1",
        "topology": "saw-core",
        "fit_scope": "one lowest-frequency cycle per independently captured waveform",
        "identifiability": "shared phase is not inferred because capture phase policies differ",
        "source_sha256": {
            waveform: file_sha256(options.derived / f"{waveform}-cycles-v1.npz")
            for waveform in WAVEFORMS
        },
        "fitted_physical_parameters": {
            "current_curvature_ratio": float(parameters[0]),
            "reset_duration_cycles": float(parameters[1]),
        },
        "runtime_physical_parameters": {
            "current_curvature_ratio": float(runtime_parameters[0]),
            "reset_duration_cycles": 0.0,
        },
        "runtime_deviation_reason": "the fitted finite-reset BLAMP increased high-frequency residual, so the bounded runtime uses the instantaneous-reset BLEP ablation",
        "optimizer": {
            "success": bool(optimizer.success),
            "status": int(optimizer.status),
            "message": str(optimizer.message),
            "function_evaluations": int(optimizer.nfev),
            "cost": float(optimizer.cost),
            "optimality": float(optimizer.optimality),
            "active_bounds": [int(value) for value in optimizer.active_mask],
        },
        "comparison_phase_shift_cycles": {
            waveform: float(parameters[5 + index]) for index, waveform in enumerate(WAVEFORMS)
        },
        **summarize(datasets, runtime_parameters),
    }
    options.output.parent.mkdir(parents=True, exist_ok=True)
    options.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(options.output)


if __name__ == "__main__":
    main()
