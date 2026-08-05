import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np


SCRIPT = Path(__file__).with_name("fit_target_conditioned_oscillator_v2.py")
SPEC = importlib.util.spec_from_file_location("fit_target_conditioned_oscillator_v2", SCRIPT)
V2 = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = V2
SPEC.loader.exec_module(V2)


class TargetConditionedV2FitTests(unittest.TestCase):
    def test_checked_in_blep_table_is_complete_and_finite(self):
        self.assertEqual(V2.BLEP_TABLE.shape, (4096,))
        self.assertTrue(np.all(np.isfinite(V2.BLEP_TABLE)))

    def test_identity_conditioned_source_matches_production_phase_convention(self):
        for waveform in ("saw", "triangle", "square"):
            production = V2.production_source(waveform, 440.0)
            conditioned = V2.conditioned_source(waveform, 440.0, 0.0, 0.0)
            np.testing.assert_allclose(conditioned, production, atol=1.0e-12)

    def test_periodic_shift_rotates_without_changing_harmonic_magnitudes(self):
        source = V2.production_source("saw", 220.0)
        shifted = V2.periodic_shift(source, 0.173)
        source_magnitude = np.abs(np.fft.rfft(V2.centered_unit_rms(source)))
        shifted_magnitude = np.abs(np.fft.rfft(V2.centered_unit_rms(shifted)))
        np.testing.assert_allclose(
            shifted_magnitude[:-1], source_magnitude[:-1], atol=1.0e-10
        )

    def test_python_production_source_matches_release_renderer_when_available(self):
        binary = V2.REPO_ROOT / "target/release/analog_osc_research"
        if not binary.is_file():
            self.skipTest("release research renderer is not built")
        from scripts.generate_target_conditioned_listening_set import render_model

        for waveform, period_samples in (
            ("saw", 275),
            ("triangle", 550),
            ("square", 275),
        ):
            frequency_hz = V2.SAMPLE_RATE_HZ / period_samples
            rendered = render_model(
                binary,
                "baseline-v1",
                waveform,
                frequency_hz,
                0,
                period_samples,
            )
            predicted = V2.production_source(
                waveform, frequency_hz, bins=period_samples
            )
            np.testing.assert_allclose(rendered, predicted, atol=5.0e-6)

    def test_profile_checksum_and_rust_output_are_reproducible(self):
        if not V2.DEFAULT_PROFILE.exists() or not V2.DEFAULT_RUST.exists():
            self.skipTest("v2 fitted artifacts have not been generated")
        profile = json.loads(V2.DEFAULT_PROFILE.read_text(encoding="utf-8"))
        checksum = profile.pop("profile_content_sha256")
        self.assertEqual(V2.profile_checksum(profile), checksum)
        with tempfile.TemporaryDirectory() as directory:
            generated = Path(directory) / "profile.rs"
            V2.write_rust_profile(generated, profile, checksum)
            self.assertEqual(
                generated.read_text(encoding="utf-8"),
                V2.DEFAULT_RUST.read_text(encoding="utf-8"),
            )


if __name__ == "__main__":
    unittest.main()
