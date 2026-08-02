import importlib.util
import sys
import unittest
from pathlib import Path

import numpy as np


SCRIPT = Path(__file__).with_name("evaluate_target_conditioned_sweeps.py")
SPEC = importlib.util.spec_from_file_location("evaluate_target_conditioned_sweeps", SCRIPT)
SWEEPS = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = SWEEPS
SPEC.loader.exec_module(SWEEPS)


class TargetConditionedSweepTests(unittest.TestCase):
    def test_analytic_harmonics_have_low_residual(self):
        sample_rate = 48_000.0
        frequency = 375.37
        index = np.arange(65_536, dtype=np.float64)
        samples = np.zeros(index.size)
        for harmonic, amplitude in ((1, 1.0), (2, 0.25), (5, 0.1), (11, 0.03)):
            samples += amplitude * np.sin(
                2.0 * np.pi * harmonic * frequency * index / sample_rate
            )
        metrics = SWEEPS.spectral_residual(samples, sample_rate, frequency)
        self.assertLess(metrics["residual_dbc"], -90.0)

    def test_nonharmonic_spur_is_reported(self):
        sample_rate = 48_000.0
        frequency = 375.37
        index = np.arange(65_536, dtype=np.float64)
        clean = np.sin(2.0 * np.pi * frequency * index / sample_rate)
        contaminated = clean + 0.01 * np.sin(2.0 * np.pi * 7_011.0 * index / sample_rate)
        clean_metrics = SWEEPS.spectral_residual(clean, sample_rate, frequency)
        contaminated_metrics = SWEEPS.spectral_residual(
            contaminated, sample_rate, frequency
        )
        self.assertGreater(
            contaminated_metrics["residual_dbc"], clean_metrics["residual_dbc"] + 30.0
        )
        self.assertGreater(contaminated_metrics["worst_residual_component_dbc"], -50.0)

    def test_frequency_selection_includes_endpoints_without_duplicates(self):
        rows = [
            {"split": "validation", "frequency_hz": float(index + 1)}
            for index in range(12)
        ]
        selected = SWEEPS.selected_frequencies({"evaluation": rows}, 7)
        self.assertEqual(selected[0], 1.0)
        self.assertEqual(selected[-1], 12.0)
        self.assertEqual(len(selected), len(set(selected)))


if __name__ == "__main__":
    unittest.main()
