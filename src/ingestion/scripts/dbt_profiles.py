"""Write the dbt profiles.yml for a workflow step from env vars.

Building the file here instead of a shell heredoc keeps user input out of
YAML/shell text — every value rides the environment (see the SECURITY notes
in charts/insight/templates/ingestion/dbt-run.yaml). pyyaml is not declared
by this package: the script only runs in the toolbox image, where dbt-core
already ships it.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

import yaml

TRUTHY = frozenset({"1", "true", "yes", "on"})


def on_cluster() -> str:
    """The cluster the adapter's `cluster` key names, or nothing.

    Empty on a standalone install, and on a cluster whose database carries the
    `Replicated` engine: that one distributes DDL itself and needs no
    `ON CLUSTER` clause. A name without the flag names no cluster — the flag
    is what turns on the replicated engines the clause would qualify.
    """
    if os.environ.get("CLICKHOUSE_CLUSTER_MODE", "").strip().lower() not in TRUTHY:
        return ""

    return os.environ.get("CLICKHOUSE_CLUSTER_NAME", "").strip()


def build_profile(correlated_subqueries: bool) -> dict:
    output = {
        "type": "clickhouse",
        "host": os.environ["CLICKHOUSE_HOST"],
        "port": int(os.environ["CLICKHOUSE_PORT"]),
        "schema": "silver",
        "user": os.environ["CLICKHOUSE_USER"],
        "password": os.environ["CLICKHOUSE_PASSWORD"],
        "secure": False,
        "send_receive_timeout": 1500,
        "query_limit": 0,
        "connect_timeout": 30,
    }

    cluster_name = on_cluster()
    if cluster_name:
        output["cluster"] = cluster_name

    if correlated_subqueries:
        # Correlated subqueries (LEFT ANTI JOIN in the identity seed models)
        # are gated behind this experimental flag on CH 25.7. A model-level
        # config() setting does NOT reach the SELECT plan in dbt-clickhouse,
        # so it must sit at profile level. Parity with test/bootstrap.
        output["settings"] = {"allow_experimental_correlated_subqueries": 1}

    return {"ingestion": {"target": "k8s", "outputs": {"k8s": output}}}


def main(argv: list[str]) -> int:
    profile = build_profile(correlated_subqueries="--correlated-subqueries" in argv)
    with Path("profiles.yml").open("w") as handle:
        yaml.safe_dump(profile, handle)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
