"""How a run learns which ClickHouse topology its expectations are read under."""

from __future__ import annotations

import pytest
from insight_datapath.topology import CLUSTER_MODE_VARIABLE, Topology, from_environment


def test_a_run_that_says_nothing_is_reading_a_single_node() -> None:
    assert from_environment({}) == Topology.STANDALONE


@pytest.mark.parametrize("flag", ["1", "true", "TRUE", " yes ", "on"])
def test_the_harness_declares_a_cluster_the_way_every_other_consumer_reads_it(flag: str) -> None:
    """The chart hands this flag to the deploy hook, dbt and reconcile as the same
    string; a suite that accepted a narrower set would disagree with the creators."""
    assert from_environment({CLUSTER_MODE_VARIABLE: flag}) == Topology.REPLICATED


@pytest.mark.parametrize("flag", ["", "  ", "false", "0", "no", "off", "cluster"])
def test_anything_that_is_not_the_flag_leaves_the_run_on_a_single_node(flag: str) -> None:
    assert from_environment({CLUSTER_MODE_VARIABLE: flag}) == Topology.STANDALONE
