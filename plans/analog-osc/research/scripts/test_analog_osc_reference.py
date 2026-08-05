import importlib.util
import unittest
from pathlib import Path

import numpy as np


SCRIPT = Path(__file__).with_name("analog_osc_reference.py")
SPEC = importlib.util.spec_from_file_location("analog_osc_reference", SCRIPT)
REFERENCE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(REFERENCE)


class ReferenceExtractionTests(unittest.TestCase):
    def test_phase_cycle_extraction_recovers_frequency_and_shape(self):
        sample_rate = 48_000.0
        frequency = 223.7
        phase = np.arange(96_000, dtype=np.float64) * frequency / sample_rate
        samples = (2.0 * np.mod(phase, 1.0) - 1.0).astype(np.float32)

        result = REFERENCE.extract_pitch(
            np, samples, sample_rate, phase_bins=1024, harmonics=64, max_cycles=256
        )

        self.assertAlmostEqual(result["frequency_hz"], frequency, places=3)
        self.assertLess(result["period_jitter_ppm"], 1.0)
        self.assertEqual(result["cycle"].shape, (1024,))
        self.assertEqual(result["harmonics"].shape, (65,))
        self.assertGreater(result["crest_factor"], 1.5)

    def test_pitch_constrained_landmarks_ignore_extra_midpoint_crossings(self):
        sample_rate = 48_000.0
        frequency = 41.2
        phase = np.arange(131_072, dtype=np.float64) * frequency / sample_rate
        cycle_phase = np.mod(phase, 1.0)
        samples = np.where(cycle_phase < 0.5, 0.35, -0.35)
        samples += (
            0.5
            * np.sin(2.0 * np.pi * 8.0 * cycle_phase)
            * np.exp(-3.0 * cycle_phase)
        )

        result = REFERENCE.extract_pitch(
            np,
            samples.astype(np.float32),
            sample_rate,
            phase_bins=1024,
            harmonics=64,
            max_cycles=128,
            expected_frequency=frequency,
        )

        self.assertAlmostEqual(result["frequency_hz"], frequency, places=2)
        self.assertEqual(result["cycle"].shape, (1024,))

    def test_split_is_deterministic_and_disjoint(self):
        splits = [REFERENCE.split_for_index(index) for index in range(72)]
        self.assertEqual(splits.count("train"), 36)
        self.assertEqual(splits.count("validation"), 18)
        self.assertEqual(splits.count("test"), 18)


if __name__ == "__main__":
    unittest.main()
