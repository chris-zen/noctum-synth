#!/usr/bin/env python3
"""Download, validate, inspect, and extract analog-oscillator references."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import pickle
import sys
import urllib.request
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[4]
DEFAULT_MANIFEST = (
    REPO_ROOT / "plans/analog-osc/research/targets/korg-monologue-v1.json"
)
DEFAULT_ROOT = REPO_ROOT / "target/analog-osc/reference/korg-monologue-v1"


def load_manifest(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as source:
        manifest = json.load(source)
    required = {"schema_version", "target_id", "files", "capture"}
    missing = sorted(required.difference(manifest))
    if missing:
        raise ValueError(f"manifest is missing fields: {', '.join(missing)}")
    return manifest


def md5_file(path: Path) -> str:
    digest = hashlib.md5(usedforsecurity=False)
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def selected_waveforms(value: str, manifest: dict[str, Any]) -> list[str]:
    if value == "all":
        return list(manifest["files"])
    if value not in manifest["files"]:
        raise ValueError(f"unknown waveform {value!r}")
    return [value]


def source_path(root: Path, entry: dict[str, Any]) -> Path:
    return root / "sources" / entry["name"]


def verify_source(path: Path, entry: dict[str, Any]) -> str:
    if not path.is_file():
        raise FileNotFoundError(f"missing source file: {path}")
    actual = md5_file(path)
    expected = entry["md5"].lower()
    if actual != expected:
        raise ValueError(
            f"checksum mismatch for {path.name}: expected {expected}, got {actual}"
        )
    return actual


def download_file(url: str, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(destination.suffix + ".partial")
    request = urllib.request.Request(url, headers={"User-Agent": "NoctumResearch/1"})
    try:
        with urllib.request.urlopen(request) as response, temporary.open("wb") as output:
            total = int(response.headers.get("Content-Length", "0"))
            copied = 0
            while chunk := response.read(1024 * 1024):
                output.write(chunk)
                copied += len(chunk)
                if total:
                    print(
                        f"\r{destination.name}: {copied / total:6.1%}",
                        end="",
                        flush=True,
                    )
            if total:
                print()
        temporary.replace(destination)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def download(manifest: dict[str, Any], root: Path, waveform: str) -> None:
    for name in selected_waveforms(waveform, manifest):
        entry = manifest["files"][name]
        destination = source_path(root, entry)
        if destination.is_file():
            try:
                verify_source(destination, entry)
                print(f"{name}: already downloaded and verified")
                continue
            except ValueError:
                destination.unlink()
        print(f"Downloading {name} from {entry['url']}")
        download_file(entry["url"], destination)
        verify_source(destination, entry)
        print(f"{name}: checksum verified")


def require_numpy():
    try:
        import numpy as np
    except ImportError as error:
        raise RuntimeError("inspection and extraction require NumPy") from error
    return np


def load_verified_pickle(path: Path, entry: dict[str, Any]) -> dict[str, Any]:
    # Pickle can execute code. Only deserialize after matching the checksum
    # pinned from the publisher's Zenodo record.
    verify_source(path, entry)
    with path.open("rb") as source:
        value = pickle.load(source)
    if not isinstance(value, dict) or "y" not in value or "z" not in value:
        raise ValueError("expected a pickle dictionary containing 'y' and 'z'")
    return value


def dataset_arrays(path: Path, entry: dict[str, Any]):
    np = require_numpy()
    value = load_verified_pickle(path, entry)
    audio = np.asarray(value["y"], dtype=np.float32)
    conditioning = np.asarray(value["z"], dtype=np.float32)
    if audio.ndim != 2:
        raise ValueError(f"expected y to have two dimensions, got {audio.shape}")
    if conditioning.shape[0] != audio.shape[0]:
        raise ValueError(
            f"conditioning/audio pitch counts differ: {conditioning.shape} vs {audio.shape}"
        )
    expected_samples = entry.get("expected_samples_per_pitch")
    if expected_samples is not None and audio.shape[1] != expected_samples:
        raise ValueError(
            f"unexpected samples per pitch: expected {expected_samples}, got {audio.shape[1]}"
        )
    if not np.isfinite(audio).all() or not np.isfinite(conditioning).all():
        raise ValueError("dataset contains NaN or infinity")
    return np, audio, conditioning


def inspect(manifest: dict[str, Any], root: Path, waveform: str) -> None:
    for name in selected_waveforms(waveform, manifest):
        entry = manifest["files"][name]
        path = source_path(root, entry)
        np, audio, conditioning = dataset_arrays(path, entry)
        summary = {
            "waveform": name,
            "source": str(path),
            "md5": entry["md5"],
            "audio_shape": list(audio.shape),
            "audio_dtype": str(audio.dtype),
            "audio_min": float(np.min(audio)),
            "audio_max": float(np.max(audio)),
            "conditioning_shape": list(conditioning.shape),
            "conditioning_min": float(np.min(conditioning)),
            "conditioning_max": float(np.max(conditioning)),
            "conditioning_first": float(np.median(conditioning[0])),
            "conditioning_last": float(np.median(conditioning[-1])),
        }
        print(json.dumps(summary, indent=2))


def upward_crossings(np, samples):
    centered = samples - np.median(samples)
    indices = np.flatnonzero((centered[:-1] <= 0.0) & (centered[1:] > 0.0))
    if indices.size < 3:
        raise ValueError("could not find enough upward crossings")
    left = centered[indices]
    right = centered[indices + 1]
    fraction = -left / np.maximum(right - left, np.finfo(np.float32).eps)
    return indices.astype(np.float64) + np.clip(fraction, 0.0, 1.0)


def estimate_frequency(np, samples, sample_rate: float, expected_frequency: float | None):
    centered = samples.astype(np.float64) - np.mean(samples, dtype=np.float64)
    fft_size = 1 << int(math.log2(centered.size))
    centered = centered[:fft_size] * np.hanning(fft_size)
    magnitude = np.abs(np.fft.rfft(centered))
    bin_hz = sample_rate / fft_size
    if expected_frequency is None:
        first_bin = max(1, int(10.0 / bin_hz))
        last_bin = min(magnitude.size - 1, int(sample_rate * 0.45 / bin_hz))
    else:
        first_bin = max(1, int(expected_frequency * 0.8 / bin_hz))
        last_bin = min(magnitude.size - 1, int(expected_frequency * 1.2 / bin_hz) + 1)
    peak_bin = first_bin + int(np.argmax(magnitude[first_bin : last_bin + 1]))
    offset = 0.0
    if 0 < peak_bin < magnitude.size - 1:
        left, center, right = np.log(np.maximum(magnitude[peak_bin - 1 : peak_bin + 2], 1e-30))
        denominator = left - 2.0 * center + right
        if abs(denominator) > 1e-20:
            offset = float(0.5 * (left - right) / denominator)
    return (peak_bin + max(-0.5, min(0.5, offset))) * bin_hz


def phase_landmarks(np, samples, period: float):
    window = max(1, min(samples.size, int(round(period * 0.5))))
    left = window // 2
    right = window - left
    prefix = np.concatenate(([0.0], np.cumsum(samples, dtype=np.float64)))
    indices = np.arange(samples.size)
    begin = np.maximum(0, indices - left)
    end = np.minimum(samples.size, indices + right)
    proxy = ((prefix[end] - prefix[begin]) / (end - begin)).astype(np.float32)
    centered = proxy - np.median(proxy)
    crossings = upward_crossings(np, proxy)
    crossing_indices = np.floor(crossings).astype(int)
    slopes = centered[crossing_indices + 1] - centered[crossing_indices]
    first_window = crossings < period * 1.25
    if not np.any(first_window):
        raise ValueError("could not locate an initial phase landmark")
    first_candidates = np.flatnonzero(first_window)
    first = first_candidates[int(np.argmax(slopes[first_candidates]))]
    landmarks = [float(crossings[first])]
    predicted = landmarks[0] + period
    while predicted < samples.size - period * 0.5:
        candidates = np.flatnonzero(
            (crossings >= predicted - period * 0.3)
            & (crossings <= predicted + period * 0.3)
        )
        if candidates.size:
            selected = candidates[int(np.argmin(np.abs(crossings[candidates] - predicted)))]
            landmark = float(crossings[selected])
        else:
            landmark = predicted
        if landmark > landmarks[-1] + period * 0.5:
            landmarks.append(landmark)
        predicted = landmarks[-1] + period
    if len(landmarks) < 4:
        raise ValueError("could not identify enough phase landmarks")
    return np.asarray(landmarks, dtype=np.float64)


def robust_cycle_set(
    np,
    samples,
    sample_rate: float,
    phase_bins: int,
    max_cycles: int,
    expected_frequency: float | None,
):
    trim = min(int(sample_rate * 0.25), samples.size // 10)
    working = samples[trim : samples.size - trim if trim else None]
    frequency = estimate_frequency(np, working, sample_rate, expected_frequency)
    crossings = phase_landmarks(np, working, sample_rate / frequency)
    periods = np.diff(crossings)
    median_period = float(np.median(periods))
    valid = (periods > median_period * 0.8) & (periods < median_period * 1.2)
    starts = np.flatnonzero(valid)
    if starts.size < 3:
        raise ValueError("too few stable cycles after period rejection")
    if starts.size > max_cycles:
        starts = starts[np.linspace(0, starts.size - 1, max_cycles).astype(int)]
    phase = np.linspace(0.0, 1.0, phase_bins, endpoint=False)
    cycles = np.empty((starts.size, phase_bins), dtype=np.float32)
    source_index = np.arange(working.size, dtype=np.float64)
    accepted_periods = np.empty(starts.size, dtype=np.float64)
    for output_index, crossing_index in enumerate(starts):
        begin = crossings[crossing_index]
        period = periods[crossing_index]
        cycles[output_index] = np.interp(begin + phase * period, source_index, working)
        accepted_periods[output_index] = period
    return cycles, accepted_periods, int(periods.size - np.count_nonzero(valid))


def extract_pitch(
    np,
    samples,
    sample_rate: float,
    phase_bins: int,
    harmonics: int,
    max_cycles: int,
    expected_frequency: float | None = None,
):
    cycles, periods, cycles_rejected = robust_cycle_set(
        np, samples, sample_rate, phase_bins, max_cycles, expected_frequency
    )
    median_cycle = np.median(cycles, axis=0).astype(np.float32)
    spectrum = np.fft.rfft(median_cycle) / phase_bins
    harmonic_count = min(harmonics + 1, spectrum.size)
    complex_harmonics = spectrum[:harmonic_count].astype(np.complex64)
    measured_frequency = float(sample_rate / np.median(periods))
    rms = float(np.sqrt(np.mean(np.square(median_cycle, dtype=np.float64))))
    peak = float(np.max(np.abs(median_cycle)))
    midpoint = float((np.max(median_cycle) + np.min(median_cycle)) * 0.5)
    cycle_peak_to_peak = np.ptp(cycles, axis=1)
    return {
        "cycle": median_cycle,
        "harmonics": complex_harmonics,
        "frequency_hz": measured_frequency,
        "dc": float(np.mean(median_cycle, dtype=np.float64)),
        "rms": rms,
        "peak": peak,
        "crest_factor": peak / max(rms, sys.float_info.min),
        "duty_above_midpoint": float(np.mean(median_cycle > midpoint)),
        "period_jitter_ppm": float(np.std(periods) / np.mean(periods) * 1_000_000.0),
        "cycle_amplitude_cv": float(
            np.std(cycle_peak_to_peak) / max(np.mean(cycle_peak_to_peak), sys.float_info.min)
        ),
        "cycles_used": int(cycles.shape[0]),
        "cycles_rejected": cycles_rejected,
    }


def split_for_index(index: int) -> str:
    if index % 4 == 3:
        return "test"
    if index % 4 == 1:
        return "validation"
    return "train"


def midi_to_hz(note: float) -> float:
    return 440.0 * 2.0 ** ((note - 69.0) / 12.0)


def extract(
    manifest: dict[str, Any],
    root: Path,
    waveform: str,
    phase_bins: int,
    harmonics: int,
    max_cycles: int,
) -> None:
    sample_rate = float(manifest["capture"]["published_sample_rate_hz"])
    output_dir = root / "derived"
    output_dir.mkdir(parents=True, exist_ok=True)
    for name in selected_waveforms(waveform, manifest):
        entry = manifest["files"][name]
        path = source_path(root, entry)
        np, audio, conditioning = dataset_arrays(path, entry)
        first_midi = entry.get(
            "nominal_first_midi", manifest["pitch_grid"]["default_nominal_first_midi"]
        )
        results = []
        for index, row in enumerate(audio):
            try:
                result = extract_pitch(
                    np,
                    row,
                    sample_rate,
                    phase_bins,
                    harmonics,
                    max_cycles,
                    midi_to_hz(first_midi + index),
                )
            except ValueError as error:
                raise ValueError(f"{name} pitch_index {index}: {error}") from error
            results.append(result)
        max_harmonic_count = max(result["harmonics"].size for result in results)
        harmonic_values = np.zeros((len(results), max_harmonic_count), dtype=np.complex64)
        for index, result in enumerate(results):
            harmonic_values[index, : result["harmonics"].size] = result["harmonics"]
        cycles = np.stack([result["cycle"] for result in results])
        npz_path = output_dir / f"{name}-cycles-v1.npz"
        np.savez_compressed(
            npz_path,
            median_cycles=cycles,
            complex_harmonics=harmonic_values,
            measured_frequency_hz=np.asarray(
                [result["frequency_hz"] for result in results], dtype=np.float64
            ),
            conditioning=conditioning,
        )
        records = []
        for index, result in enumerate(results):
            nominal_midi = first_midi + index
            expected_frequency = midi_to_hz(nominal_midi)
            records.append(
                {
                    "pitch_index": index,
                    "nominal_midi": nominal_midi,
                    "split": split_for_index(index),
                    "conditioning_min": float(np.min(conditioning[index])),
                    "conditioning_max": float(np.max(conditioning[index])),
                    "expected_frequency_hz": expected_frequency,
                    "pitch_error_cents": 1200.0
                    * math.log2(result["frequency_hz"] / expected_frequency),
                    **{
                        key: value
                        for key, value in result.items()
                        if key not in {"cycle", "harmonics"}
                    },
                }
            )
        summary = {
            "schema_version": 1,
            "extractor_revision": 1,
            "target_id": manifest["target_id"],
            "waveform": name,
            "source_file": entry["name"],
            "source_md5": entry["md5"],
            "sample_rate_hz": sample_rate,
            "phase_bins": phase_bins,
            "harmonics_including_dc": max_harmonic_count,
            "max_cycles_per_pitch": max_cycles,
            "source_array_shape": list(audio.shape),
            "npz_file": npz_path.name,
            "npz_sha256": hashlib.sha256(npz_path.read_bytes()).hexdigest(),
            "pitches": records,
        }
        summary_path = output_dir / f"{name}-summary-v1.json"
        summary_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        print(f"{name}: wrote {npz_path} and {summary_path}")


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument(
        "command", choices=("download", "inspect", "extract"), help="operation to perform"
    )
    result.add_argument("--waveform", default="all", choices=("all", "saw", "triangle", "square"))
    result.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    result.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    result.add_argument("--phase-bins", type=int, default=2048)
    result.add_argument("--harmonics", type=int, default=256)
    result.add_argument("--max-cycles", type=int, default=1024)
    return result


def main() -> int:
    args = parser().parse_args()
    manifest = load_manifest(args.manifest)
    try:
        if args.command == "download":
            download(manifest, args.root, args.waveform)
        elif args.command == "inspect":
            inspect(manifest, args.root, args.waveform)
        else:
            if args.phase_bins < 64 or not args.phase_bins & (args.phase_bins - 1) == 0:
                raise ValueError("--phase-bins must be a power of two of at least 64")
            if args.harmonics < 1 or args.max_cycles < 3:
                raise ValueError("harmonics must be positive and max-cycles must be at least 3")
            extract(
                manifest,
                args.root,
                args.waveform,
                args.phase_bins,
                args.harmonics,
                args.max_cycles,
            )
    except (OSError, RuntimeError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
