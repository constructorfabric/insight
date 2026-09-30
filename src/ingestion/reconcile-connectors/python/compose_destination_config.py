#!/usr/bin/env python3
"""Compose the Bronze ClickHouse destination's connectionConfiguration.

CLI:
  compose_destination_config.py
Env:
  RECONCILE_DEST_CLICKHOUSE_{HOST,PORT,DATABASE,USERNAME,PASSWORD}  required
  RECONCILE_DEST_CLICKHOUSE_PROTOCOL                               http|https
  RECONCILE_DEST_CLICKHOUSE_{CLUSTER_MODE,CLUSTER_NAME}            topology
Stdout: the connectionConfiguration as JSON.
Exit:   0 on success, 1 when a required variable is missing.

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


def topology(cluster_mode: str, cluster_name: str) -> dict[str, object]:
    """The replication keys, or nothing at all while the install is standalone.

    `use_replicated_engines` prefixes every created engine with `Replicated`;
    `cluster_name` appends `ON CLUSTER` to CREATE / ALTER / EXCHANGE / DROP and
    is unnecessary when the target database itself uses the `Replicated`
    database engine — hence an empty name under an enabled flag.
    """
    if cluster_mode.strip().lower() not in _TRUTHY:
        return {}

    fields: dict[str, object] = {"use_replicated_engines": True}
    if cluster_name:
        fields["cluster_name"] = cluster_name

    return fields


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

    print(json.dumps(compose(os.environ)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
