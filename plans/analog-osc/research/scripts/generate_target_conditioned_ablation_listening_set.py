#!/usr/bin/env python3
"""Generate the blind Plan 04 phase/filter ablation listening set."""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

import numpy as np
from scipy.io import wavfile

REPO_ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from scripts.generate_target_conditioned_listening_set import (  # noqa: E402
    DEFAULT_BINARY,
    DEFAULT_DERIVED,
    DEFAULT_PROFILE,
    SAMPLE_RATE_HZ,
    WAVEFORMS,
    case_id,
    git_state,
    level_match,
    periodic_target,
    render_model,
    selected_rows,
    sha256_file,
    write_audio,
)


DEFAULT_OUTPUT = (
    REPO_ROOT
    / "target/analog-osc/listening/korg-monologue-phase-filter-ablation-v1"
)
EXPERIMENT_ID = "korg-monologue-phase-filter-ablation-v1"
VARIANTS = {
    "baseline": {
        "model": "baseline-v1",
        "parameters": {},
    },
    "phase-only": {
        "model": "target-conditioned-phase-filter-v1",
        "parameters": {"phase-amount": 1.0, "filter-amount": 0.0},
    },
    "filter-only": {
        "model": "target-conditioned-phase-filter-v1",
        "parameters": {"phase-amount": 0.0, "filter-amount": 1.0},
    },
    "phase-plus-filter": {
        "model": "target-conditioned-phase-filter-v1",
        "parameters": {"phase-amount": 1.0, "filter-amount": 1.0},
    },
}
CHOICE_LABELS = ("A", "B", "C", "D")


def diagnostic_rows(waveform_profile: dict) -> list[tuple[str, dict]]:
    """Choose two unused validation pitches without consulting listening results."""
    prior_pitch_indices = {
        row["pitch_index"] for _, row in selected_rows(waveform_profile)
    }
    eligible = [
        row
        for row in waveform_profile["evaluation"]
        if row["split"] == "validation"
        and row["pitch_index"] not in prior_pitch_indices
    ]
    if len(eligible) < 4:
        raise ValueError("at least four unused validation cases are required")
    positions = (len(eligible) // 3, (2 * len(eligible)) // 3)
    return [("lower", eligible[positions[0]]), ("upper", eligible[positions[1]])]


def randomized_choices(rng: np.random.Generator) -> dict[str, str]:
    order = list(rng.permutation(list(VARIANTS)))
    return {
        f"choice-{label}": str(variant)
        for label, variant in zip(CHOICE_LABELS, order, strict=True)
    }


def validate_package(output_root: Path) -> dict[str, float | int]:
    manifest = json.loads((output_root / "manifest.json").read_text(encoding="utf-8"))
    answers = json.loads((output_root / "answer-key.json").read_text(encoding="utf-8"))
    responses = json.loads(
        (output_root / "responses-template.json").read_text(encoding="utf-8")
    )
    if len(manifest["cases"]) != 6:
        raise RuntimeError("ablation package must contain exactly six cases")
    if set(answers["target_match_ranking"]) != {
        case["id"] for case in manifest["cases"]
    }:
        raise RuntimeError("answer-key cases do not match the manifest")
    if set(responses["target_match_ranking"]) != set(
        answers["target_match_ranking"]
    ):
        raise RuntimeError("response cases do not match the answer key")

    file_count = 0
    rms_values = []
    maximum_peak = 0.0
    for case in manifest["cases"]:
        if case["split"] != "validation":
            raise RuntimeError(f"non-validation diagnostic case: {case['id']}")
        named = case["named_files"]
        blind = case["blind_target_ranking_files"]
        choices = answers["target_match_ranking"][case["id"]]
        if set(choices.values()) != set(VARIANTS):
            raise RuntimeError(f"invalid choice permutation: {case['id']}")
        if blind["reference"]["sha256"] != named["measured-target"]["sha256"]:
            raise RuntimeError(f"reference mismatch: {case['id']}")
        for choice, variant_name in choices.items():
            if blind[choice]["sha256"] != named[variant_name]["sha256"]:
                raise RuntimeError(f"blind choice mismatch: {case['id']} {choice}")
        for record in (*named.values(), *blind.values()):
            path = Path(record["path"])
            if sha256_file(path) != record["sha256"]:
                raise RuntimeError(f"hash mismatch: {path}")
            sample_rate, samples = wavfile.read(path)
            if sample_rate != SAMPLE_RATE_HZ or samples.dtype != np.float32:
                raise RuntimeError(f"unexpected WAV format: {path}")
            if samples.ndim != 1 or not np.all(np.isfinite(samples)):
                raise RuntimeError(f"invalid mono samples: {path}")
            file_count += 1
            rms_values.append(float(record["ac_rms"]))
            maximum_peak = max(maximum_peak, float(record["peak"]))
    return {
        "case_count": len(manifest["cases"]),
        "file_count": file_count,
        "minimum_ac_rms": min(rms_values),
        "maximum_ac_rms": max(rms_values),
        "maximum_peak": maximum_peak,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--profile", type=Path, default=DEFAULT_PROFILE)
    parser.add_argument("--derived-root", type=Path, default=DEFAULT_DERIVED)
    parser.add_argument("--output-root", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--duration-seconds", type=float, default=4.0)
    parser.add_argument("--level-dbfs", type=float, default=-18.0)
    parser.add_argument("--fade-ms", type=float, default=20.0)
    parser.add_argument("--seed", type=int, default=20_260_728)
    args = parser.parse_args()

    if not args.binary.is_file():
        raise FileNotFoundError(f"build the release research example first: {args.binary}")
    if args.output_root.exists() and any(args.output_root.iterdir()):
        raise FileExistsError(
            f"refusing to replace an existing listening set: {args.output_root}"
        )
    if args.duration_seconds < 1.0 or args.duration_seconds > 30.0:
        raise ValueError("--duration-seconds must be between 1 and 30")
    if args.level_dbfs > -6.0 or args.level_dbfs < -48.0:
        raise ValueError("--level-dbfs must be between -48 and -6")
    if args.fade_ms < 0.0 or args.fade_ms > 500.0:
        raise ValueError("--fade-ms must be between 0 and 500")

    profile = json.loads(args.profile.read_text(encoding="utf-8"))
    sample_count = round(args.duration_seconds * SAMPLE_RATE_HZ)
    fade_samples = round(args.fade_ms * SAMPLE_RATE_HZ / 1_000.0)
    target_rms = 10.0 ** (args.level_dbfs / 20.0)
    rng = np.random.default_rng(args.seed)
    named_root = args.output_root / "named"
    blind_root = args.output_root / "blind-target-ranking"
    cases = []
    answers = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "seed": args.seed,
        "target_match_ranking": {},
    }

    for waveform in WAVEFORMS:
        with np.load(args.derived_root / f"{waveform}-cycles-v1.npz") as source:
            cycles = np.asarray(source["median_cycles"], dtype=np.float64)
        for register, row in diagnostic_rows(profile["waveforms"][waveform]):
            frequency_hz = float(row["frequency_hz"])
            identifier = case_id(waveform, register, frequency_hz)
            warmup = max(8_192, math.ceil(SAMPLE_RATE_HZ / frequency_hz) * 32)
            raw = {
                name: render_model(
                    args.binary,
                    variant["model"],
                    waveform,
                    frequency_hz,
                    warmup,
                    sample_count,
                    variant["parameters"],
                )
                for name, variant in VARIANTS.items()
            }
            raw["measured-target"] = periodic_target(
                cycles[row["pitch_index"]], frequency_hz, sample_count
            )
            matched = {
                name: level_match(values, target_rms, fade_samples)
                for name, values in raw.items()
            }
            named_files = {
                name: write_audio(named_root / f"{identifier}-{name}.wav", values)
                for name, values in matched.items()
            }

            choices = randomized_choices(rng)
            blind_files = {
                "reference": write_audio(
                    blind_root / identifier / "reference.wav",
                    matched["measured-target"],
                )
            }
            for choice, variant_name in choices.items():
                blind_files[choice] = write_audio(
                    blind_root / identifier / f"{choice}.wav",
                    matched[variant_name],
                )
            answers["target_match_ranking"][identifier] = choices
            cases.append(
                {
                    "id": identifier,
                    "waveform": "pulse" if waveform == "square" else waveform,
                    "register": register,
                    "pitch_index": row["pitch_index"],
                    "split": row["split"],
                    "frequency_hz": frequency_hz,
                    "warmup_samples": warmup,
                    "named_files": named_files,
                    "blind_target_ranking_files": blind_files,
                }
            )
            print(f"generated {identifier}", flush=True)

    manifest = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "purpose": "diagnose phase and filter contributions after the combined model failed its first perceptual target-match gate",
        "diagnostic_only": True,
        "model_id": profile["model_id"],
        "target_id": profile["target_id"],
        "profile_id": profile["profile_id"],
        "profile_content_sha256": profile["profile_content_sha256"],
        "variants": VARIANTS,
        "selection_note": "two unused validation pitches per waveform selected deterministically without consulting prior listening choices; test pitches remain reserved for a fresh acceptance set",
        "ablation_note": "phase-only and filter-only reuse jointly fitted coefficients; this identifies contributions but is not an independently refitted optimum",
        "git": git_state(),
        "binary_sha256": sha256_file(args.binary),
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "duration_seconds": args.duration_seconds,
        "level_match": {
            "method": "centered RMS (DC excluded from measurement)",
            "target_dbfs": args.level_dbfs,
            "target_linear_rms": target_rms,
            "fade_ms": args.fade_ms,
            "normalization_note": "each source is independently gain-matched; waveform DC is preserved",
        },
        "target_reference_note": "periodic interpolation of the independently normalized measured median cycle, not a continuous raw hardware recording",
        "blind_note": "rank all four choices before opening answer-key.json",
        "cases": cases,
    }
    args.output_root.mkdir(parents=True, exist_ok=True)
    (args.output_root / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    (args.output_root / "answer-key.json").write_text(
        json.dumps(answers, indent=2) + "\n", encoding="utf-8"
    )
    responses = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "listener": "",
        "playback_chain": "",
        "target_match_ranking": {
            case["id"]: {
                "ranking_closest_to_farthest": [],
                "confidence_0_to_1": None,
                "notes": "",
            }
            for case in cases
        },
    }
    (args.output_root / "responses-template.json").write_text(
        json.dumps(responses, indent=2) + "\n", encoding="utf-8"
    )
    (args.output_root / "README.md").write_text(
        "# Korg Monologue phase/filter ablation listening set v1\n\n"
        "Use `blind-target-ranking` only. For each case, listen to "
        "`reference.wav`, then rank choices A through D from closest to "
        "farthest in `responses-template.json`. Enter labels without the "
        "`choice-` prefix, for example `[\"C\", \"A\", \"D\", \"B\"]`. "
        "Use every label exactly once. Do not inspect `answer-key.json` or "
        "`named` until all six rankings are saved. This is a diagnostic of "
        "jointly fitted components, not a final acceptance test.\n",
        encoding="utf-8",
    )
    validation = validate_package(args.output_root)
    print(f"validated {validation}")
    print(f"wrote {args.output_root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
