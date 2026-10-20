from __future__ import annotations

import copy
import unittest

from experiments import BY_NAME, SERVING
from fixtures import READ_PROBES
from report import validate


def document() -> dict[str, object]:
    experiment = BY_NAME["cpu-threads"]
    results = []
    for workload in experiment.workloads:
        for variant, threads in (("baseline", 2), ("candidate", 4)):
            for repeat in (1, 2, 3):
                sample = {
                    "workload": workload,
                    "variant": variant,
                    "repeat": repeat,
                    "parameters": {"max_threads": threads, "max_memory_usage": 2147483648},
                    "exact_rows_equal": True,
                }
                if workload == SERVING:
                    sample["reads"] = [{"name": name, "exact_rows_equal": True} for name in READ_PROBES]
                results.append(sample)
    return {"experiment": {"name": experiment.name}, "repeats": 3, "results": results}


class ReportTests(unittest.TestCase):
    def test_complete_single_parameter_comparisons_are_accepted(self) -> None:
        validate(document())

    def test_a_second_changed_parameter_is_rejected(self) -> None:
        changed = document()
        for sample in changed["results"]:
            if sample["variant"] == "candidate":
                sample["parameters"]["max_memory_usage"] = 3221225472
        with self.assertRaises(ValueError):
            validate(changed)

    def test_missing_and_duplicate_repetitions_are_rejected(self) -> None:
        valid = document()
        missing = copy.deepcopy(valid)
        missing["results"].pop()
        duplicate = copy.deepcopy(valid)
        duplicate["results"].append(copy.deepcopy(duplicate["results"][-1]))
        for invalid in (missing, duplicate):
            with self.subTest(repetitions=len(invalid["results"])), self.assertRaises(ValueError):
                validate(invalid)

    def test_failed_or_incorrect_samples_are_rejected(self) -> None:
        for failure in ({"error": "synthetic failure"}, {"exact_rows_equal": False}):
            invalid = document()
            invalid["results"][0].update(failure)
            with self.subTest(failure=failure), self.assertRaises(ValueError):
                validate(invalid)

    def test_a_serving_read_probe_cannot_be_dropped(self) -> None:
        invalid = document()
        serving = next(sample for sample in invalid["results"] if sample["workload"] == SERVING)
        serving["reads"].pop()
        with self.assertRaises(ValueError):
            validate(invalid)


if __name__ == "__main__":
    unittest.main()
