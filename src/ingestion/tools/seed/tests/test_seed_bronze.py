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

BRONZE_PREFIX = "bronze_"

CONFIG_IN_TREE = "scripts/bootstrap-db/connectors-config.yaml"


def _ingestion_tree() -> pathlib.Path | None:
    """The ingestion tree, which sits at a different depth in each layout.

    A checkout puts this suite at `src/ingestion/tools/seed/tests`; the seed image
    copies it to `/ingestion/tools/seed/tests`, one level shallower. Counting
    parents gets one of them wrong, so probe for the tree instead.
    """
    for parent in pathlib.Path(__file__).resolve().parents:
        if (parent / "connectors").is_dir() and (parent / CONFIG_IN_TREE).is_file():
            return parent
    return None


INGESTION = _ingestion_tree()

_NAMESPACE = re.compile(r"^\s*namespace:\s*(?P<ns>\S+)\s*$", re.MULTILINE)
_CONNECTOR_KEY = re.compile(r"^  (?P<name>[A-Za-z0-9][A-Za-z0-9_-]*):\s*$", re.MULTILINE)


def _namespace_to_connector(tree: pathlib.Path) -> dict[str, str]:
    """Each connector's bronze database, as its own descriptor declares it."""
    mapping: dict[str, str] = {}
    for descriptor in sorted((tree / "connectors").glob("*/*/descriptor.yaml")):
        found = _NAMESPACE.search(descriptor.read_text(encoding="utf-8"))
        if found:
            mapping[found.group("ns").strip("\"'")] = descriptor.parent.name
    return mapping


def _bootstrapped_connectors(tree: pathlib.Path) -> set[str]:
    """The connectors `seed-connectors.sh` runs, as the config lists them."""
    text = (tree / CONFIG_IN_TREE).read_text(encoding="utf-8")
    return {match.group("name") for match in _CONNECTOR_KEY.finditer(text)}


@unittest.skipIf(INGESTION is None, "no ingestion tree beside this suite")
class SeedBronzeHasACreator(unittest.TestCase):
    def test_every_bronze_relation_the_generators_clear_has_a_connector(self) -> None:
        from insight_seed.generators.insert import RESET_TARGETS

        assert INGESTION is not None

        namespaces = {db for db, _ in RESET_TARGETS if db.startswith(BRONZE_PREFIX)}
        self.assertTrue(namespaces, "no bronze targets found — has RESET_TARGETS moved?")

        by_namespace = _namespace_to_connector(INGESTION)
        unmapped = sorted(ns for ns in namespaces if ns not in by_namespace)
        self.assertEqual(unmapped, [], f"no connector declares these namespaces: {unmapped}")

        bootstrapped = _bootstrapped_connectors(INGESTION)
        missing = sorted({by_namespace[ns] for ns in namespaces} - bootstrapped)
        self.assertEqual(
            missing,
            [],
            f"the generators write bronze for {missing}, which connectors-config.yaml does "
            f"not list — the seed would fail with 'has no columns'",
        )


if __name__ == "__main__":
    unittest.main()
