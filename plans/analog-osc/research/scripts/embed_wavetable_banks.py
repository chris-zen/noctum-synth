#!/usr/bin/env python3
"""Validate v2 manifests and copy their binaries into compiled desktop assets."""

from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[4]
DEFAULT_OUTPUT = REPO_ROOT / "synth-core/src/voice/osc_engine/wavetable_banks"
MAX_COMBINED_BYTES = 20 * 1024 * 1024
BANKS = (
    (
        REPO_ROOT
        / "plans/analog-osc/research/banks/korg-monologue-measured-wavetable-v2.json",
        "monologue.f32le",
    ),
    (
        REPO_ROOT / "plans/analog-osc/research/banks/prophet5-wavetable-bank-v2.json",
        "prophet5.f32le",
    ),
)


def fnv1a32(data: bytes) -> int:
    result = 0x811C9DC5
    for byte in data:
        result = ((result ^ byte) * 0x01000193) & 0xFFFFFFFF
    return result


def load_bank(manifest_path: Path) -> tuple[Path, bytes, dict[str, Any]]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 2:
        raise ValueError(f"{manifest_path}: expected schema_version 2")
    metadata = manifest["bank_binary"]
    declared_path = Path(metadata["path"])
    source = declared_path if declared_path.is_absolute() else REPO_ROOT / declared_path
    if not source.is_file():
        source = manifest_path.parent / declared_path
    data = source.read_bytes()
    if len(data) != int(metadata["bytes"]):
        raise ValueError(
            f"{source}: manifest declares {metadata['bytes']} bytes, found {len(data)}"
        )
    if len(data) != int(metadata["sample_count"]) * 4:
        raise ValueError(f"{source}: sample count does not match byte count")
    checksum = fnv1a32(data)
    if checksum != int(metadata["fnv1a32"]):
        raise ValueError(
            f"{source}: expected FNV-1a 0x{int(metadata['fnv1a32']):08x}, "
            f"found 0x{checksum:08x}"
        )
    return source, data, manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument(
        "--bank", choices=("all", "monologue", "prophet5"), default="all"
    )
    args = parser.parse_args()
    selected = [
        (path, output_name)
        for path, output_name in BANKS
        if args.bank == "all" or output_name.startswith(args.bank)
    ]
    loaded = [(load_bank(path), output_name) for path, output_name in selected]
    combined_bytes = sum(len(bank[1]) for bank, _ in loaded)
    if combined_bytes > MAX_COMBINED_BYTES:
        raise ValueError(
            f"combined banks are {combined_bytes} bytes; cap is {MAX_COMBINED_BYTES}"
        )
    args.output.mkdir(parents=True, exist_ok=True)
    for ((source, _, _), output_name) in loaded:
        destination = args.output / output_name
        shutil.copyfile(source, destination)
        print(f"embedded {destination.relative_to(REPO_ROOT)}")
    print(f"combined compiled bank size: {combined_bytes} bytes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
