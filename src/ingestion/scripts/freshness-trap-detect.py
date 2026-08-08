#!/usr/bin/env python3
"""Detect bronze sources whose `_airbyte_extracted_at` anchor is masking a
trap that the dbt source freshness check cannot see — every nightly sync
makes the anchor look fresh even when the upstream has stopped publishing.

Two detection modes:

1. Full re-emit (heuristic, no per-source config required).
   Fires when ≥ 95% of rows have `_airbyte_extracted_at` within the last
   30h *and* `_airbyte_extracted_at` covers ≤ 2 distinct calendar days
   *and* the table has ≥ 100 rows. Catches connectors that overwrite the
   entire bronze table every run (Confluence-style).

2. Business-date divergence (explicit opt-in via `meta`).
   When a source declares
       meta:
         bronze_business_date_col: <SQL expression returning a DateTime>
   the script compares `MAX(<expr>)` against `MAX(_airbyte_extracted_at)`
   and warns when the business date lags by ≥ 24h while extracted-at is
   fresh. Catches connectors that incrementally top up the latest day's
   row (M365 / Slack / Cursor-style) — the heuristic above can't see these
   because most rows really are old, just `MAX(extract)` happens to be
   today's because of the new top-up row.

A source can opt out of mode 1 (e.g. genuine append-only full-refresh
rosters where neither the heuristic nor a business-date anchor applies)
via `meta: { bronze_freshness_trap_check: skip }` at source or table
level.

Connection params: `CLICKHOUSE_{HOST,PORT,USER,PASSWORD}` env vars (same
as the freshness CronWorkflow). Defaults match the local toolbox setup.

Exit codes:
    0 — no trap suspects
    1 — at least one suspect found (warn, not page)
    2 — script failure (schema parse, etc.)
"""
from __future__ import annotations

import os
import sys
import urllib.parse
import urllib.request
from pathlib import Path

try:
    import yaml
except ImportError:
    print("PyYAML required: pip install pyyaml", file=sys.stderr)
    sys.exit(2)

EXTRACTED_AT_LITERAL = "_airbyte_extracted_at"
RECENT_WINDOW_HOURS = 30
SUSPECT_PCT_RECENT = 95.0
SUSPECT_MAX_DISTINCT_DAYS = 2
MIN_ROWS = 100


class CHQueryError(Exception):
    """Raised when ClickHouse rejects a query (4xx / 5xx). Distinguished from
    transport errors so the caller can decide whether to silently skip
    (database missing in fixture) or escalate."""


CH_BASE_URL: str | None = None


def _resolve_ch_base_url() -> str:
    """Resolve `http://host:port/?user=…&password=…` once at startup.

    Defaults match the local toolbox so the script is runnable from a
    developer shell without exporting anything; in CI / Argo every var is
    set explicitly. Missing vars fall back to defaults — the script logs
    which params came from defaults so a half-configured prod env is
    obvious in workflow logs."""
    host_default, port_default = "host.docker.internal", "8123"
    user_default, password_default = "default", "clickhouse"
    host = os.environ.get("CLICKHOUSE_HOST") or host_default
    port = os.environ.get("CLICKHOUSE_PORT") or port_default
    user = os.environ.get("CLICKHOUSE_USER") or user_default
    password = os.environ.get("CLICKHOUSE_PASSWORD") or password_default
    using_defaults = [
        name for name, used_default in [
            ("CLICKHOUSE_HOST", host == host_default and not os.environ.get("CLICKHOUSE_HOST")),
            ("CLICKHOUSE_PORT", port == port_default and not os.environ.get("CLICKHOUSE_PORT")),
            ("CLICKHOUSE_USER", user == user_default and not os.environ.get("CLICKHOUSE_USER")),
            ("CLICKHOUSE_PASSWORD", password == password_default and not os.environ.get("CLICKHOUSE_PASSWORD")),
        ] if used_default
    ]
    if using_defaults:
        print(
            f"[freshness-trap-detect] using local defaults for: {', '.join(using_defaults)}",
            file=sys.stderr,
        )
    creds = urllib.parse.urlencode({"user": user, "password": password})
    return f"http://{host}:{port}/?{creds}"


def ch_query(sql: str) -> str:
    global CH_BASE_URL
    if CH_BASE_URL is None:
        CH_BASE_URL = _resolve_ch_base_url()
    url = f"{CH_BASE_URL}&{urllib.parse.urlencode({'query': sql})}"
    try:
        with urllib.request.urlopen(url, timeout=30) as resp:
            return resp.read().decode("utf-8").strip()
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        raise CHQueryError(f"HTTP {e.code}: {body[:200]}") from None
    except urllib.error.URLError as e:
        # DNS / refused / TLS / socket-level failures land here. Treat as
        # transport errors so the caller skips the table without crashing
        # the whole script.
        raise CHQueryError(f"URLError: {e.reason}") from None
    except (TimeoutError, OSError) as e:
        # `socket.timeout` was aliased to `TimeoutError` in Py 3.10+;
        # `OSError` covers connection-reset / broken-pipe at the socket
        # layer that bypass the URLError translation.
        raise CHQueryError(f"transport: {e}") from None


def candidate_tables(connectors_root: Path) -> list[dict]:
    """Return one record per bronze table that should be checked.

    Each record has: database, table, source_meta_business_date (str|None),
    table_meta_business_date (str|None), skip_heuristic (bool).

    A table is included if its effective `loaded_at_field` is
    `_airbyte_extracted_at` (no business-date anchor was deliberately
    chosen). Tables opted out via `meta.bronze_freshness_trap_check: skip`
    are excluded entirely. Tables that declare
    `meta.bronze_business_date_col` get the divergence check even when
    they would otherwise be skipped by the heuristic.
    """
    out: list[dict] = []
    for schema_file in sorted(connectors_root.glob("**/dbt/schema.yml")):
        try:
            doc = yaml.safe_load(schema_file.read_text())
        except yaml.YAMLError as e:
            print(f"[freshness-trap-detect] skipping malformed yaml {schema_file}: {e}", file=sys.stderr)
            continue
        if not isinstance(doc, dict):
            print(f"[freshness-trap-detect] skipping non-mapping yaml root {schema_file}", file=sys.stderr)
            continue
        for source in doc.get("sources") or []:
            name = source.get("name", "")
            # dbt convention: if a source omits `schema:`, the source `name`
            # is used as the ClickHouse database. Mirror that here so we
            # query the same DB dbt does.
            schema = source.get("schema", name)
            # Match bronze sources by either dbt name or CH schema (bamboohr
            # uses `name: bamboohr, schema: bronze_bamboohr`).
            is_bronze = (
                isinstance(name, str) and name.startswith("bronze_")
            ) or (
                isinstance(schema, str) and schema.startswith("bronze_")
            )
            if not is_bronze:
                continue
            source_anchor = source.get("loaded_at_field", "").strip()
            source_meta = source.get("meta") or {}
            for table in source.get("tables") or []:
                tname = table.get("name")
                if not tname:
                    continue
                if "freshness" in table and table["freshness"] is None:
                    continue
                anchor = table.get("loaded_at_field", "").strip() or source_anchor
                if anchor != EXTRACTED_AT_LITERAL:
                    continue
                meta = table.get("meta") or {}
                skip = (
                    (meta.get("bronze_freshness_trap_check") or "").lower() == "skip"
                    or (source_meta.get("bronze_freshness_trap_check") or "").lower() == "skip"
                )
                business = (
                    meta.get("bronze_business_date_col")
                    or source_meta.get("bronze_business_date_col")
                )
                # If the table is opted-out and has no business-date hint,
                # there is nothing to check.
                if skip and not business:
                    continue
                out.append({
                    "database": schema,
                    "table": tname,
                    "skip_heuristic": skip,
                    "business_date_col": business,
                })
    return out


def check_table(candidate: dict) -> dict | None:
    """Return a finding dict if the table is a trap suspect, else None."""
    database = candidate["database"]
    table = candidate["table"]
    business = candidate["business_date_col"]
    skip_heuristic = candidate["skip_heuristic"]

    select_parts = [
        "count() AS rows",
        f"countIf(_airbyte_extracted_at >= now() - INTERVAL {RECENT_WINDOW_HOURS} HOUR) AS recent",
        "uniqExact(toDate(_airbyte_extracted_at)) AS distinct_days",
        "max(_airbyte_extracted_at) AS last_extract",
    ]
    if business:
        select_parts.append(f"max({business}) AS last_business")
    sql = (
        "SELECT " + ", ".join(select_parts)
        + f" FROM `{database}`.`{table}` "
        "FORMAT TabSeparated"
    )
    try:
        raw = ch_query(sql)
    except CHQueryError as e:
        # Common in fixtures: bronze_<x> database does not exist locally.
        # In prod the same response means a connector was renamed / dropped
        # without a schema.yml update — surface it on stderr so workflow
        # logs catch the misconfig instead of swallowing it.
        if "Code: 81" in str(e) or "UNKNOWN_DATABASE" in str(e) or "HTTP 404" in str(e):
            print(f"  [{database}.{table}] missing in ClickHouse (skipping): {e}", file=sys.stderr)
            return None
        print(f"  [{database}.{table}] query failed: {e}", file=sys.stderr)
        return None
    if not raw:
        return None
    fields = raw.split("\t")
    rows = int(fields[0])
    if rows < MIN_ROWS:
        return None
    recent = int(fields[1])
    distinct_days = int(fields[2])
    last_extract = fields[3]
    pct_recent = 100.0 * recent / rows
    last_business = fields[4] if business and len(fields) >= 5 else None

    finding: dict | None = None

    if not skip_heuristic and (
        pct_recent >= SUSPECT_PCT_RECENT
        and distinct_days <= SUSPECT_MAX_DISTINCT_DAYS
    ):
        finding = {
            "kind": "full-reemit",
            "database": database,
            "table": table,
            "rows": rows,
            "pct_recent": round(pct_recent, 1),
            "distinct_days": distinct_days,
            "last_extract": last_extract,
        }

    if business and last_business and last_business not in ("\\N", "1970-01-01 00:00:00"):
        # Compare gap. `_airbyte_extracted_at` arrives with milliseconds
        # (e.g. '2026-05-04 02:01:58.805') which `toDateTime` cannot parse —
        # use `parseDateTime64BestEffortOrNull` for both sides.
        diff_sql = (
            "SELECT dateDiff('hour', "
            f"parseDateTime64BestEffortOrNull('{last_business}'), "
            f"parseDateTime64BestEffortOrNull('{last_extract}'))"
            " FORMAT TabSeparated"
        )
        try:
            diff_h = int(ch_query(diff_sql))
        except (CHQueryError, ValueError):
            diff_h = 0
        if diff_h >= 24:
            finding = {
                "kind": "incremental-topup",
                "database": database,
                "table": table,
                "rows": rows,
                "last_extract": last_extract,
                "last_business": last_business,
                "extract_minus_business_h": diff_h,
                "business_date_col": business,
            }
    return finding


def main(argv: list[str]) -> int:
    root = Path(argv[1]) if len(argv) >= 2 else Path("src/ingestion/connectors")
    if not root.is_dir():
        print(f"connectors root not found: {root}", file=sys.stderr)
        return 2

    candidates = candidate_tables(root)
    if not candidates:
        print("no candidate sources (every bronze source already uses a non-extracted-at anchor or is opted out)")
        return 0

    print(f"checking {len(candidates)} bronze table(s) anchored on `_airbyte_extracted_at`...")
    suspects: list[dict] = []
    for c in candidates:
        result = check_table(c)
        if result:
            suspects.append(result)

    if not suspects:
        print("no trap suspects")
        return 0

    full_reemit = [s for s in suspects if s["kind"] == "full-reemit"]
    incremental = [s for s in suspects if s["kind"] == "incremental-topup"]

    if full_reemit:
        print(f"\n{len(full_reemit)} full re-emit suspect(s):")
        print(
            f"  ≥ {SUSPECT_PCT_RECENT:.0f}% of rows have `_airbyte_extracted_at` within the "
            f"last {RECENT_WINDOW_HOURS}h, across ≤ {SUSPECT_MAX_DISTINCT_DAYS} distinct day(s). "
            "The connector is rewriting the entire table on every run — "
            "`_airbyte_extracted_at` will look fresh forever, even when the "
            "upstream stops publishing. Switch `loaded_at_field` to a real "
            "business-date column."
        )
        for s in full_reemit:
            print(
                f"  - {s['database']}.{s['table']}: "
                f"rows={s['rows']}, pct_recent={s['pct_recent']}%, "
                f"distinct_extract_days={s['distinct_days']}, "
                f"last_extract={s['last_extract']}"
            )

    if incremental:
        print(f"\n{len(incremental)} incremental top-up suspect(s):")
        print(
            "  `MAX(_airbyte_extracted_at)` is fresh but the declared "
            "business-date column is at least 24h behind. The connector is "
            "writing today's row but the upstream's reported business date "
            "has not advanced — the freshness check sees a green anchor that "
            "has nothing to do with reality. Switch `loaded_at_field` to the "
            "business-date expression you used in `meta`."
        )
        for s in incremental:
            print(
                f"  - {s['database']}.{s['table']}: "
                f"last_extract={s['last_extract']}, "
                f"last_business={s['last_business']}, "
                f"gap={s['extract_minus_business_h']}h "
                f"(business_date_col={s['business_date_col']})"
            )

    print(
        "\nLegitimate full-refresh roster tables (no daily cadence, no "
        "business-date column) can opt out per-table:\n"
        "    meta:\n"
        "      bronze_freshness_trap_check: skip"
    )
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
