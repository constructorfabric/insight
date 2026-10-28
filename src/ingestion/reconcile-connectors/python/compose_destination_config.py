#!/usr/bin/env python3
"""Compose the Bronze ClickHouse destination's connectionConfiguration.

CLI:
  compose_destination_config.py
Env:
  RECONCILE_DEST_CLICKHOUSE_{HOST,PORT,DATABASE,USERNAME,PASSWORD}  required
  RECONCILE_DEST_CLICKHOUSE_PROTOCOL                               http|https
  RECONCILE_DEST_CLICKHOUSE_{CLUSTER_MODE,CLUSTER_NAME}            topology
Stdout: the connectionConfiguration as JSON.
Exit:   0 on success, 1 when a required variable is missing or the topology
        names a replicated install without a cluster.

destination-clickhouse 2.x (Bulk-CDK) rewrote the spec: `port` is a STRING,
`protocol` is required, and the removed `ssl` / `schema` keys now yield a 422.
The topology keys arrived in 2.1.29 and are left out entirely on a standalone
install, because a destination whose spec does not declare a key rejects it.
"""

from __future__ import annotations

import json
import os
import sys
from collections.abc import Mapping

_PREFIX = "RECONCILE_DEST_CLICKHOUSE_"
_REQUIRED = ("HOST", "PORT", "DATABASE", "USERNAME", "PASSWORD")
_TRUTHY = frozenset({"1", "true", "yes", "on"})


class TopologyError(ValueError):
    """A topology the destination would accept and then build the wrong tables from."""


def topology(cluster_mode: str, cluster_name: str) -> dict[str, object]:
    """The replication keys, or nothing at all while the install is standalone.

    `use_replicated_engines` prefixes every created engine with `Replicated`;
    `cluster_name` appends `ON CLUSTER` to CREATE / ALTER / EXCHANGE / DROP.
    The two travel together: the destination creates `bronze_*` as plain
    Atomic databases, where a `Replicated*` table without `ON CLUSTER` exists
    on the one replica that ran the DDL and every other replica reports it
    missing.
    """
    if cluster_mode.strip().lower() not in _TRUTHY:
        return {}

    name = cluster_name.strip()
    if not name:
        raise TopologyError(
            f"{_PREFIX}CLUSTER_MODE is on but {_PREFIX}CLUSTER_NAME is empty. The destination "
            "creates bronze_* as Atomic databases, so a Replicated* table reaches the other "
            f"replicas only through ON CLUSTER <name>; set {_PREFIX}CLUSTER_NAME to the cluster "
            "in the server's remote_servers (clickhouse.clusterName in the chart), or turn "
            f"{_PREFIX}CLUSTER_MODE off for a standalone install."
        )

    return {"use_replicated_engines": True, "cluster_name": name}


def compose(env: Mapping[str, str]) -> dict[str, object]:
    return {
        "host": env[_PREFIX + "HOST"],
        "port": env[_PREFIX + "PORT"],
        "protocol": env.get(_PREFIX + "PROTOCOL", "http"),
        "database": env[_PREFIX + "DATABASE"],
        "username": env[_PREFIX + "USERNAME"],
        "password": env[_PREFIX + "PASSWORD"],
        "enable_json": False,
        **topology(env.get(_PREFIX + "CLUSTER_MODE", ""), env.get(_PREFIX + "CLUSTER_NAME", "")),
    }


def main() -> int:
    missing = [_PREFIX + name for name in _REQUIRED if not os.environ.get(_PREFIX + name)]
    if missing:
        print(f"compose_destination_config: missing {', '.join(missing)}", file=sys.stderr)
        return 1

    try:
        config = compose(os.environ)
    except TopologyError as refusal:
        print(f"compose_destination_config: {refusal}", file=sys.stderr)
        return 1

    print(json.dumps(config))
    return 0


if __name__ == "__main__":
    sys.exit(main())
