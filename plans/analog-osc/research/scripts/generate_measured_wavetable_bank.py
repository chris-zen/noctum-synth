#!/usr/bin/env python3
"""Compatibility entry point for the authoritative Rust v2 bank generator."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[4]


def main() -> int:
    command = [
        "cargo",
        "run",
        "--release",
        "-p",
        "synth-tools",
        "--bin",
        "wavetable_bank",
        "--",
        "--bank",
        "monologue",
        *sys.argv[1:],
    ]
    return subprocess.run(command, cwd=REPO_ROOT, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
