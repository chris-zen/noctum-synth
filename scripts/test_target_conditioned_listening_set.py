import importlib.util
import sys
import unittest
from pathlib import Path

import numpy as np


SCRIPT = Path(__file__).with_name("generate_target_conditioned_listening_set.py")
SPEC = importlib.util.spec_from_file_location("generate_target_conditioned_listening_set", SCRIPT)
LISTENING = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = LISTENING
SPEC.loader.exec_module(LISTENING)


class TargetConditionedListeningSetTests(unittest.TestCase):
    def test_level_match_reaches_requested_ac_rms_before_short_fades(self):
        index = np.arange(48_000, dtype=np.float64)
        source = 0.7 * np.sin(2.0 * np.pi * 311.3 * index / 48_000.0) + 0.01
        target = 10.0 ** (-18.0 / 20.0)
        matched = LISTENING.level_match(source, target, 0)
        self.assertAlmostEqual(LISTENING.ac_rms(matched), target, places=7)
        faded = LISTENING.level_match(source, target, 960)
        self.assertLess(LISTENING.ac_rms(faded), target)
        self.assertGreater(LISTENING.ac_rms(faded), target * 0.97)
        self.assertEqual(faded[0], 0.0)

    def test_periodic_target_follows_requested_frequency(self):
        cycle = np.sin(2.0 * np.pi * np.arange(2_048) / 2_048)
        rendered = LISTENING.periodic_target(cycle, 375.0, 48_000)
        crossings = np.flatnonzero((rendered[:-1] <= 0.0) & (rendered[1:] > 0.0))
        self.assertEqual(len(crossings), 375)

    def test_case_selection_is_low_mid_high_held_out(self):
        rows = [
            {"split": "train" if index % 2 == 0 else "validation", "frequency_hz": index}
            for index in range(14)
        ]
        selected = LISTENING.selected_rows({"evaluation": rows})
        self.assertEqual([name for name, _ in selected], ["low", "mid", "high"])
        self.assertEqual(selected[0][1]["frequency_hz"], 1)
        self.assertEqual(selected[-1][1]["frequency_hz"], 13)


if __name__ == "__main__":
    unittest.main()
