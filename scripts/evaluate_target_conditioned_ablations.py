#!/usr/bin/env python3
"""Evaluate Plan 04 phase/filter ablations across every validation pitch."""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
from pathlib import Path

import numpy as np
from scipy.optimize import minimize_scalar
from scipy.stats import spearmanr


REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT))
os.environ.setdefault("MPLCONFIGDIR", str(REPO_ROOT / "target/matplotlib"))

from scripts.evaluate_target_conditioned_sweeps import spectral_residual  # noqa: E402
from scripts.generate_target_conditioned_ablation_listening_set import (  # noqa: E402
    DEFAULT_BINARY,
    DEFAULT_DERIVED,
    DEFAULT_PROFILE,
    VARIANTS,
)
from scripts.generate_target_conditioned_listening_set import (  # noqa: E402
    SAMPLE_RATE_HZ,
    git_state,
    render_model,
    sha256_file,
)


DEFAULT_OUTPUT = REPO_ROOT / (
    "plans/analog-osc/research/reports/"
    "korg-monologue-phase-filter-ablation-curves-v1.json"
)
DEFAULT_PLOT = REPO_ROOT / (
    "plans/analog-osc/research/plots/"
    "korg-monologue-phase-filter-ablation-curves-v1.svg"
)
DEFAULT_LISTENING_ROOT = REPO_ROOT / (
    "target/analog-osc/listening/korg-monologue-phase-filter-ablation-v1"
)
WAVEFORMS = ("saw", "triangle", "square")
TARGET_METRICS = (
    "native_time_nrmse",
    "level_matched_time_nrmse",
    "phase_aligned_time_nrmse",
    "complex_harmonic_nrmse",
    "magnitude_harmonic_nrmse",
)
ALL_METRICS = (*TARGET_METRICS, "spectral_residual_dbc")


def centered_unit_rms(samples: np.ndarray) -> np.ndarray:
    values = np.asarray(samples, dtype=np.float64)
    centered = values - np.mean(values)
    rms = float(np.sqrt(np.mean(centered * centered)))
    if not math.isfinite(rms) or rms <= np.finfo(np.float64).tiny:
        raise ValueError("cannot normalize silent or non-finite samples")
    return centered / rms


def nrmse(reference: np.ndarray, candidate: np.ndarray) -> float:
    centered_reference = reference - np.mean(reference)
    scale = float(np.sqrt(np.mean(centered_reference * centered_reference)))
    return float(
        np.sqrt(np.mean((np.asarray(candidate) - reference) ** 2))
        / max(scale, np.finfo(np.float64).tiny)
    )


def harmonic_errors(
    reference: np.ndarray, candidate: np.ndarray, frequency_hz: float
) -> tuple[float, float]:
    reference_harmonics = np.fft.rfft(centered_unit_rms(reference)) / reference.size
    candidate_harmonics = np.fft.rfft(centered_unit_rms(candidate)) / candidate.size
    indices = np.arange(reference_harmonics.size)
    legal = (indices > 0) & (indices * frequency_hz < SAMPLE_RATE_HZ * 0.49)
    scale = max(
        float(np.linalg.norm(reference_harmonics[legal])),
        np.finfo(np.float64).tiny,
    )
    complex_error = float(
        np.linalg.norm(candidate_harmonics[legal] - reference_harmonics[legal])
        / scale
    )
    magnitude_error = float(
        np.linalg.norm(
            np.abs(candidate_harmonics[legal]) - np.abs(reference_harmonics[legal])
        )
        / scale
    )
    return complex_error, magnitude_error


def best_phase_alignment(
    target_cycle: np.ndarray,
    candidate_cycle: np.ndarray,
    initial_phase: float,
) -> tuple[float, float, np.ndarray]:
    candidate = centered_unit_rms(candidate_cycle)
    phase = initial_phase + np.arange(candidate.size, dtype=np.float64) / candidate.size

    def error(shift: float) -> float:
        shifted = centered_unit_rms(
            np.interp(
                np.mod(phase + shift, 1.0),
                np.arange(target_cycle.size + 1, dtype=np.float64) / target_cycle.size,
                np.concatenate((target_cycle, target_cycle[:1])),
            )
        )
        return float(np.sqrt(np.mean((candidate - shifted) ** 2)))

    grid = np.linspace(-0.5, 0.5, 513, endpoint=False)
    errors = np.asarray([error(float(shift)) for shift in grid])
    best = float(grid[int(np.argmin(errors))])
    step = 1.0 / grid.size
    result = minimize_scalar(error, bounds=(best - step, best + step), method="bounded")
    shift = float((result.x + 0.5) % 1.0 - 0.5)
    aligned_target = np.interp(
        np.mod(phase + shift, 1.0),
        np.arange(target_cycle.size + 1, dtype=np.float64) / target_cycle.size,
        np.concatenate((target_cycle, target_cycle[:1])),
    )
    return shift, error(shift), aligned_target


def cycle_metrics(
    target_cycle: np.ndarray,
    candidate_samples: np.ndarray,
    frequency_hz: float,
    warmup_samples: int,
) -> dict[str, float]:
    period_samples = int(round(SAMPLE_RATE_HZ / frequency_hz))
    cycle_count = min(16, candidate_samples.size // period_samples)
    candidate_cycle = np.mean(
        candidate_samples[: cycle_count * period_samples].reshape(
            cycle_count, period_samples
        ),
        axis=0,
    )
    initial_phase = (warmup_samples % period_samples) / period_samples
    phase = initial_phase + np.arange(period_samples, dtype=np.float64) / period_samples
    reference = np.interp(
        np.mod(phase, 1.0),
        np.arange(target_cycle.size + 1, dtype=np.float64) / target_cycle.size,
        np.concatenate((target_cycle, target_cycle[:1])),
    )
    reference_shape = centered_unit_rms(reference)
    candidate_shape = centered_unit_rms(candidate_cycle)
    phase_shift, aligned_error, _ = best_phase_alignment(
        target_cycle, candidate_cycle, initial_phase
    )
    complex_error, magnitude_error = harmonic_errors(
        reference, candidate_cycle, frequency_hz
    )
    target_rms = float(np.sqrt(np.mean((reference - np.mean(reference)) ** 2)))
    candidate_rms = float(
        np.sqrt(np.mean((candidate_cycle - np.mean(candidate_cycle)) ** 2))
    )
    return {
        "native_time_nrmse": nrmse(reference, candidate_cycle),
        "level_matched_time_nrmse": float(
            np.sqrt(np.mean((candidate_shape - reference_shape) ** 2))
        ),
        "phase_aligned_time_nrmse": aligned_error,
        "complex_harmonic_nrmse": complex_error,
        "magnitude_harmonic_nrmse": magnitude_error,
        "optimal_target_phase_shift_cycles": phase_shift,
        "candidate_to_target_ac_rms_ratio": candidate_rms
        / max(target_rms, np.finfo(np.float64).tiny),
        "candidate_minus_target_dc": float(
            np.mean(candidate_cycle) - np.mean(reference)
        ),
    }


def validation_rows(waveform_profile: dict) -> list[dict]:
    return [row for row in waveform_profile["evaluation"] if row["split"] == "validation"]


def listening_case_map(root: Path) -> dict[tuple[str, int], str]:
    manifest_path = root / "manifest.json"
    if not manifest_path.is_file():
        return {}
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    result = {}
    for case in manifest["cases"]:
        waveform = "square" if case["waveform"] == "pulse" else case["waveform"]
        result[(waveform, int(case["pitch_index"]))] = case["id"]
    return result


def summarize(cases: list[dict]) -> dict:
    summary = {}
    for waveform in WAVEFORMS:
        waveform_cases = [case for case in cases if case["waveform"] == waveform]
        metrics = {}
        for metric in ALL_METRICS:
            medians = {
                variant: float(
                    np.median([case["variants"][variant][metric] for case in waveform_cases])
                )
                for variant in VARIANTS
            }
            winner_counts = {variant: 0 for variant in VARIANTS}
            winner_changes = []
            previous = None
            for case in waveform_cases:
                winner = min(VARIANTS, key=lambda name: case["variants"][name][metric])
                winner_counts[winner] += 1
                if winner != previous:
                    winner_changes.append(
                        {"frequency_hz": case["frequency_hz"], "winner": winner}
                    )
                    previous = winner
            baseline = medians["baseline"]
            metrics[metric] = {
                "median_by_variant": medians,
                "median_relative_improvement_vs_baseline": {
                    variant: (baseline - value)
                    / max(abs(baseline), np.finfo(np.float64).tiny)
                    for variant, value in medians.items()
                    if variant != "baseline"
                },
                "winner_count_by_variant": winner_counts,
                "winner_changes": winner_changes,
            }
        summary[waveform] = {
            "validation_case_count": len(waveform_cases),
            "metrics": metrics,
        }
    return summary


def compare_with_listening(cases: list[dict], root: Path) -> dict | None:
    response_path = root / "responses-template.json"
    answer_path = root / "answer-key.json"
    manifest_path = root / "manifest.json"
    if not all(path.is_file() for path in (response_path, answer_path, manifest_path)):
        return None
    responses = json.loads(response_path.read_text(encoding="utf-8"))
    answers = json.loads(answer_path.read_text(encoding="utf-8"))
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    indexed_cases = {
        (case["waveform"], int(case["pitch_index"])): case for case in cases
    }
    comparisons = []
    for listening_case in manifest["cases"]:
        waveform = (
            "square"
            if listening_case["waveform"] == "pulse"
            else listening_case["waveform"]
        )
        case = indexed_cases[(waveform, int(listening_case["pitch_index"]))]
        identifier = listening_case["id"]
        mapping = {
            choice.removeprefix("choice-"): variant
            for choice, variant in answers["target_match_ranking"][identifier].items()
        }
        letters = responses["target_match_ranking"][identifier][
            "ranking_closest_to_farthest"
        ]
        if sorted(letters) != ["A", "B", "C", "D"]:
            raise ValueError(f"incomplete listening ranking: {identifier}")
        listener_order = [mapping[letter] for letter in letters]
        listener_rank = {
            variant: rank for rank, variant in enumerate(listener_order, start=1)
        }
        metric_results = {}
        for metric in TARGET_METRICS:
            objective_order = sorted(
                VARIANTS, key=lambda name: case["variants"][name][metric]
            )
            objective_values = [case["variants"][name][metric] for name in VARIANTS]
            subjective_ranks = [listener_rank[name] for name in VARIANTS]
            correlation = float(spearmanr(subjective_ranks, objective_values).statistic)
            metric_results[metric] = {
                "objective_order": objective_order,
                "top_choice_agrees": objective_order[0] == listener_order[0],
                "spearman_rank_correlation": correlation,
            }
        comparisons.append(
            {
                "case_id": identifier,
                "waveform": waveform,
                "pitch_index": listening_case["pitch_index"],
                "frequency_hz": listening_case["frequency_hz"],
                "listener_order": listener_order,
                "metrics": metric_results,
            }
        )
    aggregate = {}
    for metric in TARGET_METRICS:
        correlations = [
            case["metrics"][metric]["spearman_rank_correlation"]
            for case in comparisons
        ]
        aggregate[metric] = {
            "top_choice_agreement_count": sum(
                case["metrics"][metric]["top_choice_agrees"] for case in comparisons
            ),
            "case_count": len(comparisons),
            "mean_spearman_rank_correlation": float(np.mean(correlations)),
            "median_spearman_rank_correlation": float(np.median(correlations)),
        }
    return {
        "note": "descriptive comparison with one listener's six diagnostic rankings; not a fitted objective or population statistic",
        "response_sha256": sha256_file(response_path),
        "answer_key_sha256": sha256_file(answer_path),
        "listening_manifest_sha256": sha256_file(manifest_path),
        "aggregate": aggregate,
        "cases": comparisons,
    }


def write_plot(cases: list[dict], path: Path) -> None:
    import matplotlib

    matplotlib.use("Agg")
    matplotlib.rcParams["svg.hashsalt"] = "analog-osc-ablation-v1"
    import matplotlib.pyplot as plt

    plot_metrics = (
        ("level_matched_time_nrmse", "Fixed-phase shape NRMSE"),
        ("phase_aligned_time_nrmse", "Phase-aligned shape NRMSE"),
        ("magnitude_harmonic_nrmse", "Harmonic-magnitude NRMSE"),
        ("spectral_residual_dbc", "Non-harmonic residual (dBc)"),
    )
    labels = {
        "baseline": "Baseline",
        "phase-only": "Phase only",
        "filter-only": "Filter only",
        "phase-plus-filter": "Combined",
    }
    colors = {
        "baseline": "#222222",
        "phase-only": "#0072B2",
        "filter-only": "#D55E00",
        "phase-plus-filter": "#009E73",
    }
    figure, axes = plt.subplots(3, 4, figsize=(18, 10), sharex="row")
    for row_index, waveform in enumerate(WAVEFORMS):
        selected = [case for case in cases if case["waveform"] == waveform]
        frequencies = np.asarray([case["frequency_hz"] for case in selected])
        diagnostic = np.asarray([case["listening_case_id"] is not None for case in selected])
        for column_index, (metric, title) in enumerate(plot_metrics):
            axis = axes[row_index, column_index]
            for variant in VARIANTS:
                values = np.asarray(
                    [case["variants"][variant][metric] for case in selected]
                )
                axis.plot(
                    frequencies,
                    values,
                    marker="o",
                    markersize=3,
                    linewidth=1.3,
                    color=colors[variant],
                    label=labels[variant],
                )
                axis.scatter(
                    frequencies[diagnostic],
                    values[diagnostic],
                    marker="s",
                    s=32,
                    facecolors="none",
                    edgecolors=colors[variant],
                    linewidths=1.2,
                )
            axis.set_xscale("log")
            axis.grid(True, which="both", alpha=0.25)
            if row_index == 0:
                axis.set_title(title)
            if column_index == 0:
                display = "Pulse" if waveform == "square" else waveform.title()
                axis.set_ylabel(f"{display}\nLower is better")
            if row_index == 2:
                axis.set_xlabel("Frequency (Hz)")
    handles, legend_labels = axes[0, 0].get_legend_handles_labels()
    figure.legend(
        handles,
        legend_labels,
        loc="upper center",
        bbox_to_anchor=(0.5, 0.925),
        ncol=4,
        frameon=False,
    )
    figure.suptitle("Korg Monologue phase/filter validation ablations", y=0.995)
    figure.text(
        0.5,
        0.955,
        "Open squares mark the six completed listening cases",
        ha="center",
        va="top",
    )
    figure.tight_layout(rect=(0, 0, 1, 0.85))
    path.parent.mkdir(parents=True, exist_ok=True)
    output_format = path.suffix.removeprefix(".").lower()
    metadata = {"Date": None} if output_format == "svg" else None
    figure.savefig(path, format=output_format, metadata=metadata)
    plt.close(figure)
    if output_format == "svg":
        # Matplotlib emits trailing spaces in multi-line SVG path data, which
        # makes Git's whitespace validation noisy without changing rendering.
        lines = path.read_text(encoding="utf-8").splitlines()
        path.write_text(
            "\n".join(line.rstrip() for line in lines) + "\n", encoding="utf-8"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--profile", type=Path, default=DEFAULT_PROFILE)
    parser.add_argument("--derived-root", type=Path, default=DEFAULT_DERIVED)
    parser.add_argument("--listening-root", type=Path, default=DEFAULT_LISTENING_ROOT)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--plot", type=Path, default=DEFAULT_PLOT)
    args = parser.parse_args()
    if not args.binary.is_file():
        raise FileNotFoundError(f"build the release research example first: {args.binary}")

    profile = json.loads(args.profile.read_text(encoding="utf-8"))
    listening_cases = listening_case_map(args.listening_root)
    cases = []
    for waveform in WAVEFORMS:
        with np.load(args.derived_root / f"{waveform}-cycles-v1.npz") as source:
            target_cycles = np.asarray(source["median_cycles"], dtype=np.float64)
        for row in validation_rows(profile["waveforms"][waveform]):
            frequency_hz = float(row["frequency_hz"])
            period_samples = int(round(SAMPLE_RATE_HZ / frequency_hz))
            if not math.isclose(
                SAMPLE_RATE_HZ / frequency_hz, period_samples, abs_tol=1.0e-6
            ):
                raise ValueError(f"non-integral measured period: {frequency_hz}")
            warmup_samples = max(8_192, period_samples * 32)
            render_samples = max(65_536, period_samples * 16)
            variant_metrics = {}
            for name, variant in VARIANTS.items():
                samples = render_model(
                    args.binary,
                    variant["model"],
                    waveform,
                    frequency_hz,
                    warmup_samples,
                    render_samples,
                    variant["parameters"],
                )
                metrics = cycle_metrics(
                    target_cycles[row["pitch_index"]],
                    samples,
                    frequency_hz,
                    warmup_samples,
                )
                residual = spectral_residual(samples, SAMPLE_RATE_HZ, frequency_hz)
                metrics.update(residual)
                metrics["spectral_residual_dbc"] = metrics.pop("residual_dbc")
                variant_metrics[name] = metrics
            cases.append(
                {
                    "waveform": waveform,
                    "pitch_index": row["pitch_index"],
                    "split": row["split"],
                    "frequency_hz": frequency_hz,
                    "period_samples": period_samples,
                    "warmup_samples": warmup_samples,
                    "render_samples": render_samples,
                    "listening_case_id": listening_cases.get(
                        (waveform, int(row["pitch_index"]))
                    ),
                    "variants": variant_metrics,
                }
            )
            print(f"evaluated {waveform:8s} {frequency_hz:9.3f} Hz", flush=True)

    artifact = {
        "schema_version": 1,
        "metric_revision": 1,
        "experiment_id": "korg-monologue-phase-filter-ablation-curves-v1",
        "model_id": profile["model_id"],
        "profile_id": profile["profile_id"],
        "profile_content_sha256": profile["profile_content_sha256"],
        "git": git_state(),
        "binary_sha256": sha256_file(args.binary),
        "scope": "all validation pitches; test split remains reserved",
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "variants": VARIANTS,
        "metric_notes": {
            "native_time_nrmse": "sample-aligned error including native gain and DC",
            "level_matched_time_nrmse": "sample-aligned centered unit-RMS shape error, corresponding most closely to listening preparation",
            "phase_aligned_time_nrmse": "centered unit-RMS shape error after ignoring one whole-wave phase/time shift",
            "complex_harmonic_nrmse": "centered unit-RMS complex harmonic error; includes relative and absolute phase at the fixed trigger alignment",
            "magnitude_harmonic_nrmse": "centered unit-RMS harmonic-magnitude error; ignores harmonic phase",
            "spectral_residual_dbc": "energy outside guarded intended harmonic bins; more negative is better and this is not a target-distance metric",
        },
        "summary": summarize(cases),
        "listening_comparison": compare_with_listening(cases, args.listening_root),
        "plot": str(args.plot),
        "cases": cases,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2) + "\n", encoding="utf-8")
    write_plot(cases, args.plot)
    print(f"wrote {args.output}")
    print(f"wrote {args.plot}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
