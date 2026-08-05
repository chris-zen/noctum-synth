#!/usr/bin/env python3
"""Generate named and blind level-matched listening files for Plan 04."""

from __future__ import annotations

import argparse
import hashlib
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
DEFAULT_OUTPUT = REPO_ROOT / "target/analog-osc/listening/korg-monologue-phase-filter-v1"
SAMPLE_RATE_HZ = 48_000
MODELS = {
    "baseline": "baseline-v1",
    "candidate": "target-conditioned-phase-filter-v1",
}
WAVEFORMS = ("saw", "triangle", "square")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def ac_rms(samples: np.ndarray) -> float:
    values = np.asarray(samples, dtype=np.float64)
    centered = values - np.mean(values)
    return float(np.sqrt(np.mean(centered * centered)))


def level_match(samples: np.ndarray, target_rms: float, fade_samples: int) -> np.ndarray:
    values = np.asarray(samples, dtype=np.float64).copy()
    rms = ac_rms(values)
    if not math.isfinite(rms) or rms <= np.finfo(np.float64).tiny:
        raise ValueError("cannot level-match a silent or non-finite signal")
    values *= target_rms / rms
    if fade_samples > 0:
        count = min(fade_samples, values.size // 2)
        phase = np.arange(count, dtype=np.float64) / max(count, 1)
        fade = 0.5 - 0.5 * np.cos(np.pi * phase)
        values[:count] *= fade
        values[-count:] *= fade[::-1]
    if np.max(np.abs(values), initial=0.0) >= 1.0:
        raise ValueError("level-matched signal would clip")
    return values.astype(np.float32)


def periodic_target(cycle: np.ndarray, frequency_hz: float, sample_count: int) -> np.ndarray:
    phase = np.arange(sample_count, dtype=np.float64) * frequency_hz / SAMPLE_RATE_HZ
    bins = cycle.size
    return np.interp(
        np.mod(phase, 1.0),
        np.arange(bins + 1, dtype=np.float64) / bins,
        np.concatenate((cycle, cycle[:1])),
    )


def selected_rows(waveform_profile: dict) -> list[tuple[str, dict]]:
    held_out = [row for row in waveform_profile["evaluation"] if row["split"] != "train"]
    return [
        ("low", held_out[0]),
        ("mid", held_out[len(held_out) // 2]),
        ("high", held_out[-1]),
    ]


def render_model(
    binary: Path,
    model: str,
    waveform: str,
    frequency_hz: float,
    warmup_samples: int,
    sample_count: int,
    parameters: dict[str, float] | None = None,
) -> np.ndarray:
    cli_waveform = "pulse" if waveform == "square" else waveform
    with tempfile.TemporaryDirectory(prefix="analog-osc-listening-") as directory:
        root = Path(directory)
        command = [
            str(binary),
            "--model",
            model,
            "--waveform",
            cli_waveform,
            "--sample-rate",
            str(SAMPLE_RATE_HZ),
            "--frequency",
            repr(frequency_hz),
            "--shape",
            "0",
            "--warmup",
            str(warmup_samples),
            "--samples",
            str(sample_count),
            "--output-root",
            str(root),
        ]
        for parameter_id, value in sorted((parameters or {}).items()):
            command.extend(("--param", f"{parameter_id}={value}"))
        subprocess.run(
            command,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        paths = list((root / "renders" / model).glob("*.wav"))
        if len(paths) != 1:
            raise RuntimeError(f"expected one rendered WAV, found {len(paths)}")
        sample_rate, values = wavfile.read(paths[0])
        if sample_rate != SAMPLE_RATE_HZ or values.dtype != np.float32:
            raise RuntimeError(f"unexpected WAV format: {sample_rate} Hz {values.dtype}")
        return values.astype(np.float64)


def case_id(waveform: str, register: str, frequency_hz: float) -> str:
    display_waveform = "pulse" if waveform == "square" else waveform
    frequency = f"{frequency_hz:08.3f}".replace(".", "p")
    return f"{display_waveform}-{register}-{frequency}hz"


def write_audio(path: Path, samples: np.ndarray) -> dict:
    path.parent.mkdir(parents=True, exist_ok=True)
    wavfile.write(path, SAMPLE_RATE_HZ, np.asarray(samples, dtype=np.float32))
    return {
        "path": str(path),
        "sha256": sha256_file(path),
        "ac_rms": ac_rms(samples),
        "peak": float(np.max(np.abs(samples), initial=0.0)),
    }


def git_state() -> dict:
    commit = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    dirty = bool(
        subprocess.run(
            ["git", "status", "--porcelain"],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    return {"commit": commit, "dirty_worktree": dirty}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--profile", type=Path, default=DEFAULT_PROFILE)
    parser.add_argument("--derived-root", type=Path, default=DEFAULT_DERIVED)
    parser.add_argument("--output-root", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--duration-seconds", type=float, default=4.0)
    parser.add_argument("--level-dbfs", type=float, default=-18.0)
    parser.add_argument("--fade-ms", type=float, default=20.0)
    parser.add_argument("--seed", type=int, default=20_260_727)
    args = parser.parse_args()
    if not args.binary.is_file():
        raise FileNotFoundError(f"build the release research example first: {args.binary}")
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
    abx_root = args.output_root / "blind-abx"
    target_match_root = args.output_root / "blind-target-match"
    cases = []
    answers = {"schema_version": 1, "seed": args.seed, "abx": {}, "target_match": {}}

    for waveform in WAVEFORMS:
        with np.load(args.derived_root / f"{waveform}-cycles-v1.npz") as source:
            cycles = np.asarray(source["median_cycles"], dtype=np.float64)
        for register, row in selected_rows(profile["waveforms"][waveform]):
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
            files = {
                name: write_audio(named_root / f"{identifier}-{name}.wav", values)
                for name, values in matched.items()
            }

            ab_order = list(rng.permutation(["baseline", "candidate"]))
            x_source = str(rng.choice(ab_order))
            abx_files = {
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
            target_files = {
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
                    "named_files": files,
                    "blind_abx_files": abx_files,
                    "blind_target_match_files": target_files,
                }
            )
            print(f"generated {identifier}", flush=True)

    manifest = {
        "schema_version": 1,
        "model_id": profile["model_id"],
        "baseline_model_id": MODELS["baseline"],
        "target_id": profile["target_id"],
        "profile_id": profile["profile_id"],
        "profile_content_sha256": profile["profile_content_sha256"],
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
        "blind_note": "listen and record choices before opening answer-key.json",
        "cases": cases,
    }
    args.output_root.mkdir(parents=True, exist_ok=True)
    (args.output_root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (args.output_root / "answer-key.json").write_text(json.dumps(answers, indent=2) + "\n")
    responses = {
        "schema_version": 1,
        "listener": "",
        "playback_chain": "",
        "abx": {
            case["id"]: {"answer": None, "confidence_0_to_1": None, "notes": ""}
            for case in cases
        },
        "target_match": {
            case["id"]: {"closer_choice": None, "confidence_0_to_1": None, "notes": ""}
            for case in cases
        },
    }
    (args.output_root / "responses-template.json").write_text(
        json.dumps(responses, indent=2) + "\n"
    )
    (args.output_root / "README.md").write_text(
        "# Korg Monologue phase/filter listening set v1\n\n"
        "Start with `blind-abx`: decide whether X equals A or B. Then use "
        "`blind-target-match`: compare the reference with choices A and B and "
        "record which is closer in `responses-template.json`. Open "
        "`answer-key.json` only after recording your choices. `named` contains "
        "the unblinded files.\n"
    )
    print(f"wrote {args.output_root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
