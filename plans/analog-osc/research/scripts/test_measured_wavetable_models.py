import importlib.util
import sys
import unittest
from pathlib import Path

import numpy as np


SCRIPT = Path(__file__).with_name("evaluate_measured_wavetable_models.py")
SPEC = importlib.util.spec_from_file_location("evaluate_measured_wavetable_models", SCRIPT)
MODELS = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MODELS
SPEC.loader.exec_module(MODELS)


class MeasuredWavetableModelTests(unittest.TestCase):
    def test_split_reserves_validation_and_test_rows(self):
        self.assertEqual([MODELS.split_for_index(i) for i in range(4)], [
            "train", "validation", "train", "test"
        ])

    def test_complex_interpolation_preserves_endpoints_and_midpoint(self):
        frequencies = np.asarray([100.0, 400.0])
        spectra = np.asarray([[1.0 + 2.0j, 3.0 - 1.0j], [5.0 + 6.0j, 7.0 + 3.0j]])
        np.testing.assert_array_equal(
            MODELS.interpolate_complex(50.0, frequencies, spectra), spectra[0]
        )
        np.testing.assert_array_equal(
            MODELS.interpolate_complex(800.0, frequencies, spectra), spectra[1]
        )
        np.testing.assert_allclose(
            MODELS.interpolate_complex(200.0, frequencies, spectra),
            (spectra[0] + spectra[1]) * 0.5,
        )

    def test_bandlimit_keeps_dc_and_removes_harmonics_above_guard(self):
        spectrum = np.ones(33, dtype=np.complex128)
        limited = MODELS.bandlimit_spectrum(spectrum, 1_000.0)
        self.assertEqual(limited[0], 1.0)
        self.assertEqual(limited[21], 1.0)
        self.assertEqual(limited[22], 0.0)
        self.assertEqual(limited[-1], 0.0)

    def test_spectrum_reconstruction_round_trips(self):
        phase = np.arange(MODELS.PHASE_BINS) / MODELS.PHASE_BINS
        cycle = 0.2 + 0.7 * np.sin(2.0 * np.pi * phase) - 0.1 * np.cos(6.0 * np.pi * phase)
        np.testing.assert_allclose(
            MODELS.reconstruct(np.fft.rfft(cycle)), cycle, atol=1.0e-12
        )


if __name__ == "__main__":
    unittest.main()
