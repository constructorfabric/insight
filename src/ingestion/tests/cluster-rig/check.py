#!/usr/bin/env python3
"""Assert what a deploy left on every ClickHouse node of the rig.

Usage:
  check.py snapshot --out counts.json
  check.py assert   --baseline counts.json [--probe-rows N] [--users presentation]
  check.py gaps     --expect-failure-from hook.log

Runs in the dbt venv, whose dbt-clickhouse brings clickhouse-connect.

Env:
  RIG_TOPOLOGY      replicated | standalone
  RIG_NODES         comma-separated node URLs, each queried DIRECTLY
  RIG_PROXY         the URL every product client uses (round-robin on a cluster)
  CLICKHOUSE_USER, CLICKHOUSE_PASSWORD, CLICKHOUSE_DATABASE
  RIG_USER_PASSWORD_<USER>  the password a provisioned user signs in with

Every comparison reads each node directly, never through the proxy: through it,
a table on one replica answers half the time and a check passes by luck.
"""

from __future__ import annotations

import argparse
import functools
import json
import os
import re
import sys
from pathlib import Path
from typing import TextIO
from urllib.parse import urlparse

import clickhouse_connect
from clickhouse_connect.driver.client import Client
from clickhouse_connect.driver.exceptions import ClickHouseError

# INVARIANT: create-databases.sh DATABASES, minus the app database named by env.
OWNED_DATABASES = (
    "staging",
    "silver",
    "identity",
    "config",
    "presentation",
    "product_usage",
    "ingestion_history",
    "insight_datasets",
)
BRONZE_PREFIX = "bronze_"
PROBE_TABLE = "bronze_cluster_rig.probe"
PROBE_READS = 20
# What ClickHouse says when DDL meets a relation one replica lacks or holds plain.
REJECTED_RELATION = re.compile(
    r"Could not find table: `?(?P<missing>[A-Za-z_]\w*(?:\.\w+)?)`?"
    r"|Table `?(?P<absent>[A-Za-z_]\w*(?:\.\w+)?)`? does not exist"
    r"|Table `?(?P<plain>[A-Za-z_]\w*(?:\.\w+)?)`?(?: \([0-9a-f-]+\))? is not replicated"
)
# Created by every deploy either leg runs: the dbt on-run-start hooks and Bronze.
REQUIRED_RELATIONS = (
    "config.task_field_roles",
    "config.task_value_map",
    "config.ai_seat_tier_map",
    "identity.identity_persons",
    PROBE_TABLE,
)


class Failure(Exception):
    pass


def emit(line: str, stream: TextIO | None = None) -> None:
    (stream or sys.stdout).write(line + "\n")


@functools.cache
def client(url: str, user: str, password: str) -> Client:
    address = urlparse(url)
    # INVARIANT: no session id — a session lives on one replica, and the proxy
    # must stay free to send every statement to either.
    return clickhouse_connect.get_client(
        host=address.hostname,
        port=address.port,
        username=user,
        password=password,
        autogenerate_session_id=False,
        send_receive_timeout=60,
    )


def query(url: str, sql: str, *, user: str | None = None, password: str | None = None) -> str:
    try:
        connection = client(
            url,
            user or os.environ["CLICKHOUSE_USER"],
            password if password is not None else os.environ["CLICKHOUSE_PASSWORD"],
        )
        return connection.raw_query(sql).decode()
    except ClickHouseError as error:
        raise Failure(f"{url}: {error}") from error


def rows(url: str, sql: str) -> list[list[str]]:
    return [line.split("\t") for line in query(url, sql + " FORMAT TSV").splitlines() if line]


def nodes() -> list[str]:
    return [node for node in os.environ["RIG_NODES"].split(",") if node]


def replicated() -> bool:
    return os.environ["RIG_TOPOLOGY"] == "replicated"


def database_filter() -> str:
    owned = ", ".join(f"'{name}'" for name in (os.environ["CLICKHOUSE_DATABASE"], *OWNED_DATABASES))
    return f"(database IN ({owned}) OR startsWith(database, '{BRONZE_PREFIX}'))"


def tables(url: str) -> dict[str, str]:
    found = rows(
        url,
        f"SELECT database || '.' || name, engine FROM system.tables "
        f"WHERE {database_filter()} AND NOT is_temporary ORDER BY 1",
    )
    return dict(found)


def merge_tree_tables(url: str) -> list[str]:
    return sorted(name for name, engine in tables(url).items() if engine.endswith("MergeTree"))


def snapshot(out: str) -> None:
    counts = {}
    for url in nodes():
        counts[url] = {
            table: int(query(url, f"SELECT count() FROM {table}").strip()) for table in merge_tree_tables(url)
        }
    Path(out).write_text(json.dumps(counts, indent=2, sort_keys=True))
    emit(f"snapshot: {sum(len(c) for c in counts.values())} table counts across {len(counts)} node(s)")


def check_databases_and_tables() -> None:
    """1. Every database and table exists on every node, with the same engine."""
    expected = {os.environ["CLICKHOUSE_DATABASE"], *OWNED_DATABASES}
    per_node = {}
    for url in nodes():
        present = {name for (name,) in rows(url, "SELECT name FROM system.databases")}
        missing = sorted(expected - present)
        if missing:
            raise Failure(f"{url}: databases missing: {', '.join(missing)}")
        per_node[url] = tables(url)
        absent = sorted(set(REQUIRED_RELATIONS) - set(per_node[url]))
        if absent:
            raise Failure(f"{url}: relations every deploy creates are missing: {', '.join(absent)}")

    first, *rest = nodes()
    for url in rest:
        only_first = sorted(set(per_node[first]) - set(per_node[url]))
        only_other = sorted(set(per_node[url]) - set(per_node[first]))
        engines = sorted(
            f"{table}: {per_node[first][table]} vs {per_node[url][table]}"
            for table in set(per_node[first]) & set(per_node[url])
            if per_node[first][table] != per_node[url][table]
        )
        problems = [
            *(f"only on {first}: {table}" for table in only_first),
            *(f"only on {url}: {table}" for table in only_other),
            *(f"engine differs: {line}" for line in engines),
        ]
        if problems:
            raise Failure("relations differ between replicas:\n  " + "\n  ".join(problems))
    emit(f"ok  1. {len(per_node[first])} relation(s) identical on {len(per_node)} node(s)")


def check_reads_complete(probe_rows: int) -> None:
    """2. Rows written through the proxy are all there on every read through it."""
    if replicated():
        # INVARIANT: the destination writes without a quorum; a table local to
        # one node has no replica queue and fails here instead.
        for url in nodes():
            query(url, f"SYSTEM SYNC REPLICA {PROBE_TABLE}")

    proxy = os.environ["RIG_PROXY"]
    answers = [int(query(proxy, f"SELECT count() FROM {PROBE_TABLE} FINAL").strip()) for _ in range(PROBE_READS)]
    short = [count for count in answers if count != probe_rows]
    if short:
        raise Failure(
            f"{PROBE_TABLE}: {len(short)}/{PROBE_READS} reads through the proxy were not {probe_rows}: {answers}"
        )
    emit(f"ok  2. {PROBE_READS}/{PROBE_READS} reads through the proxy returned all {probe_rows} rows")


def check_users(users: list[str]) -> None:
    """3. Every provisioned user signs in on every node and holds its role there."""
    for user in users:
        password = os.environ.get(f"RIG_USER_PASSWORD_{user.upper()}")
        if not password:
            raise Failure(f"RIG_USER_PASSWORD_{user.upper()} is unset; the rig cannot sign in as {user}")
        grants = {}
        for url in nodes():
            signed_in = query(url, "SELECT currentUser()", user=user, password=password).strip()
            if signed_in != user:
                raise Failure(f"{url}: signed in as {signed_in!r}, expected {user!r}")
            roles = sorted(
                line.strip()
                for line in query(
                    url, "SELECT role_name FROM system.current_roles", user=user, password=password
                ).splitlines()
            )
            if not roles:
                raise Failure(f"{url}: {user} signs in but holds no role")
            grants[url] = (roles, sorted(query(url, f"SHOW GRANTS FOR {', '.join(roles)}").splitlines()))
        if len({json.dumps(value) for value in grants.values()}) != 1:
            raise Failure(f"{user}: roles or grants differ between nodes: {json.dumps(grants, indent=2)}")
        emit(f"ok  3. {user} signs in on {len(grants)} node(s) with {', '.join(grants[nodes()[0]][0])}")


def check_engines() -> None:
    """4. Bronze is (Replicated)ReplacingMergeTree; every other table follows the topology."""
    url = nodes()[0]
    bronze_engine = "ReplicatedReplacingMergeTree" if replicated() else "ReplacingMergeTree"
    wrong = []
    for table, engine in tables(url).items():
        if not engine.endswith("MergeTree"):
            continue
        if table.startswith(BRONZE_PREFIX):
            if engine != bronze_engine:
                wrong.append(f"{table}: {engine}, expected {bronze_engine}")
        elif engine.startswith("Replicated") != replicated():
            wrong.append(f"{table}: {engine}, expected {'a' if replicated() else 'no'} Replicated* engine")
    if wrong:
        raise Failure("engines do not follow the topology:\n  " + "\n  ".join(sorted(wrong)))
    emit(f"ok  4. every MergeTree table follows the {os.environ['RIG_TOPOLOGY']} topology")


def check_rerun(baseline: str) -> None:
    """5. A second deploy changed no table's row count."""
    before = json.loads(Path(baseline).read_text())
    changed = []
    for url in nodes():
        for table, count in before.get(url, {}).items():
            now = int(query(url, f"SELECT count() FROM {table}").strip())
            if now != count:
                changed.append(f"{url} {table}: {count} -> {now}")
    if changed:
        raise Failure("the re-run changed row counts:\n  " + "\n  ".join(changed))
    emit(f"ok  5. the re-run left every row count as it was ({sum(len(c) for c in before.values())} checked)")


def not_ready() -> list[tuple[str, str, str]]:
    """Relations missing from a replica, or MergeTree without replication: (name, engine, where)."""
    per_node = {url: tables(url) for url in nodes()}
    found = []
    for table in sorted(set().union(*per_node.values())):
        on = [url for url in nodes() if table in per_node[url]]
        engine = per_node[on[0]][table]
        everywhere = len(on) == len(nodes())
        if everywhere and (not engine.endswith("MergeTree") or engine.startswith("Replicated")):
            continue
        found.append((table, engine, "every node" if everywhere else ", ".join(on)))
    return found


def rejected_relations(hook_log: str) -> set[str]:
    """The relations ClickHouse named when it rejected the hook's DDL."""
    errors = [line for line in Path(hook_log).read_text().splitlines() if "DB::Exception" in line]

    named = set()
    for line in errors:
        for match in REJECTED_RELATION.finditer(line):
            named.update(name for name in match.groupdict().values() if name)
    return named


def gaps(hook_log: str) -> None:
    gap = not_ready()
    emit("Relations not yet cluster-ready (T11 / T12 / T23 of #2010):")
    for table, engine, where in gap:
        emit(f"  {table:<70} {engine:<28} on {where}")
    emit(f"{len(gap)} relation(s)")

    rejected = rejected_relations(hook_log)
    names = {table for table, _, _ in gap} | {table.split(".", 1)[1] for table, _, _ in gap}
    if not rejected & names:
        raise Failure(
            "the deploy hook failed, but not on a relation that is not cluster-ready "
            f"(ClickHouse named: {', '.join(sorted(rejected)) or 'nothing'}); the failure is new"
        )
    emit(f"the hook failed on {', '.join(sorted(rejected & names))}, which is not cluster-ready")


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    snap = sub.add_parser("snapshot")
    snap.add_argument("--out", required=True)
    check = sub.add_parser("assert")
    check.add_argument("--baseline", required=True)
    check.add_argument("--probe-rows", type=int, required=True)
    check.add_argument("--users", default="", help="comma-separated provisioned users to sign in as")
    expected_failure = sub.add_parser("gaps")
    expected_failure.add_argument("--expect-failure-from", required=True, help="the failed deploy hook's log")
    args = parser.parse_args()

    try:
        if args.command == "snapshot":
            snapshot(args.out)
        elif args.command == "gaps":
            gaps(args.expect_failure_from)
        else:
            check_databases_and_tables()
            check_reads_complete(args.probe_rows)
            check_users([user for user in args.users.split(",") if user])
            check_engines()
            check_rerun(args.baseline)
    except Failure as failure:
        if os.environ.get("GITHUB_ACTIONS") == "true":
            escaped = str(failure).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
            emit(f"::error title=cluster rig::{escaped}", sys.stderr)
        else:
            emit(f"FAIL {failure}", sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
