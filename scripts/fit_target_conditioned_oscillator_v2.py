#!/usr/bin/env python3
"""Fit the phase-invariant, production-source Plan 04 v2 profile."""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from pathlib import Path
from typing import Any

import numpy as np
from scipy.optimize import least_squares


REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT))

from scripts.evaluate_target_conditioned_ablations import (  # noqa: E402
    best_phase_alignment,
    centered_unit_rms,
    harmonic_errors,
)
from scripts.fit_target_conditioned_oscillator import (  # noqa: E402
    DEFAULT_DERIVED,
    PHASE_BINS,
    SAMPLE_RATE_HZ,
    FitDataset,
    interpolate_parameters,
    load_dataset,
    minimum_phase_derivative,
    profile_checksum,
    rust_float,
    sha256_file,
)


DEFAULT_PROFILE = REPO_ROOT / (
    "plans/analog-osc/research/profiles/korg-monologue-phase-filter-v2.json"
)
DEFAULT_RUST = REPO_ROOT / "synth-core/src/dsp/target_conditioned_profile_v2.rs"
BLEP_TABLE_SOURCE = REPO_ROOT / "synth-core/src/dsp/blep_table.rs"
FIT_REVISION = 2
MODEL_PARAMETER_NAMES = (
    "phase_a",
    "phase_b",
    "log_lowpass_hz",
    "log_highpass_hz",
    "log_pole_hz",
    "log_zero_hz",
)
OPTIMIZER_PARAMETER_NAMES = (*MODEL_PARAMETER_NAMES, "comparison_phase_shift_cycles")
LOWER_BOUNDS = np.asarray(
    [
        -0.70,
        -0.50,
        math.log(800.0),
        math.log(0.1),
        math.log(10.0),
        math.log(10.0),
        -0.15,
    ],
    dtype=np.float64,
)
UPPER_BOUNDS = np.asarray(
    [
        0.70,
        0.50,
        math.log(60_000.0),
        math.log(500.0),
        math.log(60_000.0),
        math.log(60_000.0),
        0.15,
    ],
    dtype=np.float64,
)
MAGNITUDE_OBJECTIVE_WEIGHT = 0.65


def load_blep_table(path: Path = BLEP_TABLE_SOURCE) -> np.ndarray:
    text = path.read_text(encoding="utf-8")
    values = re.findall(
        r"(?<![A-Za-z0-9_])([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[-+]?\d+)?)f32",
        text,
        flags=re.IGNORECASE,
    )
    table = np.asarray([float(value) for value in values], dtype=np.float64)
    if table.shape != (4096,) or not np.all(np.isfinite(table)):
        raise ValueError(f"unexpected BLEP table shape: {table.shape}")
    return table


BLEP_TABLE = load_blep_table()


def wrap01(values: np.ndarray | float) -> np.ndarray:
    return np.mod(values, 1.0)


def unwrapped_phase_map(phase: np.ndarray, phase_a: float, phase_b: float) -> np.ndarray:
    return (
        phase
        + phase_a * np.sin(2.0 * np.pi * phase) / (2.0 * np.pi)
        + phase_b * np.sin(4.0 * np.pi * phase) / (4.0 * np.pi)
    )


def table_blep_saw(phase: np.ndarray, phase_increment: float) -> np.ndarray:
    values = wrap01(np.asarray(phase, dtype=np.float64))
    output = 2.0 * values - 1.0
    if phase_increment <= 0.0:
        return output
    points = 4 if phase_increment <= 0.125 else 2 if phase_increment <= 0.25 else 1
    window = phase_increment * points
    correction = np.zeros_like(values)

    left = values > 1.0 - window
    if np.any(left):
        t = (1.0 - values[left]) / window
        indices = ((1.0 - t) * 2047.0).astype(np.int64)
        correction[left] = -BLEP_TABLE[np.minimum(indices, 2047)]
    right = values < window
    if np.any(right):
        t = values[right] / window
        indices = (t * 2047.0).astype(np.int64) + 2048
        correction[right] = -BLEP_TABLE[np.minimum(indices, 4095)]
    return output + correction


def polyblamp2_corner(phase_from_corner: np.ndarray, increment: np.ndarray) -> np.ndarray:
    phase = wrap01(phase_from_corner)
    active = (increment > 0.0) & (increment < 0.25)
    distance = np.where(
        phase < increment,
        phase,
        np.where(phase > 1.0 - increment, 1.0 - phase, np.nan),
    )
    corner = active & np.isfinite(distance)
    output = np.zeros_like(phase)
    safe_increment = np.maximum(increment[corner], 1.0e-12)
    t = 1.0 - distance[corner] / safe_increment
    output[corner] = t * t * t * safe_increment / 3.0
    return output


def polyblamp2_triangle(phase: np.ndarray, increment: np.ndarray) -> np.ndarray:
    values = wrap01(phase)
    naive = 1.0 - np.abs(values - 0.5) * 4.0
    return (
        naive
        + 8.0 * polyblamp2_corner(values, increment)
        - 8.0 * polyblamp2_corner(wrap01(values - 0.5), increment)
    )


def production_source(waveform: str, frequency_hz: float, bins: int = PHASE_BINS) -> np.ndarray:
    phase = np.arange(bins, dtype=np.float64) / bins
    increment = frequency_hz / SAMPLE_RATE_HZ
    if waveform == "saw":
        return table_blep_saw(phase, increment)
    if waveform == "triangle":
        return polyblamp2_triangle(phase, np.full(bins, increment))
    if waveform == "square":
        return table_blep_saw(phase, increment) - table_blep_saw(
            wrap01(phase + 0.5), increment
        )
    raise ValueError(f"unknown waveform {waveform!r}")


def threshold_phase(phase_a: float, phase_b: float, threshold: float) -> float:
    lower = 0.0
    upper = 1.0
    for _ in range(48):
        midpoint = (lower + upper) * 0.5
        mapped = float(unwrapped_phase_map(np.asarray([midpoint]), phase_a, phase_b)[0])
        if mapped < threshold:
            lower = midpoint
        else:
            upper = midpoint
    return (lower + upper) * 0.5


def conditioned_source(
    waveform: str,
    frequency_hz: float,
    phase_a: float,
    phase_b: float,
    bins: int = PHASE_BINS,
) -> np.ndarray:
    phase = np.arange(bins, dtype=np.float64) / bins
    warped = unwrapped_phase_map(phase, phase_a, phase_b)
    derivative = np.maximum(
        1.0
        + phase_a * np.cos(2.0 * np.pi * phase)
        + phase_b * np.cos(4.0 * np.pi * phase),
        0.08,
    )
    increment = frequency_hz / SAMPLE_RATE_HZ
    if waveform == "saw":
        # The sine warp is anchored at both wrap boundaries, so the production
        # edge remains at phase zero. Correct that edge once, then add only the
        # smooth curvature introduced by the warp.
        return table_blep_saw(phase, increment) + 2.0 * (warped - phase)
    if waveform == "triangle":
        return polyblamp2_triangle(
            wrap01(warped), np.minimum(increment * derivative, 0.499)
        )
    if waveform == "square":
        falling_phase = threshold_phase(phase_a, phase_b, 0.5)
        return table_blep_saw(phase, increment) - table_blep_saw(
            wrap01(phase - falling_phase), increment
        )
    raise ValueError(f"unknown waveform {waveform!r}")


def filter_cycle(
    source: np.ndarray,
    frequency_hz: float,
    model_parameters: np.ndarray,
    dc: float,
    gain: float = 1.0,
) -> np.ndarray:
    lowpass_hz, highpass_hz, pole_hz, zero_hz = np.exp(model_parameters[2:6])
    harmonics = np.arange(source.size // 2 + 1, dtype=np.float64)
    omega = 2.0 * np.pi * harmonics * frequency_hz / SAMPLE_RATE_HZ
    z_inverse = np.exp(-1j * omega)
    lowpass_pole = np.exp(-2.0 * np.pi * lowpass_hz / SAMPLE_RATE_HZ)
    response = (1.0 - lowpass_pole) / (1.0 - lowpass_pole * z_inverse)
    highpass_pole = np.exp(-2.0 * np.pi * highpass_hz / SAMPLE_RATE_HZ)
    response *= highpass_pole * (1.0 - z_inverse) / (1.0 - highpass_pole * z_inverse)
    pole = np.exp(-2.0 * np.pi * pole_hz / SAMPLE_RATE_HZ)
    zero = np.exp(-2.0 * np.pi * zero_hz / SAMPLE_RATE_HZ)
    response *= (1.0 - zero * z_inverse) / (1.0 - pole * z_inverse)
    output_harmonics = np.fft.rfft(source) * response * gain
    output_harmonics[harmonics * frequency_hz >= SAMPLE_RATE_HZ * 0.49] = 0.0
    output_harmonics[0] = complex(dc * source.size, 0.0)
    return np.fft.irfft(output_harmonics, n=source.size)


def render_cycle(
    waveform: str,
    frequency_hz: float,
    model_parameters: np.ndarray,
    dc: float,
    gain: float = 1.0,
) -> np.ndarray:
    source = conditioned_source(
        waveform, frequency_hz, model_parameters[0], model_parameters[1]
    )
    return filter_cycle(source, frequency_hz, model_parameters, dc, gain)


def periodic_shift(cycle: np.ndarray, shift_cycles: float) -> np.ndarray:
    harmonics = np.fft.rfft(np.asarray(cycle, dtype=np.float64))
    indices = np.arange(harmonics.size, dtype=np.float64)
    harmonics *= np.exp(2j * np.pi * indices * shift_cycles)
    return np.fft.irfft(harmonics, n=cycle.size)


def initial_parameters() -> np.ndarray:
    return np.asarray(
        [
            0.0,
            0.0,
            math.log(18_000.0),
            math.log(10.0),
            math.log(1_000.0),
            math.log(1_000.0),
            0.0,
        ],
        dtype=np.float64,
    )


def optimal_gain(target: np.ndarray, candidate: np.ndarray) -> float:
    centered_target = target - np.mean(target)
    centered_candidate = candidate - np.mean(candidate)
    gain = float(
        np.dot(centered_candidate, centered_target)
        / max(np.dot(centered_candidate, centered_candidate), np.finfo(np.float64).tiny)
    )
    return float(np.clip(gain, 0.01, 100.0))


def target_metrics(
    target: np.ndarray, candidate: np.ndarray, frequency_hz: float
) -> dict[str, float]:
    shift, phase_aligned_error, _ = best_phase_alignment(target, candidate, 0.0)
    complex_error, magnitude_error = harmonic_errors(target, candidate, frequency_hz)
    fixed_error = float(
        np.sqrt(
            np.mean(
                (centered_unit_rms(candidate) - centered_unit_rms(target)) ** 2
            )
        )
    )
    return {
        "fixed_phase_shape_nrmse": fixed_error,
        "phase_aligned_shape_nrmse": phase_aligned_error,
        "complex_harmonic_nrmse": complex_error,
        "magnitude_harmonic_nrmse": magnitude_error,
        "optimal_target_phase_shift_cycles": shift,
    }


def fit_cycle(
    waveform: str,
    frequency_hz: float,
    target: np.ndarray,
    initial: np.ndarray | None,
    max_nfev: int,
) -> tuple[np.ndarray, float, dict[str, float]]:
    baseline = production_source(waveform, frequency_hz)
    canonical_shift, _, canonical_target = best_phase_alignment(target, baseline, 0.0)
    start = np.clip(
        initial_parameters() if initial is None else initial,
        LOWER_BOUNDS + 1.0e-9,
        UPPER_BOUNDS - 1.0e-9,
    )
    target_magnitudes = np.abs(np.fft.rfft(centered_unit_rms(canonical_target)))
    legal_count = min(129, target_magnitudes.size)
    magnitude_scale = max(
        float(np.linalg.norm(target_magnitudes[1:legal_count])),
        np.finfo(np.float64).tiny,
    )

    def residual(parameters: np.ndarray) -> np.ndarray:
        model_parameters = parameters[:6]
        candidate = render_cycle(waveform, frequency_hz, model_parameters, 0.0)
        candidate_shape = centered_unit_rms(candidate)
        shifted_target = centered_unit_rms(
            periodic_shift(canonical_target, parameters[6])
        )
        time_error = (candidate_shape - shifted_target) / math.sqrt(target.size)
        candidate_magnitudes = np.abs(np.fft.rfft(candidate_shape))
        magnitude_error = (
            candidate_magnitudes[1:legal_count] - target_magnitudes[1:legal_count]
        )
        magnitude_error *= MAGNITUDE_OBJECTIVE_WEIGHT / magnitude_scale
        derivative_margin = minimum_phase_derivative(
            model_parameters[0], model_parameters[1]
        ) - 0.08
        monotonic_penalty = np.asarray([min(derivative_margin, 0.0) * 100.0])
        return np.concatenate((time_error, magnitude_error, monotonic_penalty))

    result = least_squares(
        residual,
        start,
        bounds=(LOWER_BOUNDS, UPPER_BOUNDS),
        x_scale="jac",
        max_nfev=max_nfev,
        ftol=1.0e-8,
        xtol=1.0e-8,
        gtol=1.0e-8,
    )
    model_parameters = result.x[:6]
    ungained = render_cycle(waveform, frequency_hz, model_parameters, 0.0)
    fitted_target = periodic_shift(canonical_target, result.x[6])
    gain = optimal_gain(fitted_target, ungained)
    candidate = render_cycle(
        waveform, frequency_hz, model_parameters, float(np.mean(target)), gain
    )
    diagnostics = target_metrics(target, candidate, frequency_hz)
    diagnostics.update(
        {
            "canonical_target_shift_cycles": canonical_shift,
            "comparison_nuisance_shift_cycles": float(result.x[6]),
            "minimum_phase_derivative": minimum_phase_derivative(
                model_parameters[0], model_parameters[1]
            ),
            "derived_gain": gain,
            "optimizer_cost": float(result.cost),
            "optimizer_evaluations": int(result.nfev),
            "optimizer_success": bool(result.success),
        }
    )
    return result.x, gain, diagnostics


def evaluate_waveform(
    dataset: FitDataset,
    knot_frequencies: np.ndarray,
    knot_parameters: np.ndarray,
    knot_gains: np.ndarray,
    knot_dc: np.ndarray,
) -> list[dict[str, Any]]:
    evaluation = []
    for index, (target, frequency_hz, source_record) in enumerate(
        zip(dataset.cycles, dataset.frequencies_hz, dataset.records)
    ):
        frequency = float(frequency_hz)
        parameters = interpolate_parameters(
            frequency, knot_frequencies, knot_parameters[:, :6]
        )
        gain = float(
            np.interp(math.log2(frequency), np.log2(knot_frequencies), knot_gains)
        )
        dc = float(np.interp(math.log2(frequency), np.log2(knot_frequencies), knot_dc))
        candidate = render_cycle(dataset.waveform, frequency, parameters, dc, gain)
        baseline = production_source(dataset.waveform, frequency)
        candidate_metrics = target_metrics(target, candidate, frequency)
        baseline_metrics = target_metrics(target, baseline, frequency)
        evaluation.append(
            {
                "pitch_index": index,
                "frequency_hz": frequency,
                "split": source_record["split"],
                "baseline_phase_aligned_shape_nrmse": baseline_metrics[
                    "phase_aligned_shape_nrmse"
                ],
                "model_phase_aligned_shape_nrmse": candidate_metrics[
                    "phase_aligned_shape_nrmse"
                ],
                "baseline_magnitude_harmonic_nrmse": baseline_metrics[
                    "magnitude_harmonic_nrmse"
                ],
                "model_magnitude_harmonic_nrmse": candidate_metrics[
                    "magnitude_harmonic_nrmse"
                ],
                "baseline_optimal_target_phase_shift_cycles": baseline_metrics[
                    "optimal_target_phase_shift_cycles"
                ],
                "model_optimal_target_phase_shift_cycles": candidate_metrics[
                    "optimal_target_phase_shift_cycles"
                ],
                "minimum_phase_derivative": minimum_phase_derivative(
                    parameters[0], parameters[1]
                ),
            }
        )
    return evaluation


def summarize_metrics(records: list[dict[str, Any]]) -> dict[str, Any]:
    result = {}
    for split in ("train", "validation", "test", "held_out"):
        selected = (
            [row for row in records if row["split"] != "train"]
            if split == "held_out"
            else [row for row in records if row["split"] == split]
        )
        if not selected:
            continue
        split_result = {}
        for metric in (
            "baseline_phase_aligned_shape_nrmse",
            "model_phase_aligned_shape_nrmse",
            "baseline_magnitude_harmonic_nrmse",
            "model_magnitude_harmonic_nrmse",
        ):
            values = np.asarray([row[metric] for row in selected])
            split_result[metric] = {
                "median": float(np.median(values)),
                "maximum": float(np.max(values)),
            }
        for family in ("phase_aligned_shape", "magnitude_harmonic"):
            baseline = np.asarray([row[f"baseline_{family}_nrmse"] for row in selected])
            model = np.asarray([row[f"model_{family}_nrmse"] for row in selected])
            split_result[f"{family}_wins"] = int(np.sum(model < baseline))
            split_result[f"{family}_relative_improvement_median"] = float(
                np.median(
                    (baseline - model)
                    / np.maximum(baseline, np.finfo(np.float64).tiny)
                )
            )
        result[split] = split_result
    return result


def fit_waveform(dataset: FitDataset, max_nfev: int) -> dict[str, Any]:
    train_indices = [
        index for index, record in enumerate(dataset.records) if record["split"] == "train"
    ]
    optimizer_parameters = []
    gains = []
    knots = []
    previous = None
    for ordinal, index in enumerate(train_indices, start=1):
        frequency = float(dataset.frequencies_hz[index])
        fitted, gain, diagnostics = fit_cycle(
            dataset.waveform,
            frequency,
            dataset.cycles[index],
            previous,
            max_nfev,
        )
        previous = fitted
        optimizer_parameters.append(fitted)
        gains.append(gain)
        physical = np.exp(fitted[2:6])
        knots.append(
            {
                "pitch_index": index,
                "frequency_hz": frequency,
                "log2_frequency": math.log2(frequency),
                "phase_a": float(fitted[0]),
                "phase_b": float(fitted[1]),
                "phase_offset_cycles": 0.0,
                "lowpass_hz": float(physical[0]),
                "highpass_hz": float(physical[1]),
                "pole_hz": float(physical[2]),
                "zero_hz": float(physical[3]),
                "gain": gain,
                "dc": float(np.mean(dataset.cycles[index])),
                "fit": diagnostics,
            }
        )
        print(
            f"{dataset.waveform}: fitted {ordinal:02d}/{len(train_indices)} "
            f"index={index:02d} f={frequency:8.3f}Hz "
            f"shape={diagnostics['phase_aligned_shape_nrmse']:.5f} "
            f"mag={diagnostics['magnitude_harmonic_nrmse']:.5f}",
            flush=True,
        )

    parameters = np.stack(optimizer_parameters)
    knot_frequencies = dataset.frequencies_hz[train_indices]
    evaluation = evaluate_waveform(
        dataset,
        knot_frequencies,
        parameters,
        np.asarray(gains),
        np.asarray([knot["dc"] for knot in knots]),
    )
    return {
        "source_npz_sha256": dataset.source_npz_sha256,
        "source_pickle_md5": dataset.source_pickle_md5,
        "fit_knot_count": len(knots),
        "knots": knots,
        "evaluation": evaluation,
        "metrics": summarize_metrics(evaluation),
    }


def write_rust_profile(path: Path, profile: dict[str, Any], checksum: str) -> None:
    lines = [
        "//! Generated by `scripts/fit_target_conditioned_oscillator_v2.py`.",
        "//! Do not edit fitted coefficients manually.",
        "",
        "use super::target_conditioned_oscillator::{PhaseFilterKnot, PhaseFilterProfile};",
        "",
        "pub const PROFILE_JSON_SHA256_V2: &str =",
        f'    "{checksum}";',
        "",
    ]
    constant_names = {
        "saw": "SAW_KNOTS_V2",
        "triangle": "TRIANGLE_KNOTS_V2",
        "square": "PULSE_KNOTS_V2",
    }
    for waveform, constant in constant_names.items():
        knots = profile["waveforms"][waveform]["knots"]
        lines.append(f"const {constant}: [PhaseFilterKnot; {len(knots)}] = [")
        for knot in knots:
            lines.extend(
                [
                    "    PhaseFilterKnot {",
                    f"        log2_frequency: {rust_float(knot['log2_frequency'])},",
                    f"        phase_a: {rust_float(knot['phase_a'])},",
                    f"        phase_b: {rust_float(knot['phase_b'])},",
                    "        phase_offset_cycles: 0.0_f32,",
                    f"        lowpass_hz: {rust_float(knot['lowpass_hz'])},",
                    f"        highpass_hz: {rust_float(knot['highpass_hz'])},",
                    f"        pole_hz: {rust_float(knot['pole_hz'])},",
                    f"        zero_hz: {rust_float(knot['zero_hz'])},",
                    f"        gain: {rust_float(knot['gain'])},",
                    f"        dc: {rust_float(knot['dc'])},",
                    "    },",
                ]
            )
        lines.append("];")
        lines.append("")
    lines.extend(
        [
            "pub static KORG_MONOLOGUE_PHASE_FILTER_V2: PhaseFilterProfile = PhaseFilterProfile {",
            '    id: "korg-monologue-phase-filter-v2",',
            '    target_id: "korg-monologue-v1",',
            "    revision: 2,",
            "    saw: &SAW_KNOTS_V2,",
            "    triangle: &TRIANGLE_KNOTS_V2,",
            "    pulse: &PULSE_KNOTS_V2,",
            "};",
            "",
        ]
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8")


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--derived-root", type=Path, default=DEFAULT_DERIVED)
    result.add_argument("--profile-json", type=Path, default=DEFAULT_PROFILE)
    result.add_argument("--rust-output", type=Path, default=DEFAULT_RUST)
    result.add_argument("--max-nfev", type=int, default=180)
    return result


def main() -> int:
    args = parser().parse_args()
    if args.max_nfev < 20:
        raise ValueError("--max-nfev must be at least 20")
    waveforms = {
        waveform: fit_waveform(load_dataset(args.derived_root, waveform), args.max_nfev)
        for waveform in ("saw", "triangle", "square")
    }
    profile: dict[str, Any] = {
        "schema_version": 1,
        "fit_revision": FIT_REVISION,
        "model_id": "target-conditioned-phase-filter-v2",
        "profile_id": "korg-monologue-phase-filter-v2",
        "target_id": "korg-monologue-v1",
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "phase_bins": PHASE_BINS,
        "fit_policy": {
            "training_split": "pitch_index modulo 4 is 0 or 2",
            "interpolation": "piecewise linear in log2 measured frequency",
            "initial_runtime_phase": 0.0,
            "production_source": "table-BLEP saw/pulse and PolyBLAMP triangle, matching the compiled production phase convention",
            "phase_map": "p + a*sin(2*pi*p)/(2*pi) + b*sin(4*pi*p)/(4*pi), anchored at phase zero",
            "stored_global_phase_offset": False,
            "comparison_phase_policy": "align target to production source, then optimize and discard one bounded nuisance cycle shift per fit",
            "minimum_phase_derivative": 0.08,
            "filter": "first-order lowpass, first-order highpass, pole-zero section",
            "objective": "centered unit-RMS phase-aligned shape plus normalized harmonic-magnitude error",
            "harmonic_magnitude_weight": MAGNITUDE_OBJECTIVE_WEIGHT,
            "gain": "derived after shape fit by bounded least squares",
            "nonlinearity": "disabled",
            "listening_policy": "no revealed listening choice is used as a fit target or coefficient constraint",
        },
        "model_parameter_names": list(MODEL_PARAMETER_NAMES),
        "optimizer_parameter_names": list(OPTIMIZER_PARAMETER_NAMES),
        "blep_table_sha256": sha256_file(BLEP_TABLE_SOURCE),
        "waveforms": waveforms,
    }
    checksum = profile_checksum(profile)
    profile["profile_content_sha256"] = checksum
    args.profile_json.parent.mkdir(parents=True, exist_ok=True)
    args.profile_json.write_text(json.dumps(profile, indent=2) + "\n", encoding="utf-8")
    write_rust_profile(args.rust_output, profile, checksum)
    print(f"wrote {args.profile_json}")
    print(f"wrote {args.rust_output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
