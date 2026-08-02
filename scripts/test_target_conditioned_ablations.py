import importlib.util
import sys
import unittest
from pathlib import Path

import numpy as np


SCRIPT = Path(__file__).with_name("evaluate_target_conditioned_ablations.py")
SPEC = importlib.util.spec_from_file_location(
    "evaluate_target_conditioned_ablations", SCRIPT
)
ABLATIONS = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = ABLATIONS
SPEC.loader.exec_module(ABLATIONS)


class TargetConditionedAblationTests(unittest.TestCase):
    def test_shape_metrics_ignore_gain_and_dc_but_native_error_does_not(self):
        phase = np.arange(256, dtype=np.float64) / 256
        target = np.sin(2.0 * np.pi * phase) + 0.1 * np.sin(6.0 * np.pi * phase)
        candidate = 2.5 * target + 0.75
        self.assertGreater(ABLATIONS.nrmse(target, candidate), 1.0)
        normalized_error = np.sqrt(
            np.mean(
                (
                    ABLATIONS.centered_unit_rms(target)
                    - ABLATIONS.centered_unit_rms(candidate)
                )
                ** 2
            )
        )
        self.assertLess(normalized_error, 1.0e-12)
        complex_error, magnitude_error = ABLATIONS.harmonic_errors(
            target, candidate, 100.0
        )
        self.assertLess(complex_error, 1.0e-12)
        self.assertLess(magnitude_error, 1.0e-12)

    def test_phase_alignment_recovers_known_shift(self):
        bins = 512
        phase = np.arange(bins, dtype=np.float64) / bins
        target = np.sin(2.0 * np.pi * phase) + 0.4 * np.sin(4.0 * np.pi * phase)
        shifted = np.sin(2.0 * np.pi * (phase + 0.137)) + 0.4 * np.sin(
            4.0 * np.pi * (phase + 0.137)
        )
        shift, error, _ = ABLATIONS.best_phase_alignment(target, shifted, 0.0)
        self.assertAlmostEqual(shift, 0.137, places=4)
        self.assertLess(error, 1.0e-5)

    def test_validation_rows_exclude_train_and_test(self):
        profile = {
            "evaluation": [
                {"split": "train"},
                {"split": "validation"},
                {"split": "test"},
                {"split": "validation"},
            ]
        }
        self.assertEqual(
            ABLATIONS.validation_rows(profile),
            [{"split": "validation"}, {"split": "validation"}],
        )


if __name__ == "__main__":
    unittest.main()
