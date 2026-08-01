#!/usr/bin/env python3
"""Generate the untouched-test blind listening gate for Plan 04 v2."""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

import numpy as np

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

from generate_target_conditioned_listening_set import (
    REPO_ROOT,
    SAMPLE_RATE_HZ,
    WAVEFORMS,
    case_id,
    git_state,
    level_match,
    periodic_target,
    render_model,
    sha256_file,
    write_audio,
)


DEFAULT_BINARY = REPO_ROOT / "target/release/analog_osc_research"
DEFAULT_PROFILE = REPO_ROOT / (
    "plans/analog-osc/research/profiles/korg-monologue-phase-filter-v2.json"
)
DEFAULT_DERIVED = REPO_ROOT / "target/analog-osc/reference/korg-monologue-v1/derived"
DEFAULT_OUTPUT = REPO_ROOT / (
    "target/analog-osc/listening/korg-monologue-phase-filter-v2"
)
MODELS = {
    "baseline": "baseline-v1",
    "candidate": "target-conditioned-phase-filter-v2",
}
# Pitch 71 appeared in the v1 blind package. The validation split and all
# revealed v1 ablation pitches are also excluded by selecting test rows only.
PREVIOUSLY_HEARD_TEST_PITCH_INDICES = frozenset({71})


def selected_fresh_test_rows(waveform_profile: dict) -> list[tuple[str, dict]]:
    available = [
        row
        for row in waveform_profile["evaluation"]
        if row["split"] == "test"
        and row["pitch_index"] not in PREVIOUSLY_HEARD_TEST_PITCH_INDICES
    ]
    if len(available) < 3:
        raise ValueError("fewer than three fresh test rows are available")
    return [
        ("low", available[0]),
        ("mid", available[len(available) // 2]),
        ("high", available[-1]),
    ]


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
    if not 1.0 <= args.duration_seconds <= 30.0:
        raise ValueError("--duration-seconds must be between 1 and 30")
    if not -48.0 <= args.level_dbfs <= -6.0:
        raise ValueError("--level-dbfs must be between -48 and -6")
    if not 0.0 <= args.fade_ms <= 500.0:
        raise ValueError("--fade-ms must be between 0 and 500")

    profile = json.loads(args.profile.read_text(encoding="utf-8"))
    if profile["model_id"] != MODELS["candidate"]:
        raise ValueError(
            f"expected profile for {MODELS['candidate']}, got {profile['model_id']}"
        )
    sample_count = round(args.duration_seconds * SAMPLE_RATE_HZ)
    fade_samples = round(args.fade_ms * SAMPLE_RATE_HZ / 1_000.0)
    target_rms = 10.0 ** (args.level_dbfs / 20.0)
    rng = np.random.default_rng(args.seed)
    named_root = args.output_root / "named"
    abx_root = args.output_root / "blind-abx"
    target_match_root = args.output_root / "blind-target-match"
    cases = []
    answers = {"schema_version": 2, "seed": args.seed, "abx": {}, "target_match": {}}

    for waveform in WAVEFORMS:
        with np.load(args.derived_root / f"{waveform}-cycles-v1.npz") as source:
            cycles = np.asarray(source["median_cycles"], dtype=np.float64)
        rows = selected_fresh_test_rows(profile["waveforms"][waveform])
        for register, row in rows:
            frequency_hz = float(row["frequency_hz"])
            identifier = case_id(waveform, register, frequency_hz)
            warmup = max(8_192, math.ceil(SAMPLE_RATE_HZ / frequency_hz) * 32)
            raw = {
                name: render_model(
                    args.binary,
                    model,
                    waveform,
                    frequency_hz,
                    warmup,
                    sample_count,
                )
                for name, model in MODELS.items()
            }
            raw["measured_target"] = periodic_target(
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

            ab_order = list(rng.permutation(["baseline", "candidate"]))
            x_source = str(rng.choice(ab_order))
            blind_abx_files = {
                "A": write_audio(abx_root / identifier / "A.wav", matched[ab_order[0]]),
                "B": write_audio(abx_root / identifier / "B.wav", matched[ab_order[1]]),
                "X": write_audio(abx_root / identifier / "X.wav", matched[x_source]),
            }
            answers["abx"][identifier] = {
                "A": ab_order[0],
                "B": ab_order[1],
                "X": x_source,
                "correct": "A" if x_source == ab_order[0] else "B",
            }

            choice_order = list(rng.permutation(["baseline", "candidate"]))
            blind_target_files = {
                "reference": write_audio(
                    target_match_root / identifier / "reference.wav",
                    matched["measured_target"],
                ),
                "choice-A": write_audio(
                    target_match_root / identifier / "choice-A.wav",
                    matched[choice_order[0]],
                ),
                "choice-B": write_audio(
                    target_match_root / identifier / "choice-B.wav",
                    matched[choice_order[1]],
                ),
            }
            answers["target_match"][identifier] = {
                "choice-A": choice_order[0],
                "choice-B": choice_order[1],
            }
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
                    "blind_abx_files": blind_abx_files,
                    "blind_target_match_files": blind_target_files,
                }
            )
            print(f"generated {identifier}", flush=True)

    manifest = {
        "schema_version": 2,
        "purpose": "fresh untouched-test acceptance gate for Plan 04 v2",
        "model_id": profile["model_id"],
        "baseline_model_id": MODELS["baseline"],
        "target_id": profile["target_id"],
        "profile_id": profile["profile_id"],
        "profile_content_sha256": profile["profile_content_sha256"],
        "git": git_state(),
        "binary_sha256": sha256_file(args.binary),
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "duration_seconds": args.duration_seconds,
        "selection": {
            "split": "test only",
            "registers": "lowest, middle, and highest available fresh test row",
            "excluded_previously_heard_test_pitch_indices": sorted(
                PREVIOUSLY_HEARD_TEST_PITCH_INDICES
            ),
            "coefficient_tuning_after_reveal_forbidden": True,
        },
        "level_match": {
            "method": "centered RMS (DC excluded from measurement)",
            "target_dbfs": args.level_dbfs,
            "target_linear_rms": target_rms,
            "fade_ms": args.fade_ms,
            "normalization_note": (
                "each source is independently gain-matched; waveform DC is preserved"
            ),
        },
        "target_reference_note": (
            "periodic interpolation of the independently normalized measured median "
            "cycle, not a continuous raw hardware recording"
        ),
        "blind_note": "record choices before opening answer-key.json or named files",
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
        "schema_version": 2,
        "listener": "",
        "playback_chain": "",
        "abx": {
            case["id"]: {"answer": None, "confidence_0_to_1": None, "notes": ""}
            for case in cases
        },
        "target_match": {
            case["id"]: {
                "closer_choice": None,
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
        "# Korg Monologue phase/filter listening set v2\n\n"
        "This is the fresh test-split acceptance gate. Do not open `named` or "
        "`answer-key.json` yet. First complete `blind-abx`, recording whether X "
        "equals A or B. Then complete `blind-target-match`, recording whether "
        "choice A or B is closer to the reference. Save all answers in "
        "`responses-template.json`, then tell Codex you are done.\n",
        encoding="utf-8",
    )
    print(f"wrote {args.output_root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
