"""`dev-compose.sh` must create exactly the bronze the generators write into.

The DDL snapshot no longer carries bronze, so the only thing that creates those
tables is `destination-clickhouse`, run per connector before the seed container
starts. `dev-compose.sh` names the connectors to run; the generators name the
relations they write. Nothing links the two at runtime — a generator that starts
writing a new connector's bronze simply fails the seed with "has no columns" —
so the link is asserted here.
"""

from __future__ import annotations

import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[5]
DEV_COMPOSE = ROOT / "dev-compose.sh"
CONNECTORS = ROOT / "src/ingestion/connectors"

BRONZE_PREFIX = "bronze_"

_ARRAY = re.compile(r"^SEED_BRONZE_CONNECTORS=\((?P<names>[^)]*)\)", re.MULTILINE)
_NAMESPACE = re.compile(r"^\s*namespace:\s*(?P<ns>\S+)\s*$", re.MULTILINE)


def _declared_connectors() -> set[str]:
    """The connector names `dev-compose.sh` runs before seeding."""
    match = _ARRAY.search(DEV_COMPOSE.read_text(encoding="utf-8"))
    assert match, "SEED_BRONZE_CONNECTORS is not declared in dev-compose.sh"
    return set(match.group("names").split())


def _connector_for_namespace() -> dict[str, str]:
    """Each connector's bronze database, as its descriptor declares it."""
    mapping: dict[str, str] = {}
    for descriptor in sorted(CONNECTORS.glob("*/*/descriptor.yaml")):
        found = _NAMESPACE.search(descriptor.read_text(encoding="utf-8"))
        if found:
            mapping[found.group("ns").strip("\"'")] = descriptor.parent.name
    return mapping


class SeedBronzeConnectors(unittest.TestCase):
    def test_every_bronze_relation_the_generators_clear_has_a_connector_to_create_it(self) -> None:
        from insight_seed.generators.insert import RESET_TARGETS

        namespaces = {db for db, _ in RESET_TARGETS if db.startswith(BRONZE_PREFIX)}
        by_namespace = _connector_for_namespace()

        unmapped = sorted(ns for ns in namespaces if ns not in by_namespace)
        self.assertEqual(unmapped, [], f"no connector declares these namespaces: {unmapped}")

        needed = {by_namespace[ns] for ns in namespaces}
        missing = sorted(needed - _declared_connectors())
        self.assertEqual(
            missing,
            [],
            f"the generators write bronze for {missing}, which dev-compose.sh does not "
            f"create — add them to SEED_BRONZE_CONNECTORS or the seed fails with "
            f"'has no columns'",
        )

    def test_no_connector_is_created_for_bronze_nothing_writes(self) -> None:
        """Each name costs a container run on every seed, so an unused one is waste."""
        from insight_seed.generators.insert import RESET_TARGETS

        namespaces = {db for db, _ in RESET_TARGETS if db.startswith(BRONZE_PREFIX)}
        by_namespace = _connector_for_namespace()
        needed = {by_namespace[ns] for ns in namespaces if ns in by_namespace}

        extra = sorted(_declared_connectors() - needed)
        self.assertEqual(extra, [], f"no generator writes bronze for: {extra}")


if __name__ == "__main__":
    unittest.main()
