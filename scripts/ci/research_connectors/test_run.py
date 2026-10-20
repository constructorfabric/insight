import unittest

from run import validate


class ResultTests(unittest.TestCase):
    def test_dropped_retry_requests_cannot_look_like_a_speedup(self) -> None:
        baseline = {
            "variant": "baseline",
            "exit_code": 0,
            "outcomes": ["passed"],
            "requests": ["retry", "success"],
            "coverage": [1],
        }
        candidate = {**baseline, "variant": "candidate", "requests": ["success"]}
        with self.assertRaisesRegex(ValueError, "requests differs"):
            validate([baseline, candidate])

    def test_changed_coverage_cannot_look_like_a_speedup(self) -> None:
        baseline = {
            "variant": "baseline",
            "exit_code": 0,
            "outcomes": ["passed"],
            "requests": ["request"],
            "coverage": [1],
        }
        with self.assertRaisesRegex(ValueError, "coverage differs"):
            validate([baseline, {**baseline, "coverage": []}])


if __name__ == "__main__":
    unittest.main()
