import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np


SCRIPT = Path(__file__).with_name("fit_target_conditioned_oscillator.py")
SPEC = importlib.util.spec_from_file_location("fit_target_conditioned_oscillator", SCRIPT)
FITTER = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = FITTER
SPEC.loader.exec_module(FITTER)


class TargetConditionedFitTests(unittest.TestCase):
    def test_phase_map_is_periodic_and_monotonic_for_valid_coefficients(self):
        phase = np.arange(4096, dtype=np.float64) / 4096
        mapped = FITTER.phase_map(phase, 0.12, -0.04, 0.0)
        unwrapped = np.unwrap(mapped * 2.0 * np.pi) / (2.0 * np.pi)
        self.assertGreater(np.min(np.diff(unwrapped)), 0.0)
        self.assertGreater(FITTER.minimum_phase_derivative(0.12, -0.04), 0.8)

    def test_log_frequency_interpolation_preserves_knots_and_midpoint(self):
        frequencies = np.asarray([100.0, 400.0])
        parameters = np.asarray([[0.0, 1.0], [2.0, 3.0]])
        np.testing.assert_array_equal(
            FITTER.interpolate_parameters(100.0, frequencies, parameters), parameters[0]
        )
        np.testing.assert_array_equal(
            FITTER.interpolate_parameters(400.0, frequencies, parameters), parameters[1]
        )
        np.testing.assert_allclose(
            FITTER.interpolate_parameters(200.0, frequencies, parameters), [1.0, 2.0]
        )

    def test_rendered_cycles_are_finite_for_every_waveform(self):
        parameters = FITTER.initial_parameters(np.linspace(-1.0, 1.0, FITTER.PHASE_BINS))
        for waveform in ("saw", "triangle", "square"):
            cycle = FITTER.render_cycle(waveform, 440.0, parameters, 0.0)
            self.assertEqual(cycle.shape, (FITTER.PHASE_BINS,))
            self.assertTrue(np.isfinite(cycle).all())

    def test_checked_in_profile_checksum_and_rust_source_are_reproducible(self):
        profile_path = FITTER.DEFAULT_PROFILE
        rust_path = FITTER.DEFAULT_RUST
        if not profile_path.exists() or not rust_path.exists():
            self.skipTest("fitted artifacts have not been generated")
        profile = json.loads(profile_path.read_text(encoding="utf-8"))
        checksum = profile.pop("profile_content_sha256")
        self.assertEqual(FITTER.profile_checksum(profile), checksum)
        with tempfile.TemporaryDirectory() as directory:
            generated = Path(directory) / "profile.rs"
            FITTER.write_rust_profile(generated, profile, checksum)
            self.assertEqual(
                generated.read_text(encoding="utf-8"),
                rust_path.read_text(encoding="utf-8"),
            )


if __name__ == "__main__":
    unittest.main()
