"""The inputs the removal passes read, as typed fixtures.

Typed rather than bare dicts because these records decide deletions: a key
spelled wrong in a test is a record the reader silently skips, and the test
would still pass while proving nothing.
"""

from __future__ import annotations

import json
from collections.abc import Iterable
from typing import NamedTuple


class Source(NamedTuple):
    """One entry of the `sources/list` payload."""

    name: str
    source_id: str
    #: Empty leaves the field out entirely, which is the listing that makes
    #: ownership fall back to the name.
    definition_id: str = ""

    def payload(self) -> dict[str, str]:
        record = {"name": self.name, "sourceId": self.source_id}
        if self.definition_id:
            record["sourceDefinitionId"] = self.definition_id
        return record


class Definition(NamedTuple):
    """One source definition, the way this loop publishes them (ADR-0009)."""

    connector: str
    definition_id: str

    def payload(self) -> dict[str, object]:
        return {
            "name": self.connector,
            "sourceDefinitionId": self.definition_id,
            "custom": True,
        }


def listing(records: Iterable[Source] | Iterable[Definition]) -> str:
    """The JSON array a reader is handed."""
    return json.dumps([record.payload() for record in records])


def plan_row(connector: str, source_id: str = "", secret: str = "") -> str:
    """One `disc_load_instances` row: eight descriptor columns, three instance.

    The instance columns are empty for a descriptor no Secret names, which is
    what the removal passes read as "not installed here".
    """
    namespace = "bronze_" + connector.replace("-", "_")
    return "\t".join(
        [connector, "dir", "1", "nocode", "", "", "", namespace, source_id, secret, "hash"]
    )
