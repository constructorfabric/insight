import unittest

from snapshot import serving_nodes


class DependencyTests(unittest.TestCase):
    def test_views_keep_dependencies_but_physical_fixtures_stop_upstream_expansion(self) -> None:
        nodes = {}
        definitions = [
            ("gold_facts", "table", ["bronze_feed"], ["gold"]),
            ("bronze_feed", "table", [], []),
            ("metric_entity_cohorts_current", "view", ["class_people", "person_map"], ["gold"]),
            ("class_people", "table", ["bronze_feed"], []),
            ("person_map", "view", ["identity_inputs", "account_assignment"], []),
            ("identity_inputs", "incremental", ["bronze_feed"], []),
            ("account_assignment", "view", [], []),
        ]
        for name, materialized, parents, tags in definitions:
            nodes[name] = {
                "name": name,
                "resource_type": "model",
                "config": {"materialized": materialized},
                "depends_on": {"nodes": parents},
                "tags": tags,
            }
        names = [node["name"] for node in serving_nodes({"nodes": nodes})]
        self.assertEqual(
            names,
            [
                "gold_facts",
                "class_people",
                "identity_inputs",
                "account_assignment",
                "person_map",
                "metric_entity_cohorts_current",
            ],
        )
