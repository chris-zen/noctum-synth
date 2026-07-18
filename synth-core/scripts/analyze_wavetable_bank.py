#!/usr/bin/env python3
"""Aggregate spectral/level checks for a generated f32 prototype bank."""

import argparse
import csv
import math
import struct
from pathlib import Path

import numpy as np

RATE = 48_000
LIMITS = np.array([4095, 2047, 1023, 511, 255, 127, 63, 31, 15, 7, 3, 1])
LENGTHS = np.array([8192, 4096, 2048, 1024, 512, 256, 256, 256, 256, 128, 64, 64])
OFFSETS = np.array([0, 8192, 12288, 14336, 15360, 15872, 16128, 16384, 16640, 16896, 17024, 17088])
WAVE_SAMPLES = 17_152
FFT_SAMPLES = 48_000


def table_sample(bank, wave, phase, increment):
    safe = int(0.45 / increment) if increment > 0 else 2**32 - 1
    matches = np.flatnonzero(LIMITS <= safe)
    level = int(matches[0]) if len(matches) else len(LIMITS) - 1
    length = int(LENGTHS[level])
    position = phase * length
    index = position.astype(np.int64) & (length - 1)
    fraction = position - index
    offset = (0 if wave == "saw" else WAVE_SAMPLES) + int(OFFSETS[level])
    table = bank[offset : offset + length]
    return table[index] + (table[(index + 1) & (length - 1)] - table[index]) * fraction


def render(bank, wave, frequency, pulse_width=0.5, shape=0.0):
    frequency = min(frequency, RATE * 0.499)
    increment = frequency / RATE
    phase = (np.arange(FFT_SAMPLES, dtype=np.float64) * increment) % 1.0
    if wave == "pulse":
        saw = table_sample(bank, "saw", phase, increment)
        shifted = table_sample(bank, "saw", (phase + pulse_width) % 1.0, increment)
        return saw - shifted + 2.0 * pulse_width - 1.0
    if wave == "sawtri":
        saw = table_sample(bank, "saw", phase, increment)
        triangle = table_sample(bank, "triangle", phase, increment)
        return saw + (triangle - saw) * shape
    return table_sample(bank, wave, phase, increment)


def metrics(signal, frequency):
    count = len(signal)
    index = np.arange(count)
    phase = 2.0 * np.pi * index / (count - 1)
    window = (
        0.35875
        - 0.48829 * np.cos(phase)
        + 0.14128 * np.cos(2.0 * phase)
        - 0.01168 * np.cos(3.0 * phase)
    )
    spectrum = np.fft.rfft(signal * window)
    mask = np.zeros(len(spectrum), dtype=bool)
    mask[:6] = True
    bin_hz = RATE / count
    harmonic = frequency
    while harmonic < RATE * 0.5:
        center = round(harmonic / bin_hz)
        mask[max(0, center - 5) : min(len(mask), center + 6)] = True
        harmonic += frequency
    fundamental = round(frequency / bin_hz)
    fundamental_power = np.square(np.abs(spectrum[max(0, fundamental - 5) : fundamental + 6])).sum()
    alias_power = np.square(np.abs(spectrum[~mask])).sum()
    alias_dbc = 10.0 * np.log10(max(alias_power / fundamental_power, 1e-30))
    alias_dbfs = 10.0 * np.log10(max(alias_power / window.sum() ** 2, 1e-30)) + 6.020599913
    rms = math.sqrt(float(np.square(signal).mean()))
    return alias_dbc, alias_dbfs, 20.0 * math.log10(max(rms, 1e-30)), float(signal.mean()), float(np.max(np.abs(signal)))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("bank", type=Path)
    parser.add_argument("--output", type=Path, default=Path("plans/wavetable-quality-summary.csv"))
    args = parser.parse_args()
    raw = args.bank.read_bytes()
    bank = np.array(struct.unpack(f"<{len(raw) // 4}f", raw), dtype=np.float64)
    if len(bank) != WAVE_SAMPLES * 2:
        raise SystemExit(f"unexpected bank samples: {len(bank)}")

    rows = []
    for note in range(128):
        frequency = 440.0 * 2.0 ** ((note - 69) / 12.0)
        for wave in ("saw", "triangle", "pulse", "sawtri"):
            signal = render(bank, wave, frequency, pulse_width=0.5, shape=0.5)
            rows.append(("midi", note, wave, frequency, *metrics(signal, min(frequency, RATE * 0.499))))
    for width_percent in range(1, 100):
        width = width_percent / 100.0
        signal = render(bank, "pulse", 997.0, pulse_width=width)
        rows.append(("pulse_width", width, "pulse", 997.0, *metrics(signal, 997.0)))
    for shape in np.linspace(0.0, 1.0, 101):
        signal = render(bank, "sawtri", 997.0, shape=float(shape))
        rows.append(("shape", float(shape), "sawtri", 997.0, *metrics(signal, 997.0)))

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", newline="") as output:
        writer = csv.writer(output)
        writer.writerow(("group", "value", "waveform", "frequency_hz", "alias_dbc", "alias_dbfs", "rms_dbfs", "dc", "peak"))
        writer.writerows(rows)

    print("waveform,worst_alias_dbfs_through_4khz,worst_alias_dbc_through_4khz")
    for wave in ("saw", "triangle", "pulse", "sawtri"):
        subset = [row for row in rows if row[0] == "midi" and row[2] == wave and row[3] <= 4_000.0]
        print(f"{wave},{max(row[5] for row in subset):.2f},{max(row[4] for row in subset):.2f}")


if __name__ == "__main__":
    main()
