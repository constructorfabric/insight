"""Every bronze relation the generators write must have a connector to create it.

The DDL snapshot no longer carries bronze, so the only thing that creates those
tables is `destination-clickhouse`, run per connector by `dev-compose.sh` before
the seed container starts. It runs the connectors named in
`bootstrap-db/connectors-config.yaml`; a generator writing into a database no
connector there produces would fail the seed with "has no columns", which is a
late and unhelpful place to learn it.
"""

from __future__ import annotations

import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[5]
CONNECTORS = ROOT / "src/ingestion/connectors"
CONNECTORS_CONFIG = ROOT / "src/ingestion/scripts/bootstrap-db/connectors-config.yaml"

BRONZE_PREFIX = "bronze_"

_NAMESPACE = re.compile(r"^\s*namespace:\s*(?P<ns>\S+)\s*$", re.MULTILINE)
_CONNECTOR_KEY = re.compile(r"^  (?P<name>[A-Za-z0-9][A-Za-z0-9_-]*):\s*$", re.MULTILINE)


def _namespace_to_connector() -> dict[str, str]:
    """Each connector's bronze database, as its own descriptor declares it."""
    mapping: dict[str, str] = {}
    for descriptor in sorted(CONNECTORS.glob("*/*/descriptor.yaml")):
        found = _NAMESPACE.search(descriptor.read_text(encoding="utf-8"))
        if found:
            mapping[found.group("ns").strip("\"'")] = descriptor.parent.name
    return mapping


def _bootstrapped_connectors() -> set[str]:
    """The connectors `seed-connectors.sh` runs, as the config lists them."""
    text = CONNECTORS_CONFIG.read_text(encoding="utf-8")
    return {match.group("name") for match in _CONNECTOR_KEY.finditer(text)}


class SeedBronzeHasACreator(unittest.TestCase):
    def test_every_bronze_relation_the_generators_clear_has_a_connector(self) -> None:
        from insight_seed.generators.insert import RESET_TARGETS

        namespaces = {db for db, _ in RESET_TARGETS if db.startswith(BRONZE_PREFIX)}
        self.assertTrue(namespaces, "no bronze targets found — has RESET_TARGETS moved?")

        by_namespace = _namespace_to_connector()
        unmapped = sorted(ns for ns in namespaces if ns not in by_namespace)
        self.assertEqual(unmapped, [], f"no connector declares these namespaces: {unmapped}")

        bootstrapped = _bootstrapped_connectors()
        missing = sorted({by_namespace[ns] for ns in namespaces} - bootstrapped)
        self.assertEqual(
            missing,
            [],
            f"the generators write bronze for {missing}, which connectors-config.yaml does "
            f"not list — the seed would fail with 'has no columns'",
        )


if __name__ == "__main__":
    unittest.main()
