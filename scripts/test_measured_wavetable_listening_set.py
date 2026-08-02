#!/usr/bin/env python3
"""Tests for the measured-wavetable blind listening gate."""

from __future__ import annotations

import unittest

from generate_measured_wavetable_listening_set import (
    PREVIOUSLY_HEARD_PITCH_INDICES,
    selected_fresh_test_rows,
)


class MeasuredWavetableListeningSetTests(unittest.TestCase):
    def test_selection_uses_fresh_test_rows_across_registers(self) -> None:
        cases = [
            {"pitch_index": index, "split": "test" if index % 2 else "validation"}
            for index in range(72)
        ]
        selected = selected_fresh_test_rows({"cases": cases})
        self.assertEqual([name for name, _ in selected], ["low", "mid", "high"])
        indices = [row["pitch_index"] for _, row in selected]
        self.assertTrue(all(index % 2 for index in indices))
        self.assertTrue(all(index not in PREVIOUSLY_HEARD_PITCH_INDICES for index in indices))
        self.assertEqual(len(set(indices)), 3)
        self.assertLess(indices[0], indices[1])
        self.assertLess(indices[1], indices[2])

    def test_selection_rejects_too_few_fresh_rows(self) -> None:
        cases = [
            {"pitch_index": 3, "split": "test"},
            {"pitch_index": 7, "split": "test"},
            {"pitch_index": 9, "split": "validation"},
        ]
        with self.assertRaisesRegex(ValueError, "fewer than three"):
            selected_fresh_test_rows({"cases": cases})


if __name__ == "__main__":
    unittest.main()
