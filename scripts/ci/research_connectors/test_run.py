import unittest

from run import LOOKBACK_REQUEST, validate


class ResultTests(unittest.TestCase):
    def test_dropped_retry_requests_cannot_look_like_a_speedup(self) -> None:
        baseline = {
            "variant": "baseline",
            "exit_code": 0,
            "outcomes": ["passed"],
            "requests": [["case", "retry", 429, 1], ["case", "success", 200, 1]],
            "coverage": [1],
        }
        candidate = {**baseline, "variant": "candidate", "requests": [["case", "success", 200, 1]]}
        with self.assertRaisesRegex(ValueError, "requests differs"):
            validate([baseline, candidate])

    def test_only_the_known_successful_lookback_can_repeat(self) -> None:
        baseline = {
            "variant": "baseline",
            "exit_code": 0,
            "outcomes": ["passed"],
            "requests": [[*LOOKBACK_REQUEST, 2]],
            "coverage": [1],
        }
        validate([baseline, {**baseline, "variant": "candidate", "requests": [[*LOOKBACK_REQUEST, 1]]}])
        for count in (0, 3):
            with self.subTest(count=count), self.assertRaises(ValueError):
                validate([baseline, {**baseline, "variant": "candidate", "requests": [[*LOOKBACK_REQUEST, count]]}])
        with self.assertRaisesRegex(ValueError, "requests differs"):
            validate([baseline, {**baseline, "variant": "candidate", "requests": [["other", "request", 200, 1]]}])

    def test_changed_coverage_cannot_look_like_a_speedup(self) -> None:
        baseline = {
            "variant": "baseline",
            "exit_code": 0,
            "outcomes": ["passed"],
            "requests": [["case", "request", 200, 1]],
            "coverage": [1],
        }
        with self.assertRaisesRegex(ValueError, "coverage differs"):
            validate([baseline, {**baseline, "coverage": []}])


if __name__ == "__main__":
    unittest.main()
