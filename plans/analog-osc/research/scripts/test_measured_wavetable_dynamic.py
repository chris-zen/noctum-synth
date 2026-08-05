import importlib.util
import sys
import unittest
from pathlib import Path

import numpy as np


SCRIPT = Path(__file__).with_name("evaluate_measured_wavetable_dynamic.py")
SPEC = importlib.util.spec_from_file_location("evaluate_measured_wavetable_dynamic", SCRIPT)
DYNAMIC = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = DYNAMIC
SPEC.loader.exec_module(DYNAMIC)


class MeasuredWavetableDynamicTests(unittest.TestCase):
    def test_rate_comparison_agrees_for_bandlimited_tone(self):
        seconds = 1.0
        low_rate = 48_000
        high_rate = 192_000
        low_time = np.arange(int(seconds * low_rate)) / low_rate
        high_time = np.arange(int(seconds * high_rate)) / high_rate
        low = np.sin(2.0 * np.pi * 440.0 * low_time)
        high = np.sin(2.0 * np.pi * 440.0 * high_time)

        metrics = DYNAMIC.compare_rates(low, high)

        self.assertLess(metrics["normalized_rms_error"], 1.0e-4)
        self.assertGreater(metrics["correlation"], 0.999999)

    def test_high_band_metric_separates_low_and_high_tones(self):
        sample_rate = 48_000
        time = np.arange(sample_rate) / sample_rate
        low = np.sin(2.0 * np.pi * 440.0 * time)
        high = np.sin(2.0 * np.pi * 20_000.0 * time)

        self.assertLess(DYNAMIC.high_band_dbc(low, sample_rate), -100.0)
        self.assertGreater(DYNAMIC.high_band_dbc(high, sample_rate), -1.0)


if __name__ == "__main__":
    unittest.main()
