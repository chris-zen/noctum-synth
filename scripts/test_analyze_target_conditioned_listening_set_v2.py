import importlib.util
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("analyze_target_conditioned_listening_set_v2.py")
SPEC = importlib.util.spec_from_file_location(
    "analyze_target_conditioned_listening_set_v2", SCRIPT
)
ANALYSIS = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = ANALYSIS
SPEC.loader.exec_module(ANALYSIS)


class TargetConditionedV2ListeningAnalysisTests(unittest.TestCase):
    def test_normalizes_plain_and_prefixed_choices(self):
        self.assertEqual(ANALYSIS.normalized_choice("a", ""), "A")
        self.assertEqual(ANALYSIS.normalized_choice("choice-B", "choice-"), "B")
        with self.assertRaises(ValueError):
            ANALYSIS.normalized_choice("C", "")

    def test_binomial_upper_tail(self):
        self.assertAlmostEqual(ANALYSIS.binomial_upper_tail(9, 9), 1.0 / 512.0)
        self.assertAlmostEqual(ANALYSIS.binomial_upper_tail(6, 9), 0.25390625)

    def test_summary_keeps_discrimination_separate_from_target_match(self):
        rows = [
            {
                "waveform": "saw",
                "abx_correct": True,
                "abx_confidence": 0.8,
                "target_match_source": "baseline",
                "target_match_confidence": 0.6,
            },
            {
                "waveform": "saw",
                "abx_correct": False,
                "abx_confidence": 0.4,
                "target_match_source": "candidate",
                "target_match_confidence": 0.2,
            },
        ]
        summary = ANALYSIS.summarize(rows)["saw"]
        self.assertEqual(summary["abx_correct"], 1)
        self.assertEqual(summary["baseline_judged_closer"], 1)
        self.assertEqual(summary["candidate_judged_closer"], 1)
        self.assertAlmostEqual(summary["mean_abx_confidence"], 0.6)


if __name__ == "__main__":
    unittest.main()
