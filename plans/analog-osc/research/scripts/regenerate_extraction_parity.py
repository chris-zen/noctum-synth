#!/usr/bin/env python3
"""Regenerate Rust/Python extractor parity expectations from tracked samples."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np

from analog_osc_reference import estimate_frequency, extract_pitch


ROOT = Path(__file__).resolve().parents[4]
FIXTURES = ROOT / "synth-capture/tests/fixtures/extraction"
METADATA = FIXTURES / "python_parity_v1.json"


def main() -> None:
    metadata = json.loads(METADATA.read_text(encoding="utf-8"))
    for case in metadata["cases"].values():
        samples = np.fromfile(FIXTURES / case["samples_file"], dtype="<f4")
        sample_rate = float(case["sample_rate_hz"])
        search = float(case["search_frequency_hz"])
        if case.get("python") is None:
            case["python_frequency_hz"] = float(
                estimate_frequency(np, samples, sample_rate, search)
            )
            continue
        result = extract_pitch(np, samples, sample_rate, 2048, 256, 1024, search)
        cycle = result.pop("cycle")
        harmonics = result.pop("harmonics")
        case["python"] = {
            **result,
            "median_cycle_stride64": cycle[::64].astype(float).tolist(),
            "median_cycle_len": int(cycle.size),
            "harmonics_re_head8": harmonics.real[:8].astype(float).tolist(),
            "harmonics_im_head8": harmonics.imag[:8].astype(float).tolist(),
            "harmonics_len": int(harmonics.size),
        }
    METADATA.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
