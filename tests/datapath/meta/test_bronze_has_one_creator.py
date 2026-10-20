"""Bronze is created by the connectors, and the harness is not a second creator."""

from __future__ import annotations

from pathlib import Path

import pytest
from insight_datapath.ch_seeder import CHSeeder, SeederError
from insight_datapath.instance import InstanceConfig

REPO_ROOT = Path(__file__).resolve().parents[3]
SCHEMA = REPO_ROOT / "tests/lib/insight_datapath/schema.py"


def test_the_harness_pre_creates_no_relation_at_all() -> None:
    """`CREATE TABLE IF NOT EXISTS` means the first creator wins, so anything this
    harness applied ahead of the destination would decide the shape instead of it.
    The harness now creates databases and runs the connectors, nothing more."""
    body = SCHEMA.read_text(encoding="utf-8")

    assert "connectors-ddl" not in body, "the harness applies a DDL snapshot again"
    assert "CREATE TABLE" not in body.upper(), "the harness creates a relation of its own"


def test_a_table_no_connector_created_is_refused_rather_than_invented(
    instance_cfg: InstanceConfig,
) -> None:
    """An invented table is a plain MergeTree keyed on nothing: it deduplicates nothing,
    and the spec above it passes over a shape no deployment has."""
    seeder = CHSeeder(instance_cfg)
    table = "bronze_nothing.no_such_stream"
    schemas = {table: {"properties": {"unique_key": {"type": "string"}}}}

    with pytest.raises(SeederError, match="does not exist"):
        seeder.seed_bronze({table: []}, schemas)
