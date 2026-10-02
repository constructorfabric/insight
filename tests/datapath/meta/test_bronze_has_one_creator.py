"""Bronze is created by the connectors, and the harness is not a second creator."""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml
from insight_datapath.ch_seeder import CHSeeder, SeederError
from insight_datapath.instance import InstanceConfig
from insight_datapath.schema import WAREHOUSE_SNAPSHOT

REPO_ROOT = Path(__file__).resolve().parents[3]
CONNECTORS_CONFIG = REPO_ROOT / "src/ingestion/scripts/bootstrap-db/connectors-config.yaml"


def _connector_names() -> set[str]:
    config = yaml.safe_load(CONNECTORS_CONFIG.read_text(encoding="utf-8"))
    return set(config["connectors"])


def test_the_snapshot_the_harness_applies_names_no_connector() -> None:
    """`CREATE TABLE IF NOT EXISTS` means the first creator wins, so a connector file
    applied here would decide the shape before the destination ever ran."""
    overlap = sorted(set(WAREHOUSE_SNAPSHOT) & _connector_names())

    assert not overlap, f"the harness would pre-create bronze for: {overlap}"


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
