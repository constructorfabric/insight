import unittest

from run import dbt_work


class WorkTests(unittest.TestCase):
    def test_missing_transformation_work_cannot_count_as_equivalent(self) -> None:
        with self.assertRaisesRegex(ValueError, "no transformation"):
            dbt_work({"timings": []})

    def test_failed_or_dropped_dbt_tests_change_the_work_signature(self) -> None:
        node = {"unique_id": "test.example", "status": "pass", "execution_time": 1}
        baseline = {"timings": [{"stage": "dbt_build", "selector": "tag:gold", "nodes": [node]}]}
        for nodes in ([], [{**node, "status": "fail"}]):
            candidate = {"timings": [{"stage": "dbt_build", "selector": "tag:gold", "nodes": nodes}]}
            with self.subTest(nodes=nodes):
                self.assertNotEqual(dbt_work(baseline), dbt_work(candidate))
