#!/usr/bin/env python3
"""Assert what a deploy and a dbt build left on every ClickHouse node of the rig.

Usage:
  check.py snapshot --out counts.json
  check.py assert   --baseline counts.json --probe-rows N --run-results run_results.json
                    [--users presentation] [--columns db.table:column ...]
  check.py relation --name db.table --rows N     a service's table on every node
  check.py absent   --name db.table              a service's table gone from every node
  check.py quorum-boundary --out boundary.json    where every node's query log ends now
  check.py quorum --after boundary.json           the persons sync since then wrote and read with quorum

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
# Created by the deploy hook (migrations and dbt on-run-start hooks) and by the
# destination write, whatever the connector; dbt's own relations come from
# run_results.json.
REQUIRED_RELATIONS = (
    "config.task_field_roles",
    "config.task_value_map",
    "config.ai_seat_tier_map",
    "identity.identity_persons",
    "product_usage.usage_events",
    "ingestion_history.sync_events",
    "silver.contract_version",
    PROBE_TABLE,
    "bronze_github.commits",
    "bronze_github_directory.org_members",
)
# What the seeded github sync must put rows into, layer by layer: the staging
# projection, the silver class and fact, the identity inputs, and gold.
POPULATED_RELATIONS = (
    "staging.github__commits",
    "staging.github__file_changes",
    "staging.github__pull_requests",
    "staging.github__account_emails",
    "silver.class_git_commits",
    "silver.class_git_file_changes",
    "silver.class_git_pull_requests",
    "silver.fct_git_commit",
    "identity.identity_inputs",
    "{gold}.git_authored_commits",
    "{gold}.git_metric_observations",
)
# identity-resolution stages a persons snapshot here before swapping it live.
PERSONS_STAGING_PREFIX = "identity.identity_persons_staging_"
# A dbt-clickhouse intermediate the run should have dropped behind itself.
DBT_LEFTOVER = r"__dbt_(exchange_test|new_data|tmp|backup|distributed)"


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


def owned_databases() -> set[str]:
    return {os.environ["CLICKHOUSE_DATABASE"], *OWNED_DATABASES}


def database_filter() -> str:
    owned = ", ".join(f"'{name}'" for name in sorted(owned_databases()))
    return f"(database IN ({owned}) OR startsWith(database, '{BRONZE_PREFIX}'))"


def databases(url: str) -> dict[str, str]:
    return dict(
        rows(
            url,
            f"SELECT name, engine FROM system.databases "
            f"WHERE name IN ({', '.join(repr(name) for name in sorted(owned_databases()))}) "
            f"OR startsWith(name, '{BRONZE_PREFIX}') ORDER BY 1",
        )
    )


def tables(url: str) -> dict[str, str]:
    found = rows(
        url,
        f"SELECT database || '.' || name, engine FROM system.tables "
        f"WHERE {database_filter()} AND NOT is_temporary ORDER BY 1",
    )
    return dict(found)


def columns(url: str) -> dict[str, str]:
    """Every column of every relation, as database.table.column -> type."""
    return dict(
        rows(
            url,
            f"SELECT database || '.' || table || '.' || name, type FROM system.columns "
            f"WHERE {database_filter()} ORDER BY 1",
        )
    )


def sync_replicas() -> None:
    """Wait for every replica to drain its queue, so direct reads compare final state.

    The destination writes without a quorum, and dbt's quorum covers only what
    dbt wrote; a table local to one node has no replica queue and fails here.
    """
    if not replicated():
        return
    for url in nodes():
        for table, engine in tables(url).items():
            if engine.startswith("Replicated"):
                query(url, f"SYSTEM SYNC REPLICA {table}")


def counts(url: str) -> dict[str, int]:
    """Row counts as a reader sees them: FINAL on a replacing table, where a
    record a later sync re-delivered supersedes the row it had rather than
    joining it."""
    found = {}
    for table, engine in tables(url).items():
        if not engine.endswith("MergeTree"):
            continue
        final = " FINAL" if "Replacing" in engine else ""
        found[table] = int(query(url, f"SELECT count() FROM {table}{final}").strip())
    return found


def snapshot(out: str) -> None:
    sync_replicas()
    per_node = {url: counts(url) for url in nodes()}
    Path(out).write_text(json.dumps(per_node, indent=2, sort_keys=True))
    emit(f"snapshot: {sum(len(c) for c in per_node.values())} table counts across {len(per_node)} node(s)")


def built_relations(run_results: str) -> list[str]:
    """Every relation the dbt run reports as built, as database.name."""
    results = json.loads(Path(run_results).read_text())["results"]
    failed = sorted(f"{result['unique_id']}: {result['status']}" for result in results if result["status"] != "success")
    if failed:
        raise Failure("the dbt run did not build every selected model:\n  " + "\n  ".join(failed))
    return sorted({result["relation_name"].replace("`", "") for result in results if result.get("relation_name")})


def differences(per_node: dict[str, dict[str, str]], what: str) -> list[str]:
    first, *rest = nodes()
    problems = []
    for url in rest:
        problems += [f"{what} only on {first}: {name}" for name in sorted(set(per_node[first]) - set(per_node[url]))]
        problems += [f"{what} only on {url}: {name}" for name in sorted(set(per_node[url]) - set(per_node[first]))]
        problems += sorted(
            f"{what} differs: {name}: {per_node[first][name]} vs {per_node[url][name]}"
            for name in set(per_node[first]) & set(per_node[url])
            if per_node[first][name] != per_node[url][name]
        )
    return problems


def check_databases_and_tables(run_results: str, required_columns: list[str]) -> None:
    """1. Every database and relation exists on every node with the same engine
    and the same columns, including everything the deploy and the dbt run
    created and every column a later sync added."""
    required = sorted({*REQUIRED_RELATIONS, *built_relations(run_results)})
    db_per_node = {url: databases(url) for url in nodes()}
    per_node = {url: tables(url) for url in nodes()}
    columns_per_node = {url: columns(url) for url in nodes()}
    for url in nodes():
        missing = sorted(owned_databases() - set(db_per_node[url]))
        if missing:
            raise Failure(f"{url}: databases missing: {', '.join(missing)}")
        absent = sorted(set(required) - set(per_node[url]))
        if absent:
            raise Failure(f"{url}: relations the deploy or the dbt run creates are missing: {', '.join(absent)}")
        absent = sorted(column for column in required_columns if column not in columns_per_node[url])
        if absent:
            raise Failure(f"{url}: columns a sync added are missing: {', '.join(absent)}")

    problems = (
        differences(db_per_node, "database engine")
        + differences(per_node, "relation")
        + differences(columns_per_node, "column")
    )
    if problems:
        raise Failure("nodes differ:\n  " + "\n  ".join(problems))
    first = nodes()[0]
    emit(
        f"ok  1. {len(db_per_node[first])} database(s), {len(per_node[first])} relation(s) and "
        f"{len(columns_per_node[first])} column(s) identical on {len(per_node)} node(s); {len(required)} required, "
        f"{len(required) - len(REQUIRED_RELATIONS)} of them built by dbt; {len(required_columns)} added column(s)"
    )


def check_reads_complete(probe_rows: int) -> None:
    """2. Rows written through the proxy are all there on every read through it."""
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
    """4. Owned databases are Replicated and bronze is Atomic on a cluster; Bronze
    tables are (Replicated)ReplacingMergeTree; every other MergeTree table follows
    the topology."""
    url = nodes()[0]
    wrong = []
    owned_engine = "Replicated" if replicated() else "Atomic"
    for database, engine in databases(url).items():
        expected = "Atomic" if database.startswith(BRONZE_PREFIX) else owned_engine
        if engine != expected:
            wrong.append(f"database {database}: {engine}, expected {expected}")
    bronze_engine = "ReplicatedReplacingMergeTree" if replicated() else "ReplacingMergeTree"
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
    emit(f"ok  4. every database and MergeTree table follows the {os.environ['RIG_TOPOLOGY']} topology")


def check_counts_agree() -> dict[str, dict[str, int]]:
    """5. Every table holds the same rows on every node."""
    per_node = {url: {table: str(count) for table, count in counts(url).items()} for url in nodes()}
    problems = differences(per_node, "row count")
    if problems:
        raise Failure("row counts differ between nodes:\n  " + "\n  ".join(problems))
    emit(f"ok  5. {len(per_node[nodes()[0]])} table row counts agree on {len(per_node)} node(s)")
    return {url: {table: int(count) for table, count in found.items()} for url, found in per_node.items()}


def check_populated(now: dict[str, dict[str, int]]) -> None:
    """8. The seeded sync reached every layer: each relation holds rows on every node."""
    gold = os.environ["CLICKHOUSE_DATABASE"]
    empty = [
        f"{url}: {name}: {now[url].get(name, 'missing')}"
        for url in nodes()
        for name in (relation.format(gold=gold) for relation in POPULATED_RELATIONS)
        if not now[url].get(name)
    ]
    if empty:
        raise Failure("relations the seeded sync should have filled are empty:\n  " + "\n  ".join(empty))
    emit(f"ok  8. {len(POPULATED_RELATIONS)} relation(s) from staging to gold hold rows on {len(nodes())} node(s)")


def check_no_leftovers() -> None:
    """6. dbt dropped every intermediate relation it staged."""
    leftovers = []
    for url in nodes():
        for (name,) in rows(
            url, f"SELECT database || '.' || name FROM system.tables WHERE match(name, '{DBT_LEFTOVER}') ORDER BY 1"
        ):
            leftovers.append(f"{url}: {name}")
    if leftovers:
        raise Failure("dbt intermediates left behind:\n  " + "\n  ".join(leftovers))
    emit("ok  6. no dbt intermediate relation left behind")


def check_rerun(baseline: str, now: dict[str, dict[str, int]]) -> None:
    """7. The second sync, deploy, dbt build and service run changed no table's row count."""
    before = json.loads(Path(baseline).read_text())
    changed = [
        f"{url} {table}: {count} -> {now[url].get(table, 'gone')}"
        for url, found in before.items()
        for table, count in found.items()
        if now.get(url, {}).get(table) != count
    ]
    if changed:
        raise Failure("the re-run changed row counts:\n  " + "\n  ".join(changed))
    emit(f"ok  7. the re-run left every row count as it was ({sum(len(c) for c in before.values())} checked)")


def engine_follows_topology(engine: str) -> bool:
    return not engine.endswith("MergeTree") or engine.startswith("Replicated") == replicated()


def check_relation(name: str, expected_rows: int) -> None:
    """A table a service created and filled stands on every node, with the
    engine the topology calls for, holding every row the service wrote."""
    sync_replicas()
    for url in nodes():
        engine = tables(url).get(name)
        if engine is None:
            raise Failure(f"{url}: {name} is missing")
        if not engine_follows_topology(engine):
            raise Failure(f"{url}: {name} is {engine}, expected {'a' if replicated() else 'no'} Replicated* engine")
        found = int(query(url, f"SELECT count() FROM {name}").strip())
        if found != expected_rows:
            raise Failure(f"{url}: {name} holds {found} row(s), expected {expected_rows}")
    emit(f"ok  {name} ({tables(nodes()[0])[name]}) holds {expected_rows} row(s) on {len(nodes())} node(s)")


def check_absent(name: str) -> None:
    """A table a service dropped is gone from every node."""
    for url in nodes():
        if name in tables(url):
            raise Failure(f"{url}: {name} still stands")
    emit(f"ok  {name} is gone from {len(nodes())} node(s)")


def quorum_boundary(out: str) -> None:
    """Each node's clock, flushed logs and all: a later quorum check reads only
    what the log gained after this instant."""
    boundary = {}
    for url in nodes():
        query(url, "SYSTEM FLUSH LOGS")
        boundary[url] = int(query(url, "SELECT toUnixTimestamp64Micro(now64(6))").strip())
    Path(out).write_text(json.dumps(boundary))


def check_quorum(after: str) -> None:
    """On a cluster, the persons sync run since the boundary confirmed its
    staging insert on a quorum and read its count back sequentially consistent
    — the pair that makes the count it judges the batch by mean something
    (insight-clickhouse's consistency.rs). Read from the query log of whichever
    replica served each statement; the proxy spreads them."""
    if not replicated():
        emit("ok  quorum settings are not expected on a standalone warehouse")
        return
    boundary = json.loads(Path(after).read_text())
    inserts: list[list[str]] = []
    reads: list[list[str]] = []
    for url in nodes():
        query(url, "SYSTEM FLUSH LOGS")
        found = rows(
            url,
            "SELECT query_kind, Settings['insert_quorum'], Settings['insert_quorum_parallel'], "
            "Settings['select_sequential_consistency'] FROM system.query_log "
            f"WHERE event_time_microseconds > fromUnixTimestamp64Micro({boundary[url]}) "
            # SAFETY: by the statement text, not `tables`: a bare count() names no table there.
            f"AND type = 'QueryFinish' AND is_initial_query AND query LIKE '%{PERSONS_STAGING_PREFIX.split('.')[1]}%' "
            "AND (query_kind = 'Insert' OR query LIKE 'SELECT count() FROM %') ORDER BY event_time_microseconds",
        )
        inserts += [row for row in found if row[0] == "Insert"]
        reads += [row for row in found if row[0] == "Select"]
    if not inserts:
        raise Failure(f"no insert into {PERSONS_STAGING_PREFIX}* in any node's query log since the boundary")
    if not reads:
        raise Failure(f"no read of {PERSONS_STAGING_PREFIX}* in any node's query log since the boundary")
    unconfirmed = [row for row in inserts if (row[1], row[2]) != ("auto", "0")]
    if unconfirmed:
        raise Failure(
            f"{len(unconfirmed)}/{len(inserts)} persons staging insert(s) ran without insert_quorum=auto, insert_quorum_parallel=0"
        )
    stale = [row for row in reads if row[3] != "1"]
    if stale:
        raise Failure(f"{len(stale)}/{len(reads)} persons staging read(s) ran without select_sequential_consistency=1")
    emit(
        f"ok  {len(inserts)} persons staging insert(s) confirmed on a quorum, {len(reads)} read(s) sequentially consistent"
    )


def report(failure: Failure) -> None:
    if os.environ.get("GITHUB_ACTIONS") == "true":
        escaped = str(failure).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        emit(f"::error title=cluster rig::{escaped}", sys.stderr)
    else:
        emit(f"FAIL {failure}", sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    snap = sub.add_parser("snapshot")
    snap.add_argument("--out", required=True)
    check = sub.add_parser("assert")
    check.add_argument("--baseline", required=True)
    check.add_argument("--probe-rows", type=int, required=True)
    check.add_argument("--run-results", required=True, help="the dbt run's run_results.json")
    check.add_argument("--users", default="", help="comma-separated provisioned users to sign in as")
    check.add_argument("--columns", nargs="*", default=[], help="db.table:column a sync added, expected on every node")
    relation = sub.add_parser("relation")
    relation.add_argument("--name", required=True)
    relation.add_argument("--rows", type=int, required=True)
    absent = sub.add_parser("absent")
    absent.add_argument("--name", required=True)
    boundary = sub.add_parser("quorum-boundary")
    boundary.add_argument("--out", required=True)
    quorum = sub.add_parser("quorum")
    quorum.add_argument("--after", required=True, help="the boundary.json quorum-boundary wrote")
    args = parser.parse_args()

    try:
        if args.command == "snapshot":
            snapshot(args.out)
        elif args.command == "relation":
            check_relation(args.name, args.rows)
        elif args.command == "absent":
            check_absent(args.name)
        elif args.command == "quorum-boundary":
            quorum_boundary(args.out)
        elif args.command == "quorum":
            check_quorum(args.after)
        else:
            sync_replicas()
            check_databases_and_tables(args.run_results, [column.replace(":", ".") for column in args.columns])
            check_reads_complete(args.probe_rows)
            check_users([user for user in args.users.split(",") if user])
            check_engines()
            now = check_counts_agree()
            check_no_leftovers()
            check_rerun(args.baseline, now)
            check_populated(now)
    except Failure as failure:
        report(failure)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
