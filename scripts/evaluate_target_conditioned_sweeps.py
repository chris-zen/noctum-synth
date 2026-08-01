#!/usr/bin/env python3
"""Run static log-pitch residual sweeps for the target-conditioned oscillator."""

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
from scipy.signal.windows import blackmanharris


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BINARY = REPO_ROOT / "target/release/analog_osc_research"
DEFAULT_PROFILE = REPO_ROOT / "plans/analog-osc/research/profiles/korg-monologue-phase-filter-v1.json"
DEFAULT_OUTPUT = REPO_ROOT / "plans/analog-osc/research/reports/korg-monologue-phase-filter-sweeps-v1.json"
BASELINE_MODEL = "baseline-v1"
DEFAULT_CANDIDATE_MODEL = "target-conditioned-phase-filter-v1"
WAVEFORMS = ("saw", "triangle", "square")


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def spectral_residual(
    samples: np.ndarray,
    sample_rate_hz: float,
    fundamental_hz: float,
    guard_bins: int = 10,
) -> dict[str, float | int]:
    """Measure energy outside guarded intended harmonic frequencies.

    For these deterministic oscillators, out-of-mask energy is a useful alias
    and implementation-residual indicator. It is intentionally not presented
    as a pure alias measurement for noisy or drifting recorded hardware.
    """
    fft_size = 1 << (len(samples).bit_length() - 1)
    values = np.asarray(samples[:fft_size], dtype=np.float64)
    values -= np.mean(values)
    window = blackmanharris(fft_size, sym=False)
    spectrum = np.fft.rfft(values * window)
    power = np.abs(spectrum) ** 2
    bin_hz = sample_rate_hz / fft_size
    legal = np.zeros(power.size, dtype=bool)
    legal[: min(guard_bins + 1, legal.size)] = True
    harmonic = 1
    fundamental_power = np.finfo(np.float64).tiny
    while harmonic * fundamental_hz < sample_rate_hz * 0.5:
        center = harmonic * fundamental_hz / bin_hz
        lower = max(0, math.floor(center) - guard_bins)
        upper = min(power.size - 1, math.ceil(center) + guard_bins)
        legal[lower : upper + 1] = True
        if harmonic == 1:
            fundamental_power = float(np.sum(power[lower : upper + 1]))
        harmonic += 1
    legal_power = float(np.sum(power[legal]))
    residual_bins = power[~legal]
    residual_power = float(np.sum(residual_bins))
    worst_residual = float(np.max(residual_bins, initial=0.0))
    tiny = np.finfo(np.float64).tiny
    return {
        "fft_size": fft_size,
        "guard_bins": guard_bins,
        "legal_harmonic_count": harmonic - 1,
        "residual_dbc": float(10.0 * np.log10(max(residual_power / max(legal_power, tiny), tiny))),
        "worst_residual_component_dbc": float(
            10.0 * np.log10(max(worst_residual / max(fundamental_power, tiny), tiny))
        ),
    }


def selected_frequencies(waveform_profile: dict, count: int) -> list[float]:
    source_rows = waveform_profile.get("evaluation", waveform_profile.get("cases"))
    if source_rows is None:
        raise ValueError("waveform profile has neither evaluation nor cases")
    rows = [row for row in source_rows if row["split"] != "train"]
    positions = np.linspace(0, len(rows) - 1, count)
    indices = sorted({int(round(position)) for position in positions})
    return [float(rows[index]["frequency_hz"]) for index in indices]


def render(
    binary: Path,
    model: str,
    waveform: str,
    frequency_hz: float,
    sample_rate_hz: int,
    warmup_samples: int,
    render_samples: int,
    shape: float = 0.0,
) -> np.ndarray:
    cli_waveform = "pulse" if waveform == "square" else waveform
    with tempfile.TemporaryDirectory(prefix="analog-osc-sweep-") as directory:
        root = Path(directory)
        subprocess.run(
            [
                str(binary),
                "--model",
                model,
                "--waveform",
                cli_waveform,
                "--sample-rate",
                str(sample_rate_hz),
                "--frequency",
                repr(frequency_hz),
                "--shape",
                repr(shape),
                "--warmup",
                str(warmup_samples),
                "--samples",
                str(render_samples),
                "--output-root",
                str(root),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        paths = list((root / "renders" / model).glob("*.wav"))
        if len(paths) != 1:
            raise RuntimeError(f"expected one rendered WAV, found {len(paths)}")
        actual_rate, values = wavfile.read(paths[0])
        if actual_rate != sample_rate_hz or values.dtype != np.float32:
            raise RuntimeError(f"unexpected WAV format: {actual_rate} Hz {values.dtype}")
        return values.astype(np.float64)


def summarize(cases: list[dict]) -> dict:
    result = {}
    waveforms = list(dict.fromkeys(row["waveform"] for row in cases))
    for sample_rate_hz in sorted({row["sample_rate_hz"] for row in cases}):
        rate_result = {}
        for waveform in waveforms:
            selected = [
                row
                for row in cases
                if row["sample_rate_hz"] == sample_rate_hz and row["waveform"] == waveform
            ]
            candidate = np.asarray(
                [row["candidate_residual_dbc"] for row in selected], dtype=np.float64
            )
            baseline = np.asarray(
                [row["baseline_residual_dbc"] for row in selected], dtype=np.float64
            )
            delta = candidate - baseline
            material_failures = (delta > 3.0) & (candidate > -70.0)
            rate_result[waveform] = {
                "case_count": len(selected),
                "baseline_residual_dbc_median": float(np.median(baseline)),
                "candidate_residual_dbc_median": float(np.median(candidate)),
                "candidate_minus_baseline_dbc_median": float(np.median(delta)),
                "candidate_minus_baseline_dbc_maximum": float(np.max(delta)),
                "cases_regressing_more_than_3_db": int(np.sum(delta > 3.0)),
                "candidate_residual_dbc_worst": float(np.max(candidate)),
                "material_gate_failures": int(np.sum(material_failures)),
            }
        result[str(sample_rate_hz)] = rate_result
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--profile", type=Path, default=DEFAULT_PROFILE)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--candidate-model", default=DEFAULT_CANDIDATE_MODEL)
    parser.add_argument("--sample-rates", default="48000,96000")
    parser.add_argument("--frequencies-per-waveform", type=int, default=7)
    parser.add_argument("--fft-size", type=int, default=65_536)
    parser.add_argument("--shapes", default="0")
    parser.add_argument("--waveforms", default=",".join(WAVEFORMS))
    args = parser.parse_args()
    if not args.binary.is_file():
        raise FileNotFoundError(f"build the release research example first: {args.binary}")
    if args.frequencies_per_waveform < 3:
        raise ValueError("--frequencies-per-waveform must be at least 3")
    if args.fft_size < 16_384 or args.fft_size & (args.fft_size - 1):
        raise ValueError("--fft-size must be a power of two of at least 16384")
    sample_rates = [int(value) for value in args.sample_rates.split(",")]
    shapes = [float(value) for value in args.shapes.split(",")]
    waveforms = [value.strip() for value in args.waveforms.split(",")]
    if any(rate < 32_000 for rate in sample_rates):
        raise ValueError("sample rates below 32 kHz are outside this desktop sweep")
    if any(not math.isfinite(shape) or not 0.0 <= shape <= 1.0 for shape in shapes):
        raise ValueError("--shapes values must be finite and within [0, 1]")
    if any(waveform not in (*WAVEFORMS, "saw-triangle") for waveform in waveforms):
        raise ValueError("--waveforms contains an unsupported waveform")

    profile = json.loads(args.profile.read_text(encoding="utf-8"))
    cases = []
    for sample_rate_hz in sample_rates:
        for waveform in waveforms:
            profile_waveform = "triangle" if waveform == "saw-triangle" else waveform
            frequencies = selected_frequencies(
                profile["waveforms"][profile_waveform], args.frequencies_per_waveform
            )
            for frequency_hz in frequencies:
                for shape in shapes:
                    warmup = max(8_192, math.ceil(sample_rate_hz / frequency_hz) * 32)
                    metrics = {}
                    for model in (BASELINE_MODEL, args.candidate_model):
                        samples = render(
                            args.binary,
                            model,
                            waveform,
                            frequency_hz,
                            sample_rate_hz,
                            warmup,
                            args.fft_size,
                            shape,
                        )
                        metrics[model] = spectral_residual(samples, sample_rate_hz, frequency_hz)
                    cases.append(
                        {
                            "waveform": waveform,
                            "shape": shape,
                            "sample_rate_hz": sample_rate_hz,
                            "frequency_hz": frequency_hz,
                            "warmup_samples": warmup,
                            "baseline_residual_dbc": metrics[BASELINE_MODEL]["residual_dbc"],
                            "candidate_residual_dbc": metrics[args.candidate_model]["residual_dbc"],
                            "candidate_minus_baseline_dbc": metrics[args.candidate_model]["residual_dbc"]
                            - metrics[BASELINE_MODEL]["residual_dbc"],
                            "baseline_worst_component_dbc": metrics[BASELINE_MODEL][
                                "worst_residual_component_dbc"
                            ],
                            "candidate_worst_component_dbc": metrics[args.candidate_model][
                                "worst_residual_component_dbc"
                            ],
                        }
                    )
                    print(
                        f"{sample_rate_hz:6d} Hz {waveform:8s} shape={shape:.3f} "
                        f"{frequency_hz:9.3f} Hz "
                        f"baseline={metrics[BASELINE_MODEL]['residual_dbc']:7.2f} dBc "
                        f"candidate={metrics[args.candidate_model]['residual_dbc']:7.2f} dBc",
                        flush=True,
                    )

    artifact = {
        "schema_version": 1,
        "metric_revision": 1,
        "model_id": profile.get("model_id", args.candidate_model),
        "baseline_model_id": BASELINE_MODEL,
        "candidate_model_id": args.candidate_model,
        "profile_id": profile.get("profile_id", profile.get("experiment_id", "unknown")),
        "profile_content_sha256": profile.get(
            "profile_content_sha256", sha256_file(args.profile)
        ),
        "method": {
            "window": "4-term Blackman-Harris",
            "fft_size": args.fft_size,
            "shapes": shapes,
            "legal_mask": "intended harmonics below Nyquist with +/-10-bin guards",
            "interpretation": "deterministic non-harmonic implementation residual; lower dBc is better",
            "retention_threshold": "candidate may not exceed baseline by more than 3 dB",
            "material_warning_threshold": "a >3 dB regression is material here only when candidate residual also exceeds -70 dBc",
        },
        "summary": summarize(cases),
        "cases": cases,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
