import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np


SCRIPT = Path(__file__).with_name("generate_measured_wavetable_bank.py")
SPEC = importlib.util.spec_from_file_location("generate_measured_wavetable_bank", SCRIPT)
BANK = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = BANK
SPEC.loader.exec_module(BANK)


class MeasuredWavetableBankGenerationTests(unittest.TestCase):
    def test_layout_size_is_complete(self):
        self.assertEqual(
            BANK.BANK_SAMPLES,
            len(BANK.WAVEFORMS) * BANK.PITCH_COUNT * BANK.TABLE_LENGTH,
        )

    def test_pitch_safe_reconstruction_retains_only_safe_harmonics(self):
        phase = np.arange(BANK.PHASE_BINS) / BANK.PHASE_BINS
        cycle = np.sin(2.0 * np.pi * phase) + 0.25 * np.sin(10.0 * np.pi * phase)
        guard_frequency = BANK.NYQUIST_GUARD * BANK.BANK_REFERENCE_SAMPLE_RATE_HZ / 3.5
        table, limit = BANK.pitch_safe_table(cycle, guard_frequency)
        spectrum = np.abs(np.fft.rfft(table)) / table.size
        self.assertEqual(limit, 3)
        self.assertGreater(spectrum[1], 0.49)
        self.assertLess(spectrum[5], 1.0e-6)

    def test_fnv_hash_is_stable_and_byte_ordered(self):
        values = np.asarray([0.0, 1.0, -1.0], dtype="<f4")
        self.assertEqual(BANK.fnv1a32(values), 0x21354B65)

    def test_generated_artifacts_are_self_consistent_when_present(self):
        if not all(
            path.exists() for path in (BANK.DEFAULT_MANIFEST, BANK.DEFAULT_BINARY, BANK.DEFAULT_RUST)
        ):
            self.skipTest("measured bank artifacts have not been generated")
        manifest = json.loads(BANK.DEFAULT_MANIFEST.read_text(encoding="utf-8"))
        checksum = manifest.pop("manifest_content_sha256")
        self.assertEqual(BANK.profile_checksum(manifest), checksum)
        self.assertEqual(BANK.sha256_file(BANK.DEFAULT_BINARY), manifest["bank_binary"]["sha256"])
        values = np.fromfile(BANK.DEFAULT_BINARY, dtype="<f4")
        self.assertEqual(values.shape, (BANK.BANK_SAMPLES,))
        self.assertTrue(np.all(np.isfinite(values)))
        self.assertEqual(BANK.fnv1a32(values), manifest["bank_binary"]["fnv1a32"])
        with tempfile.TemporaryDirectory() as directory:
            generated = Path(directory) / "profile.rs"
            BANK.write_rust_metadata(generated, manifest, checksum)
            self.assertEqual(
                generated.read_text(encoding="utf-8"),
                BANK.DEFAULT_RUST.read_text(encoding="utf-8"),
            )


if __name__ == "__main__":
    unittest.main()
