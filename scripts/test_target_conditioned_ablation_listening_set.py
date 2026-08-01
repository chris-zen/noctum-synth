import importlib.util
import sys
import unittest
from pathlib import Path

import numpy as np


SCRIPT = Path(__file__).with_name(
    "generate_target_conditioned_ablation_listening_set.py"
)
SPEC = importlib.util.spec_from_file_location(
    "generate_target_conditioned_ablation_listening_set", SCRIPT
)
ABLATION = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = ABLATION
SPEC.loader.exec_module(ABLATION)


class TargetConditionedAblationListeningSetTests(unittest.TestCase):
    def test_selection_uses_two_unused_validation_cases(self):
        rows = []
        for index in range(72):
            if index % 2 == 0:
                split = "train"
            elif index % 4 == 1:
                split = "validation"
            else:
                split = "test"
            rows.append(
                {
                    "split": split,
                    "pitch_index": index,
                    "frequency_hz": float(index + 1),
                }
            )
        profile = {"evaluation": rows}
        prior = {
            row["pitch_index"] for _, row in ABLATION.selected_rows(profile)
        }
        selected = ABLATION.diagnostic_rows(profile)
        self.assertEqual([register for register, _ in selected], ["lower", "upper"])
        self.assertTrue(all(row["split"] == "validation" for _, row in selected))
        self.assertTrue(all(row["pitch_index"] not in prior for _, row in selected))

    def test_randomization_contains_every_variant_exactly_once(self):
        choices = ABLATION.randomized_choices(np.random.default_rng(1234))
        self.assertEqual(set(choices), {"choice-A", "choice-B", "choice-C", "choice-D"})
        self.assertEqual(set(choices.values()), set(ABLATION.VARIANTS))

    def test_ablation_definitions_isolate_phase_and_filter(self):
        self.assertEqual(ABLATION.VARIANTS["baseline"]["parameters"], {})
        self.assertEqual(
            ABLATION.VARIANTS["phase-only"]["parameters"],
            {"phase-amount": 1.0, "filter-amount": 0.0},
        )
        self.assertEqual(
            ABLATION.VARIANTS["filter-only"]["parameters"],
            {"phase-amount": 0.0, "filter-amount": 1.0},
        )
        self.assertEqual(
            ABLATION.VARIANTS["phase-plus-filter"]["parameters"],
            {"phase-amount": 1.0, "filter-amount": 1.0},
        )

    def test_fixed_seed_choice_order_is_reproducible(self):
        left = ABLATION.randomized_choices(np.random.default_rng(20_260_728))
        right = ABLATION.randomized_choices(np.random.default_rng(20_260_728))
        self.assertEqual(left, right)


if __name__ == "__main__":
    unittest.main()
