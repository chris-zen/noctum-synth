#!/usr/bin/env python3
"""Validate generated wavetable banks and copy them into compiled assets."""

from __future__ import annotations

import argparse
import shutil
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = REPO_ROOT / "synth-core/src/voice/osc_engine/wavetable_banks"


@dataclass(frozen=True)
class Bank:
    source: Path
    output_name: str
    samples: int
    fnv1a32: int


BANKS = (
    Bank(
        REPO_ROOT / "target/analog-osc/banks/korg-monologue-measured-bank-v1.f32le",
        "monologue.f32le",
        221_184,
        0x06C0FF46,
    ),
    Bank(
        REPO_ROOT / "target/analog-osc/banks/arturia-prophet5-measured-bank-v1.f32le",
        "prophet5.f32le",
        227_328,
        0xFA4A0D1C,
    ),
)


def fnv1a32(data: bytes) -> int:
    result = 0x811C9DC5
    for byte in data:
        result = ((result ^ byte) * 0x01000193) & 0xFFFFFFFF
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)

    for bank in BANKS:
        data = bank.source.read_bytes()
        expected_bytes = bank.samples * 4
        if len(data) != expected_bytes:
            raise ValueError(
                f"{bank.source}: expected {expected_bytes} bytes, found {len(data)}"
            )
        checksum = fnv1a32(data)
        if checksum != bank.fnv1a32:
            raise ValueError(
                f"{bank.source}: expected FNV-1a 0x{bank.fnv1a32:08x}, "
                f"found 0x{checksum:08x}"
            )
        destination = args.output / bank.output_name
        shutil.copyfile(bank.source, destination)
        print(f"embedded {destination.relative_to(REPO_ROOT)}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
