#!/usr/bin/env python3
"""Fit the compact pitch-conditioned phase/filter oscillator profile."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np
from scipy.optimize import least_squares


REPO_ROOT = Path(__file__).resolve().parents[4]
DEFAULT_DERIVED = REPO_ROOT / "target/analog-osc/reference/korg-monologue-v1/derived"
DEFAULT_PROFILE = (
    REPO_ROOT
    / "plans/analog-osc/research/profiles/korg-monologue-phase-filter-v1.json"
)
DEFAULT_RUST = REPO_ROOT / "synth-core/src/dsp/target_conditioned_profile.rs"
SAMPLE_RATE_HZ = 48_000.0
PHASE_BINS = 2_048
FIT_REVISION = 1
PARAMETER_NAMES = (
    "phase_a",
    "phase_b",
    "phase_offset_cycles",
    "log_lowpass_hz",
    "log_highpass_hz",
    "log_pole_hz",
    "log_zero_hz",
    "gain",
)
LOWER_BOUNDS = np.asarray(
    [
        -0.70,
        -0.50,
        -0.15,
        math.log(800.0),
        math.log(0.1),
        math.log(10.0),
        math.log(10.0),
        0.2,
    ],
    dtype=np.float64,
)
UPPER_BOUNDS = np.asarray(
    [
        0.70,
        0.50,
        0.15,
        math.log(60_000.0),
        math.log(500.0),
        math.log(60_000.0),
        math.log(60_000.0),
        3.0,
    ],
    dtype=np.float64,
)


@dataclass(frozen=True)
class FitDataset:
    waveform: str
    cycles: np.ndarray
    frequencies_hz: np.ndarray
    records: list[dict[str, Any]]
    source_npz_sha256: str
    source_pickle_md5: str


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def load_dataset(root: Path, waveform: str) -> FitDataset:
    npz_path = root / f"{waveform}-cycles-v1.npz"
    summary_path = root / f"{waveform}-summary-v1.json"
    with np.load(npz_path) as values:
        cycles = np.asarray(values["median_cycles"], dtype=np.float64)
        frequencies = np.asarray(values["measured_frequency_hz"], dtype=np.float64)
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    if cycles.shape != (72, PHASE_BINS) or frequencies.shape != (72,):
        raise ValueError(f"unexpected {waveform} derived shape: {cycles.shape}")
    if not np.isfinite(cycles).all() or not np.isfinite(frequencies).all():
        raise ValueError(f"{waveform} contains non-finite derived data")
    return FitDataset(
        waveform=waveform,
        cycles=cycles,
        frequencies_hz=frequencies,
        records=summary["pitches"],
        source_npz_sha256=sha256_file(npz_path),
        source_pickle_md5=summary["source_md5"],
    )


def phase_map(phase: np.ndarray, phase_a: float, phase_b: float, offset: float) -> np.ndarray:
    shifted = np.mod(phase + offset, 1.0)
    warped = (
        shifted
        + phase_a * np.sin(2.0 * np.pi * shifted) / (2.0 * np.pi)
        + phase_b * np.sin(4.0 * np.pi * shifted) / (4.0 * np.pi)
    )
    return np.mod(warped, 1.0)


def minimum_phase_derivative(phase_a: float, phase_b: float, bins: int = PHASE_BINS) -> float:
    phase = np.arange(bins, dtype=np.float64) / bins
    derivative = 1.0 + phase_a * np.cos(2.0 * np.pi * phase)
    derivative += phase_b * np.cos(4.0 * np.pi * phase)
    return float(np.min(derivative))


def geometric_source(waveform: str, phase: np.ndarray) -> np.ndarray:
    if waveform == "saw":
        return 2.0 * np.mod(phase + 0.5, 1.0) - 1.0
    if waveform == "triangle":
        return 1.0 - 4.0 * np.abs(np.mod(phase + 0.25, 1.0) - 0.5)
    if waveform == "square":
        return np.where(phase < 0.5, 1.0, -1.0)
    raise ValueError(f"unknown waveform {waveform!r}")


def render_cycle(
    waveform: str,
    frequency_hz: float,
    parameters: np.ndarray,
    dc: float,
    bins: int = PHASE_BINS,
    sample_rate_hz: float = SAMPLE_RATE_HZ,
) -> np.ndarray:
    phase = np.arange(bins, dtype=np.float64) / bins
    phase_a, phase_b, offset = parameters[:3]
    source = geometric_source(waveform, phase_map(phase, phase_a, phase_b, offset))
    source_harmonics = np.fft.rfft(source) / bins
    harmonics = np.arange(source_harmonics.size, dtype=np.float64)
    omega = 2.0 * np.pi * harmonics * frequency_hz / sample_rate_hz
    z_inverse = np.exp(-1j * omega)

    lowpass_hz, highpass_hz, pole_hz, zero_hz = np.exp(parameters[3:7])
    lowpass_pole = np.exp(-2.0 * np.pi * lowpass_hz / sample_rate_hz)
    response = (1.0 - lowpass_pole) / (1.0 - lowpass_pole * z_inverse)

    highpass_pole = np.exp(-2.0 * np.pi * highpass_hz / sample_rate_hz)
    response *= highpass_pole * (1.0 - z_inverse) / (1.0 - highpass_pole * z_inverse)

    pole = np.exp(-2.0 * np.pi * pole_hz / sample_rate_hz)
    zero = np.exp(-2.0 * np.pi * zero_hz / sample_rate_hz)
    response *= (1.0 - zero * z_inverse) / (1.0 - pole * z_inverse)
    output_harmonics = source_harmonics * response * parameters[7]
    output_harmonics[harmonics * frequency_hz >= sample_rate_hz * 0.49] = 0.0
    output_harmonics[0] = complex(dc, 0.0)
    return np.fft.irfft(output_harmonics * bins, n=bins)


def identity_cycle(waveform: str, frequency_hz: float, target: np.ndarray) -> np.ndarray:
    phase = np.arange(target.size, dtype=np.float64) / target.size
    source = geometric_source(waveform, phase)
    harmonics = np.fft.rfft(source)
    indices = np.arange(harmonics.size)
    harmonics[indices * frequency_hz >= SAMPLE_RATE_HZ * 0.49] = 0.0
    source = np.fft.irfft(harmonics, n=target.size)
    centered_source = source - np.mean(source)
    centered_target = target - np.mean(target)
    gain = np.dot(centered_source, centered_target) / max(
        np.dot(centered_source, centered_source), np.finfo(np.float64).tiny
    )
    return centered_source * gain + np.mean(target)


def normalized_errors(target: np.ndarray, candidate: np.ndarray, frequency_hz: float) -> dict[str, float]:
    centered_target = target - np.mean(target)
    centered_candidate = candidate - np.mean(candidate)
    target_rms = np.sqrt(np.mean(centered_target * centered_target))
    time_nrmse = np.sqrt(np.mean((candidate - target) ** 2)) / max(
        target_rms, np.finfo(np.float64).tiny
    )
    target_harmonics = np.fft.rfft(centered_target) / target.size
    candidate_harmonics = np.fft.rfft(centered_candidate) / target.size
    harmonic_indices = np.arange(target_harmonics.size)
    legal = (harmonic_indices > 0) & (
        harmonic_indices * frequency_hz < SAMPLE_RATE_HZ * 0.49
    )
    complex_error = np.linalg.norm(candidate_harmonics[legal] - target_harmonics[legal])
    complex_error /= max(np.linalg.norm(target_harmonics[legal]), np.finfo(np.float64).tiny)
    return {
        "time_nrmse": float(time_nrmse),
        "complex_harmonic_nrmse": float(complex_error),
    }


def initial_parameters(target: np.ndarray) -> np.ndarray:
    centered_rms = np.sqrt(np.mean((target - np.mean(target)) ** 2))
    return np.asarray(
        [
            0.0,
            0.0,
            0.0,
            math.log(12_000.0),
            math.log(10.0),
            math.log(1_000.0),
            math.log(1_000.0),
            centered_rms / 0.58,
        ],
        dtype=np.float64,
    )


def fit_cycle(
    waveform: str,
    frequency_hz: float,
    target: np.ndarray,
    initial: np.ndarray | None,
    max_nfev: int,
) -> tuple[np.ndarray, dict[str, float]]:
    target_rms = np.sqrt(np.mean((target - np.mean(target)) ** 2))
    dc = float(np.mean(target))
    start = np.clip(
        initial_parameters(target) if initial is None else initial,
        LOWER_BOUNDS + 1.0e-9,
        UPPER_BOUNDS - 1.0e-9,
    )

    def residual(parameters: np.ndarray) -> np.ndarray:
        candidate = render_cycle(waveform, frequency_hz, parameters, dc)
        time_error = (candidate - target) / max(target_rms, np.finfo(np.float64).tiny)
        target_harmonics = np.fft.rfft(target - dc) / target.size
        candidate_harmonics = np.fft.rfft(candidate - dc) / target.size
        harmonic_count = min(129, target_harmonics.size)
        harmonic_scale = max(
            np.linalg.norm(target_harmonics[1:harmonic_count]),
            np.finfo(np.float64).tiny,
        )
        complex_error = (candidate_harmonics[1:harmonic_count] - target_harmonics[1:harmonic_count])
        complex_error = np.concatenate((complex_error.real, complex_error.imag))
        complex_error *= math.sqrt(target.size) * 0.35 / harmonic_scale
        derivative_margin = minimum_phase_derivative(parameters[0], parameters[1]) - 0.08
        monotonic_penalty = np.asarray([min(derivative_margin, 0.0) * 100.0])
        return np.concatenate((time_error, complex_error, monotonic_penalty))

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
    candidate = render_cycle(waveform, frequency_hz, result.x, dc)
    diagnostics = normalized_errors(target, candidate, frequency_hz)
    diagnostics.update(
        {
            "minimum_phase_derivative": minimum_phase_derivative(result.x[0], result.x[1]),
            "optimizer_cost": float(result.cost),
            "optimizer_evaluations": int(result.nfev),
            "optimizer_success": bool(result.success),
        }
    )
    return result.x, diagnostics


def interpolate_parameters(
    frequency_hz: float, knot_frequencies: np.ndarray, knot_parameters: np.ndarray
) -> np.ndarray:
    log_frequency = math.log2(frequency_hz)
    log_knots = np.log2(knot_frequencies)
    if log_frequency <= log_knots[0]:
        return knot_parameters[0].copy()
    if log_frequency >= log_knots[-1]:
        return knot_parameters[-1].copy()
    upper = int(np.searchsorted(log_knots, log_frequency))
    lower = upper - 1
    amount = (log_frequency - log_knots[lower]) / (log_knots[upper] - log_knots[lower])
    return knot_parameters[lower] + (knot_parameters[upper] - knot_parameters[lower]) * amount


def summarize_metrics(records: list[dict[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for split in ("train", "validation", "test", "held_out"):
        selected = records if split == "held_out" else [row for row in records if row["split"] == split]
        if split == "held_out":
            selected = [row for row in records if row["split"] != "train"]
        if not selected:
            continue
        result[split] = {}
        for metric in (
            "baseline_time_nrmse",
            "model_time_nrmse",
            "baseline_complex_harmonic_nrmse",
            "model_complex_harmonic_nrmse",
        ):
            values = np.asarray([row[metric] for row in selected], dtype=np.float64)
            result[split][metric] = {
                "median": float(np.median(values)),
                "maximum": float(np.max(values)),
            }
        baseline = np.asarray([row["baseline_time_nrmse"] for row in selected])
        model = np.asarray([row["model_time_nrmse"] for row in selected])
        result[split]["time_nrmse_relative_improvement_median"] = float(
            np.median((baseline - model) / np.maximum(baseline, np.finfo(np.float64).tiny))
        )
    return result


def fit_waveform(dataset: FitDataset, max_nfev: int) -> dict[str, Any]:
    train_indices = [index for index, record in enumerate(dataset.records) if record["split"] == "train"]
    parameters = []
    knots = []
    previous = None
    for ordinal, index in enumerate(train_indices, start=1):
        frequency = float(dataset.frequencies_hz[index])
        fitted, diagnostics = fit_cycle(
            dataset.waveform,
            frequency,
            dataset.cycles[index],
            previous,
            max_nfev,
        )
        previous = fitted
        parameters.append(fitted)
        physical = np.exp(fitted[3:7])
        knots.append(
            {
                "pitch_index": index,
                "frequency_hz": frequency,
                "log2_frequency": math.log2(frequency),
                "phase_a": float(fitted[0]),
                "phase_b": float(fitted[1]),
                "phase_offset_cycles": float(fitted[2]),
                "lowpass_hz": float(physical[0]),
                "highpass_hz": float(physical[1]),
                "pole_hz": float(physical[2]),
                "zero_hz": float(physical[3]),
                "gain": float(fitted[7]),
                "dc": float(np.mean(dataset.cycles[index])),
                "fit": diagnostics,
            }
        )
        print(
            f"{dataset.waveform}: fitted {ordinal:02d}/{len(train_indices)} "
            f"index={index:02d} f={frequency:8.3f}Hz nrmse={diagnostics['time_nrmse']:.5f}",
            flush=True,
        )

    knot_frequencies = dataset.frequencies_hz[train_indices]
    knot_parameters = np.stack(parameters)
    evaluation = []
    for index, (target, frequency, source_record) in enumerate(
        zip(dataset.cycles, dataset.frequencies_hz, dataset.records)
    ):
        fitted = interpolate_parameters(float(frequency), knot_frequencies, knot_parameters)
        dc = float(np.interp(math.log2(frequency), np.log2(knot_frequencies), [knot["dc"] for knot in knots]))
        candidate = render_cycle(dataset.waveform, float(frequency), fitted, dc)
        baseline = identity_cycle(dataset.waveform, float(frequency), target)
        model_errors = normalized_errors(target, candidate, float(frequency))
        baseline_errors = normalized_errors(target, baseline, float(frequency))
        evaluation.append(
            {
                "pitch_index": index,
                "frequency_hz": float(frequency),
                "split": source_record["split"],
                "baseline_time_nrmse": baseline_errors["time_nrmse"],
                "model_time_nrmse": model_errors["time_nrmse"],
                "baseline_complex_harmonic_nrmse": baseline_errors["complex_harmonic_nrmse"],
                "model_complex_harmonic_nrmse": model_errors["complex_harmonic_nrmse"],
                "minimum_phase_derivative": minimum_phase_derivative(fitted[0], fitted[1]),
            }
        )
    return {
        "source_npz_sha256": dataset.source_npz_sha256,
        "source_pickle_md5": dataset.source_pickle_md5,
        "fit_knot_count": len(knots),
        "knots": knots,
        "evaluation": evaluation,
        "metrics": summarize_metrics(evaluation),
    }


def profile_checksum(profile: dict[str, Any]) -> str:
    canonical = json.dumps(profile, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(canonical).hexdigest()


def rust_float(value: float) -> str:
    if not math.isfinite(value):
        raise ValueError("cannot emit non-finite profile coefficient")
    return f"{value:.9e}_f32"


def write_rust_profile(path: Path, profile: dict[str, Any], checksum: str) -> None:
    lines = [
        "//! Generated by `plans/analog-osc/research/scripts/fit_target_conditioned_oscillator.py`.",
        "//! Do not edit fitted coefficients manually.",
        "",
        "use super::target_conditioned_oscillator::{PhaseFilterKnot, PhaseFilterProfile};",
        "",
        "pub const PROFILE_JSON_SHA256: &str =",
        f'    "{checksum}";',
        "",
    ]
    constant_names = {"saw": "SAW_KNOTS", "triangle": "TRIANGLE_KNOTS", "square": "PULSE_KNOTS"}
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
                    f"        phase_offset_cycles: {rust_float(knot['phase_offset_cycles'])},",
                    f"        lowpass_hz: {rust_float(knot['lowpass_hz'])},",
                    f"        highpass_hz: {rust_float(knot['highpass_hz'])},",
                    f"        pole_hz: {rust_float(knot['pole_hz'])},",
                    f"        zero_hz: {rust_float(knot['zero_hz'])},",
                    f"        gain: {rust_float(knot['gain'])},",
                    f"        dc: {rust_float(knot['dc'])},",
                    "    },",
                ]
            )
        lines.extend(["];", ""])
    lines.extend(
        [
            "pub static KORG_MONOLOGUE_PHASE_FILTER_V1: PhaseFilterProfile = PhaseFilterProfile {",
            '    id: "korg-monologue-phase-filter-v1",',
            '    target_id: "korg-monologue-v1",',
            "    revision: 1,",
            "    saw: &SAW_KNOTS,",
            "    triangle: &TRIANGLE_KNOTS,",
            "    pulse: &PULSE_KNOTS,",
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
    result.add_argument("--max-nfev", type=int, default=240)
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
        "model_id": "target-conditioned-phase-filter-v1",
        "profile_id": "korg-monologue-phase-filter-v1",
        "target_id": "korg-monologue-v1",
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "phase_bins": PHASE_BINS,
        "fit_policy": {
            "training_split": "pitch_index modulo 4 is 0 or 2",
            "interpolation": "piecewise linear in log2 measured frequency",
            "phase_map": "p + a*sin(2*pi*p)/(2*pi) + b*sin(4*pi*p)/(4*pi)",
            "minimum_phase_derivative": 0.08,
            "filter": "first-order lowpass, first-order highpass, pole-zero section",
            "objective": "normalized time error plus complex error for harmonics 1..128",
            "nonlinearity": "disabled",
        },
        "parameter_names": list(PARAMETER_NAMES),
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
