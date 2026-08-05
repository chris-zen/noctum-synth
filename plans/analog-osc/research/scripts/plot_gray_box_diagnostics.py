#!/usr/bin/env python3
"""Plot a gray-box diagnostic CSV emitted by analog_osc_research."""

from __future__ import annotations

import argparse
import csv
import os
import tempfile
from pathlib import Path

os.environ.setdefault("MPLCONFIGDIR", str(Path(tempfile.gettempdir()) / "noctum-matplotlib"))
os.environ.setdefault("XDG_CACHE_HOME", str(Path(tempfile.gettempdir()) / "noctum-cache"))

import matplotlib.pyplot as plt

plt.rcParams["svg.hashsalt"] = "plan12-gray-box-v1"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("output", type=Path)
    options = parser.parse_args()
    with options.trace.open(newline="", encoding="utf-8") as source:
        rows = list(csv.DictReader(source))

    samples = [int(row["sample"]) for row in rows]
    capacitor = [float(row["capacitor_v"]) for row in rows]
    threshold = [float(row["threshold_v"]) for row in rows]
    raw = [float(row["raw_output"]) for row in rows]
    corrected = [float(row["corrected_output"]) for row in rows]
    events = [
        index
        for index, row in enumerate(rows)
        if int(row["state_events"]) > 0
    ]

    figure, axes = plt.subplots(2, 1, figsize=(10, 5.5), sharex=True)
    axes[0].plot(samples, capacitor, label="capacitor", linewidth=1.4)
    axes[0].plot(samples, threshold, label="pulse threshold", linestyle="--")
    axes[0].set_ylabel("normalized voltage")
    axes[0].legend(loc="upper right")
    axes[1].plot(samples, raw, label="raw", linewidth=1.0, alpha=0.75)
    axes[1].plot(samples, corrected, label="corrected output", linewidth=1.3)
    axes[1].set_ylabel("amplitude")
    axes[1].set_xlabel("sample")
    axes[1].legend(loc="upper right")
    for axis in axes:
        for event in events:
            axis.axvline(event, color="tab:red", alpha=0.18, linewidth=0.8)
        axis.grid(alpha=0.18)
    figure.suptitle("Plan 12 saw-core state and fractional reset events")
    figure.tight_layout()
    options.output.parent.mkdir(parents=True, exist_ok=True)
    figure.savefig(options.output, format="svg", metadata={"Date": None})


if __name__ == "__main__":
    main()
