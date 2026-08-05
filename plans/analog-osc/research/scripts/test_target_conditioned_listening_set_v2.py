import importlib.util
import json
import sys
import unittest
from pathlib import Path

import numpy as np
from scipy.io import wavfile


SCRIPT = Path(__file__).with_name("generate_target_conditioned_listening_set_v2.py")
SPEC = importlib.util.spec_from_file_location(
    "generate_target_conditioned_listening_set_v2", SCRIPT
)
LISTENING = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = LISTENING
SPEC.loader.exec_module(LISTENING)


class TargetConditionedV2ListeningSetTests(unittest.TestCase):
    def test_selects_only_fresh_low_middle_high_test_rows(self):
        evaluation = []
        for index in range(72):
            split = "train" if index % 2 == 0 else "validation"
            if index % 4 == 3:
                split = "test"
            evaluation.append(
                {"pitch_index": index, "split": split, "frequency_hz": float(index)}
            )
        selected = LISTENING.selected_fresh_test_rows({"evaluation": evaluation})
        self.assertEqual([register for register, _ in selected], ["low", "mid", "high"])
        self.assertEqual([row["pitch_index"] for _, row in selected], [3, 35, 67])
        self.assertTrue(all(row["split"] == "test" for _, row in selected))
        self.assertNotIn(71, [row["pitch_index"] for _, row in selected])

    def test_generated_package_is_complete_when_present(self):
        root = LISTENING.DEFAULT_OUTPUT
        manifest_path = root / "manifest.json"
        if not manifest_path.exists():
            self.skipTest("v2 listening package has not been generated")
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        answers = json.loads((root / "answer-key.json").read_text(encoding="utf-8"))
        responses = json.loads(
            (root / "responses-template.json").read_text(encoding="utf-8")
        )
        self.assertEqual(len(manifest["cases"]), 9)
        self.assertEqual({case["split"] for case in manifest["cases"]}, {"test"})
        self.assertEqual(
            {case["pitch_index"] for case in manifest["cases"]}, {3, 35, 67}
        )

        referenced_files = []
        response_values = []
        for case in manifest["cases"]:
            identifier = case["id"]
            named = case["named_files"]
            abx = case["blind_abx_files"]
            target_match = case["blind_target_match_files"]
            self.assertEqual(len(named) + len(abx) + len(target_match), 9)
            referenced_files.extend([*named.values(), *abx.values(), *target_match.values()])

            key = answers["abx"][identifier]
            self.assertEqual(abx["X"]["sha256"], abx[key["correct"]]["sha256"])
            self.assertNotEqual(abx["A"]["sha256"], abx["B"]["sha256"])
            for choice in ("choice-A", "choice-B"):
                source_name = answers["target_match"][identifier][choice]
                self.assertEqual(
                    target_match[choice]["sha256"], named[source_name]["sha256"]
                )
            self.assertEqual(
                target_match["reference"]["sha256"],
                named["measured_target"]["sha256"],
            )
            abx_response = responses["abx"][identifier]["answer"]
            target_response = responses["target_match"][identifier]["closer_choice"]
            self.assertIn(abx_response, {None, "A", "B"})
            self.assertIn(target_response, {None, "A", "B", "choice-A", "choice-B"})
            response_values.extend((abx_response, target_response))

        self.assertEqual(len(referenced_files), 81)
        self.assertTrue(
            all(value is None for value in response_values)
            or all(value is not None for value in response_values)
        )
        for record in referenced_files:
            path = Path(record["path"])
            self.assertTrue(path.is_file())
            self.assertEqual(LISTENING.sha256_file(path), record["sha256"])
            sample_rate, samples = wavfile.read(path)
            self.assertEqual(sample_rate, LISTENING.SAMPLE_RATE_HZ)
            self.assertEqual(samples.dtype, np.float32)
            self.assertEqual(samples.ndim, 1)
            self.assertTrue(np.all(np.isfinite(samples)))
            self.assertLess(float(np.max(np.abs(samples))), 1.0)


if __name__ == "__main__":
    unittest.main()
