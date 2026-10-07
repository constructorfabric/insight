"""Which ClickHouse topology a run reads its expectations under.

A clustered install creates `Replicated*` engines where a single node creates the
plain family, so a gate that compares an engine must know which install it is
looking at. The chart carries that one decision to every creator as
`CLICKHOUSE_CLUSTER_MODE` (`charts/insight/README.md`, "ClickHouse topology"), and
a suite that must know it reads the same variable from its harness.

Read from the environment rather than from the server: a rule derived from what
the warehouse happens to hold cannot fail on a warehouse that holds the wrong
thing, which is the whole job of the gates above this.
"""

from __future__ import annotations

import os
from collections.abc import Mapping
from enum import Enum

CLUSTER_MODE_VARIABLE = "CLICKHOUSE_CLUSTER_MODE"

_TRUTHY = frozenset({"1", "true", "yes", "on"})


class Topology(Enum):
    """Whether the relations this run inspects were created to replicate."""

    STANDALONE = "standalone"
    REPLICATED = "replicated"


def from_environment(environ: Mapping[str, str] | None = None) -> Topology:
    """The topology the harness declared, a single node unless it said otherwise.

    Unset is standalone because that is every local and CI instance today; a run
    that forgets to declare a cluster therefore asserts the single-node shape and
    fails loudly, rather than accepting both.
    """
    env = os.environ if environ is None else environ
    flag = (env.get(CLUSTER_MODE_VARIABLE) or "").strip().lower()

    return Topology.REPLICATED if flag in _TRUTHY else Topology.STANDALONE
