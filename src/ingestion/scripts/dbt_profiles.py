"""The one writer of every throwaway dbt profiles.yml.

Each step that runs dbt — the Argo dbt-run and data-quality steps, the deploy
hook, bootstrap, the sample seeder, the two transform rigs, the data-path
harness — picks its own target name and warehouse and takes the body from here,
so the connection shape and the ClickHouse topology (epic #2010) are stated
once instead of once per step.

Building the file here instead of a shell heredoc keeps user input out of
YAML/shell text — every value rides the environment (see the SECURITY notes
in charts/insight/templates/ingestion/dbt-run.yaml). pyyaml is not declared
by this package: the script only runs where dbt-core already ships it.

A model's ENGINE is not here. The adapter reads it from `config.get('engine')`
alone, so a profile-level `engine:` key is accepted and dropped; the project
states the default in `src/ingestion/dbt/dbt_project.yml` and each model
overrides it through the `insight_engine` macro.
"""

from __future__ import annotations

import argparse
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlparse

import yaml

TRUTHY = frozenset({"1", "true", "yes", "on"})

DEFAULT_SCHEMA = "silver"
SEND_RECEIVE_TIMEOUT_SECS = 1500
CONNECT_TIMEOUT_SECS = 30

_DEFAULT_PORT_BY_SCHEME = {"https": 8443, "http": 8123}


@dataclass(frozen=True)
class Connection:
    """Which warehouse a step's profile points at."""

    host: str
    port: int
    user: str
    password: str
    secure: bool = False
    schema: str = DEFAULT_SCHEMA


def cluster_mode() -> bool:
    """Whether the install declared its warehouse replicated."""
    return (os.environ.get("CLICKHOUSE_CLUSTER_MODE") or "").strip().lower() in TRUTHY


def on_cluster() -> str:
    """The cluster the adapter's `cluster` key names, or nothing.

    Empty on a standalone install. A name without the flag names no cluster —
    the flag is what turns on the replicated engines the clause would qualify.
    The flag without a name is refused by the chart, so no name arrives here
    while one is needed.
    """
    if not cluster_mode():
        return ""

    return (os.environ.get("CLICKHOUSE_CLUSTER_NAME") or "").strip()


def session_settings(*, correlated_subqueries: bool) -> dict[str, object]:
    """The settings every statement of the run carries.

    `custom_settings` is the adapter's only profile-level settings key: a plain
    `settings:` block is dropped, and a model's own `settings=` reaches the
    table's DDL rather than the session.
    """
    settings: dict[str, object] = {}

    if correlated_subqueries:
        # Correlated subqueries (LEFT ANTI JOIN in the identity seed models)
        # are gated behind this experimental flag on CH 25.7. A model-level
        # config() setting does NOT reach the SELECT plan in dbt-clickhouse,
        # so it must sit at session level.
        settings["allow_experimental_correlated_subqueries"] = 1

    if cluster_mode():
        # SAFETY: a build reads what the step before it wrote. Without a quorum
        # on the write and sequential consistency on the read, a replica that
        # has not caught up answers the next model with a partial table.
        settings["insert_quorum"] = "auto"
        settings["select_sequential_consistency"] = 1

    return settings


def build_output(connection: Connection, *, correlated_subqueries: bool, threads: int = 1) -> dict[str, object]:
    """One target's output block: the connection, plus whatever the topology adds."""
    output: dict[str, object] = {
        "type": "clickhouse",
        "threads": threads,
        "host": connection.host,
        "port": connection.port,
        "schema": connection.schema,
        "user": connection.user,
        "password": connection.password,
        "secure": connection.secure,
        "send_receive_timeout": SEND_RECEIVE_TIMEOUT_SECS,
        "connect_timeout": CONNECT_TIMEOUT_SECS,
    }

    cluster_name = on_cluster()
    if cluster_name:
        output["cluster"] = cluster_name

    settings = session_settings(correlated_subqueries=correlated_subqueries)
    if settings:
        output["custom_settings"] = settings

    return output


def build_profile(
    target: str, connection: Connection, *, correlated_subqueries: bool, threads: int = 1
) -> dict[str, object]:
    """The whole profiles.yml body: one named target over one connection."""
    output = build_output(connection, correlated_subqueries=correlated_subqueries, threads=threads)

    return {"ingestion": {"target": target, "outputs": {target: output}}}


def connection_from_env() -> Connection:
    """The warehouse, from whichever variables the calling step already sets.

    `CLICKHOUSE_URL` is the deploy hook's and the seeder's shape; the Argo steps
    and bootstrap set `CLICKHOUSE_HOST` / `CLICKHOUSE_PORT` and say `https`
    through `CLICKHOUSE_PROTOCOL`.
    """
    url = (os.environ.get("CLICKHOUSE_URL") or "").strip()

    if url:
        parsed = urlparse(url)
        if not parsed.hostname:
            raise ValueError(f"CLICKHOUSE_URL names no host: {url!r}")
        scheme = parsed.scheme or "http"
        return Connection(
            host=parsed.hostname,
            port=parsed.port or _DEFAULT_PORT_BY_SCHEME.get(scheme, 8123),
            user=os.environ["CLICKHOUSE_USER"],
            password=os.environ["CLICKHOUSE_PASSWORD"],
            secure=scheme == "https",
        )

    return Connection(
        host=os.environ["CLICKHOUSE_HOST"],
        port=int(os.environ["CLICKHOUSE_PORT"]),
        user=os.environ["CLICKHOUSE_USER"],
        password=os.environ["CLICKHOUSE_PASSWORD"],
        secure=(os.environ.get("CLICKHOUSE_PROTOCOL") or "http").strip().lower() == "https",
    )


def main(argv: list[str]) -> int:
    """Write `<--profiles-dir>/profiles.yml` for the step named by `--target`."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", default="k8s", help="profile target name to write")
    parser.add_argument("--profiles-dir", default=".", type=Path, help="directory to write profiles.yml into")
    parser.add_argument("--correlated-subqueries", action="store_true")
    args = parser.parse_args(argv)

    profile = build_profile(args.target, connection_from_env(), correlated_subqueries=args.correlated_subqueries)

    with (args.profiles_dir / "profiles.yml").open("w") as handle:
        yaml.safe_dump(profile, handle)

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
