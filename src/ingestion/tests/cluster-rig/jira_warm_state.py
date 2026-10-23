#!/usr/bin/env python3
"""The jira bronze a warm install carries into a deploy, and what the heals owe it.

Usage:
  jira_warm_state.py plant    through RIG_PROXY, over the tables the destination made
  jira_warm_state.py assert   on every RIG_NODES node directly
  jira_warm_state.py drop     take the jira bronze and the catalogue record away again

Every jira heal in apply-ch-migrations.sh is guarded on a bronze_jira table and
no-ops on an install that never synced jira. The rig's destination write makes
the tables, empty and in today's shape; `plant` then turns them into what a
connector older than 6.1.0 left behind, so that each heal has work to do on
the deploy that follows, and `assert` reads the outcome the warm-upgrade lane
expects (.github/workflows/scripts/warm-upgrade-sim.sh) on every node:

- jira_issue: `jira_id` typed as a number (pre-#1866), the one issue keyed by
  both keys it has had -> retyped to a string, collapsed onto its id;
- jira_comments: no `jira_id` column at all -> added back, then filled;
- jira_issue_history: `items` double-encoded -> unescaped, and `jira_id`
  filled from the issue streams, the row naming a key no stream delivered
  staying NULL, the row that already had an id keeping it;
- jira_worklogs: Jira's own `issueId` preferred over the key;
- staging.jira__catalogue_first_seen: the pre-history shape, a plain
  MergeTree keyed by source -> rebuilt as a ReplacingMergeTree keyed by tenant
  and source, with the recorded instants kept.

The connector's own models are not part of the rig's dbt run, and a silver
class whose only contributor is one of them cannot be built with the bronze
standing; `drop` takes it away once the heals have been judged.

Runs in the dbt venv, like check.py, whose helpers it shares.
"""

from __future__ import annotations

import os
import sys

import check

TENANT = "warm"
SOURCE = "jira-1"
# A substream filled from the issue streams, the comment resolving through
# jira_issue_keys, the worklog preferring Jira's own issueId, and the row count
# the whole rebuild must keep; then jira_issue collapsed onto its id.
EXPECTED_IDENTITY = "101|101|NULL|999|101|102|777|101|8|1|warm-jira-1-101"
EXPECTED_CATALOGUE = "-jira-gone,,jira-gone,2026-01-02 00:00:00.000;warm-jira-1,warm,jira-1,2026-01-01 00:00:00.000"


def settings() -> dict[str, object]:
    """The statements go through the proxy; on a cluster the rows must stand on
    every replica before the heals read them, whichever replica answers."""
    if not check.replicated():
        return {}
    return {"insert_quorum": "auto", "insert_quorum_parallel": 0, "alter_sync": 2, "mutations_sync": 2}


def command(sql: str) -> None:
    proxy = os.environ["RIG_PROXY"]
    try:
        check.client(proxy, os.environ["CLICKHOUSE_USER"], os.environ["CLICKHOUSE_PASSWORD"]).command(
            sql, settings=settings()
        )
    except check.ClickHouseError as error:
        raise check.Failure(f"{proxy}: {error}") from error


def plant() -> None:
    prefix = "Replicated" if check.replicated() else ""
    statements = [
        # pre-#1866: the id came out of a JSON number.
        "ALTER TABLE bronze_jira.jira_issue MODIFY COLUMN jira_id Nullable(Int64)",
        # pre-6.1.0: the substreams carried no id.
        "ALTER TABLE bronze_jira.jira_comments DROP COLUMN jira_id",
        f"""INSERT INTO bronze_jira.jira_issue
            (unique_key, tenant_id, source_id, id_readable, jira_id, _airbyte_raw_id, _airbyte_extracted_at)
            VALUES
            ('warm-jira-WARM-1', '{TENANT}', '{SOURCE}', 'WARM-1', 101, 'raw-i1', '2026-01-02 00:00:00'),
            ('warm-jira-WARM-OLD-7', '{TENANT}', '{SOURCE}', 'WARM-OLD-7', 101, 'raw-i0', '2026-01-01 00:00:00')""",
        f"""INSERT INTO bronze_jira.jira_issue_keys
            (unique_key, tenant_id, source_id, id_readable, jira_id, _airbyte_raw_id, _airbyte_extracted_at)
            VALUES ('warm-jira-WARM-2', '{TENANT}', '{SOURCE}', 'WARM-2', '102', 'raw-k1', '2026-01-02 00:00:00')""",
        f"""INSERT INTO bronze_jira.jira_issue_history
            (unique_key, tenant_id, source_id, id_readable, jira_id, items, _airbyte_raw_id, _airbyte_extracted_at)
            VALUES
            ('warm-jira-h-current', '{TENANT}', '{SOURCE}', 'WARM-1', NULL, '"[]"', 'raw-h1', '2026-01-03 00:00:00'),
            ('warm-jira-h-moved', '{TENANT}', '{SOURCE}', 'WARM-OLD-7', NULL, '[]', 'raw-h2', '2026-01-03 00:00:00'),
            ('warm-jira-h-unknown', '{TENANT}', '{SOURCE}', 'WARM-GONE-9', NULL, '[]', 'raw-h3', '2026-01-03 00:00:00'),
            ('warm-jira-h-known', '{TENANT}', '{SOURCE}', 'WARM-1', '999', '[]', 'raw-h4', '2026-01-03 00:00:00'),
            ('warm-jira-h-recent', '{TENANT}', '{SOURCE}', 'WARM-1', NULL, '[]', 'raw-h5', now())""",
        f"""INSERT INTO bronze_jira.jira_comments
            (unique_key, tenant_id, source_id, id_readable, body, _airbyte_raw_id, _airbyte_extracted_at)
            VALUES ('warm-jira-c-keys', '{TENANT}', '{SOURCE}', 'WARM-2', 'comment', 'raw-c1', '2026-01-03 00:00:00')""",
        f"""INSERT INTO bronze_jira.jira_worklogs
            (unique_key, tenant_id, source_id, id_readable, jira_id, issueId, _airbyte_raw_id, _airbyte_extracted_at)
            VALUES
            ('warm-jira-w-own', '{TENANT}', '{SOURCE}', 'WARM-GONE-9', NULL, '777', 'raw-w1', '2026-01-03 00:00:00'),
            ('warm-jira-w-key', '{TENANT}', '{SOURCE}', 'WARM-1', NULL, NULL, 'raw-w2', '2026-01-03 00:00:00')""",
        f"""INSERT INTO bronze_jira.jira_fields
            (unique_key, tenant_id, source_id, _airbyte_raw_id, _airbyte_extracted_at)
            VALUES ('warm-jira-1-summary', '{TENANT}', '{SOURCE}', 'raw-f1', '2026-03-01 00:00:00')""",
        # The pre-history catalogue record: staging is a Replicated database on
        # a cluster, so the CREATE reaches every replica on its own.
        "DROP TABLE IF EXISTS staging.jira__catalogue_first_seen",
        f"""CREATE TABLE staging.jira__catalogue_first_seen
            (insight_source_id String, catalogue_first_sync DateTime64(3), recorded_at DateTime64(3))
            ENGINE = {prefix}MergeTree ORDER BY insight_source_id""",
        """INSERT INTO staging.jira__catalogue_first_seen (insight_source_id, catalogue_first_sync, recorded_at)
            VALUES
            ('jira-1', '2026-01-01 00:00:00', '2026-01-01 00:05:00'),
            ('jira-gone', '2026-01-02 00:00:00', '2026-01-02 00:05:00')""",
    ]
    for sql in statements:
        command(sql)
    check.sync_replicas()
    check.emit(
        f"planted the pre-6.1.0 jira bronze and the pre-history catalogue record through {os.environ['RIG_PROXY']}"
    )


def assert_healed() -> None:
    check.sync_replicas()
    for url in check.nodes():
        identity = check.query(
            url,
            """SELECT concat(
                ifNull(maxIf(jira_id, unique_key = 'warm-jira-h-current'), 'NULL'), '|',
                ifNull(maxIf(jira_id, unique_key = 'warm-jira-h-moved'), 'NULL'), '|',
                ifNull(maxIf(jira_id, unique_key = 'warm-jira-h-unknown'), 'NULL'), '|',
                ifNull(maxIf(jira_id, unique_key = 'warm-jira-h-known'), 'NULL'), '|',
                ifNull(maxIf(jira_id, unique_key = 'warm-jira-h-recent'), 'NULL'), '|',
                ifNull(maxIf(jira_id, unique_key = 'warm-jira-c-keys'), 'NULL'), '|',
                ifNull(maxIf(jira_id, unique_key = 'warm-jira-w-own'), 'NULL'), '|',
                ifNull(maxIf(jira_id, unique_key = 'warm-jira-w-key'), 'NULL'), '|',
                toString(count()), '|',
                toString((SELECT count() FROM bronze_jira.jira_issue FINAL)), '|',
                ifNull((SELECT anyLast(unique_key) FROM bronze_jira.jira_issue FINAL), 'NULL'))
            FROM (
                SELECT unique_key, jira_id FROM bronze_jira.jira_issue_history FINAL
                UNION ALL SELECT unique_key, jira_id FROM bronze_jira.jira_comments FINAL
                UNION ALL SELECT unique_key, jira_id FROM bronze_jira.jira_worklogs FINAL
            )""",
        ).strip()
        if identity != EXPECTED_IDENTITY:
            raise check.Failure(
                f"{url}: jira issue identity after the heals is {identity!r}, expected {EXPECTED_IDENTITY!r}"
            )
        shape = check.query(
            url,
            """SELECT concat(
                (SELECT type FROM system.columns WHERE database = 'bronze_jira' AND table = 'jira_issue' AND name = 'jira_id'), '|',
                toString((SELECT count() FROM system.columns WHERE database = 'bronze_jira' AND table = 'jira_comments' AND name = 'jira_id')), '|',
                (SELECT ifNull(items, 'NULL') FROM bronze_jira.jira_issue_history FINAL WHERE unique_key = 'warm-jira-h-current'))""",
        ).strip()
        if shape != "Nullable(String)|1|[]":
            raise check.Failure(
                f"{url}: jira bronze schema after the heals is {shape!r}, expected "
                "'Nullable(String)|1|[]' (jira_id retyped, jira_comments.jira_id added, items unescaped)"
            )
        catalogue = check.query(
            url,
            """SELECT concat(
                (SELECT engine FROM system.tables WHERE database = 'staging' AND name = 'jira__catalogue_first_seen'), '|',
                arrayStringConcat(groupArray(row), ';'))
            FROM (
                SELECT concat(unique_key, ',', tenant_id, ',', insight_source_id, ',', toString(catalogue_first_sync)) AS row
                FROM staging.jira__catalogue_first_seen FINAL ORDER BY unique_key
            )""",
        ).strip()
        prefix = "Replicated" if check.replicated() else ""
        expected = f"{prefix}ReplacingMergeTree|{EXPECTED_CATALOGUE}"
        if catalogue != expected:
            raise check.Failure(f"{url}: jira catalogue record after the heal is {catalogue!r}, expected {expected!r}")
    check.emit(
        f"ok  jira heals: issue id recovered, bronze schema healed and the catalogue record rebuilt on {len(check.nodes())} node(s)"
    )


def drop() -> None:
    on_cluster = f"ON CLUSTER {os.environ['CLICKHOUSE_CLUSTER_NAME']}" if check.replicated() else ""
    command(f"DROP DATABASE IF EXISTS bronze_jira {on_cluster} SYNC")
    command("DROP TABLE IF EXISTS staging.jira__catalogue_first_seen SYNC")
    check.emit("dropped the jira bronze and the catalogue record")


def main() -> int:
    try:
        if sys.argv[1:] == ["plant"]:
            plant()
        elif sys.argv[1:] == ["assert"]:
            assert_healed()
        elif sys.argv[1:] == ["drop"]:
            drop()
        else:
            check.emit(__doc__ or "", sys.stderr)
            return 2
    except check.Failure as failure:
        check.report(failure)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
