from __future__ import annotations

import unittest

from run import MIB, variants


class SettingsTests(unittest.TestCase):
    def test_experiments_preserve_the_query_memory_cap_and_automatic_join_fallback(self) -> None:
        baseline = {
            "max_memory_usage": 2 * 1024 * MIB,
            "max_threads": 2,
            "max_bytes_before_external_group_by": 256 * MIB,
            "max_bytes_before_external_sort": 256 * MIB,
            "max_bytes_in_join": 256 * MIB,
            "join_algorithm": "auto",
        }
        original = dict(baseline)
        for name, settings in variants(baseline).items():
            with self.subTest(variant=name):
                self.assertEqual(settings["max_memory_usage"], baseline["max_memory_usage"])
                self.assertEqual(settings["join_algorithm"], "auto")
                self.assertLess(settings["max_bytes_before_external_group_by"], settings["max_memory_usage"])
                self.assertLess(settings["max_bytes_before_external_sort"], settings["max_memory_usage"])
                self.assertLess(settings["max_bytes_in_join"], settings["max_memory_usage"])
        self.assertEqual(baseline, original)


if __name__ == "__main__":
    unittest.main()
