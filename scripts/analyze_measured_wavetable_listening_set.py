#!/usr/bin/env python3
"""Decode a completed measured-wavetable blind listening response."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

from analyze_target_conditioned_listening_set_v2 import (
    confidence,
    normalized_choice,
    summarize,
)
from generate_target_conditioned_listening_set import REPO_ROOT, sha256_file


DEFAULT_ROOT = REPO_ROOT / (
    "target/analog-osc/listening/korg-monologue-measured-wavetable-v1"
)
DEFAULT_OUTPUT = REPO_ROOT / (
    "plans/analog-osc/research/reports/"
    "korg-monologue-measured-wavetable-listening-v1.json"
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--listening-root", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()

    response_path = args.listening_root / "responses-template.json"
    answer_path = args.listening_root / "answer-key.json"
    manifest_path = args.listening_root / "manifest.json"
    responses = json.loads(response_path.read_text(encoding="utf-8"))
    answers = json.loads(answer_path.read_text(encoding="utf-8"))
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    rows = []
    for case in manifest["cases"]:
        identifier = case["id"]
        abx_response = responses["abx"][identifier]
        target_response = responses["target_match"][identifier]
        abx_answer = normalized_choice(abx_response["answer"], "")
        target_choice = normalized_choice(target_response["closer_choice"], "choice-")
        target_source = answers["target_match"][identifier][f"choice-{target_choice}"]
        rows.append(
            {
                "id": identifier,
                "waveform": case["waveform"],
                "register": case["register"],
                "pitch_index": case["pitch_index"],
                "frequency_hz": case["frequency_hz"],
                "abx_answer": abx_answer,
                "abx_correct": abx_answer == answers["abx"][identifier]["correct"],
                "abx_confidence": confidence(abx_response["confidence_0_to_1"]),
                "abx_notes": str(abx_response.get("notes", "")),
                "target_match_choice": target_choice,
                "target_match_source": target_source,
                "target_match_confidence": confidence(
                    target_response["confidence_0_to_1"]
                ),
                "target_match_notes": str(target_response.get("notes", "")),
            }
        )

    artifact = {
        "schema_version": 1,
        "listening_set": manifest["model_id"],
        "seed": answers["seed"],
        "listener": responses.get("listener", ""),
        "playback_chain": responses.get("playback_chain", ""),
        "response_sha256": sha256_file(response_path),
        "answer_key_sha256": sha256_file(answer_path),
        "manifest_sha256": sha256_file(manifest_path),
        "summary": summarize(rows),
        "cases": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
