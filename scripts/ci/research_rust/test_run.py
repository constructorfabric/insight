import unittest
from pathlib import Path

from run import environment, fixture, scenarios


class ResearchTests(unittest.TestCase):
    def test_each_candidate_changes_one_setting(self) -> None:
        root = Path("/tmp/synthetic-ci-research")
        baseline = environment("baseline", root)
        for variant, changed in (("clippy-flags", set()), ("compiler-cache", {"RUSTC_WRAPPER"})):
            env = environment(variant, root)
            diff = {k for k in baseline.keys() | env.keys() if baseline.get(k) != env.get(k)}
            self.assertEqual(diff, changed | {"CARGO_TARGET_DIR"})

    def test_warm_repetitions_keep_source_and_edits_change_it(self) -> None:
        self.assertEqual(scenarios(2), [("cold", 1, 1), ("warm", 1, 1), ("warm", 2, 1), ("edit", 1, 2), ("edit", 2, 3)])
        self.assertNotEqual(fixture("", 1), fixture("", 2))


if __name__ == "__main__":
    unittest.main()
