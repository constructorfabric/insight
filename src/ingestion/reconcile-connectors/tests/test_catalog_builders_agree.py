"""The three catalog builders must key bronze the same way.

`normalize_catalog.py` builds the catalog production connections carry,
`bootstrap-db/create-connector-tables.sh` builds the one the committed DDL
snapshot is generated from, and the declarative-connector dev tool builds the
one a connector author runs locally. The destination reads that key to decide
the table's ORDER BY, so a builder that keys a stream differently creates a
bronze table production never has -- and the difference surfaces only as a
relation that silently never dedups.

They are three languages (Python, jq, inline Python in shell), so the
agreement is asserted over the text each one emits.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

from pathlib import Path

import pytest

INGESTION = Path(__file__).resolve().parents[2]

BUILDERS = {
    "reconcile (production connections)": (
        INGESTION / "reconcile-connectors/python/normalize_catalog.py",
        '"primaryKey": [[UNIQUE_KEY]]',
        "_stream_has_unique_key",
    ),
    "bootstrap-db (snapshot generation)": (
        INGESTION / "scripts/bootstrap-db/create-connector-tables.sh",
        'primary_key: [["unique_key"]]',
        'has("unique_key") | not',
    ),
    "declarative-connector (dev rig)": (
        INGESTION / "tools/declarative-connector/generate-catalog.sh",
        "stream_entry['primary_key'] = [['unique_key']]",
        "'unique_key' not in",
    ),
}


@pytest.mark.parametrize(("label", "spec"), BUILDERS.items(), ids=list(BUILDERS))
def test_every_builder_keys_a_stream_on_unique_key(label: str, spec: tuple) -> None:
    """The dedup key is the same in all three or the table shapes diverge."""
    path, key_literal, _guard = spec
    assert key_literal in path.read_text(encoding="utf-8"), f"should key on unique_key: {label}"


@pytest.mark.parametrize(("label", "spec"), BUILDERS.items(), ids=list(BUILDERS))
def test_no_builder_defers_to_a_source_declared_primary_key(label: str, spec: tuple) -> None:
    """A source's own primary key orders the table by whatever the vendor
    considers unique, which is not the identity stamp bronze dedups on."""
    path, _key_literal, _guard = spec
    assert "source_defined_primary_key" not in path.read_text(encoding="utf-8"), (
        f"should ignore the source-declared key: {label}"
    )


@pytest.mark.parametrize(("label", "spec"), BUILDERS.items(), ids=list(BUILDERS))
def test_every_builder_refuses_a_stream_without_unique_key(label: str, spec: tuple) -> None:
    """A keyless stream is an authoring bug. Every builder fails on it rather
    than emitting a catalog that lands an ever-duplicating table."""
    path, _key_literal, guard = spec
    assert guard in path.read_text(encoding="utf-8"), f"should refuse a keyless stream: {label}"
