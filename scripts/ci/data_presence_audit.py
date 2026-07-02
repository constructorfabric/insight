#!/usr/bin/env python3
"""data_presence_audit.py — does the fetch actually write correct, deduped, fresh
data to ClickHouse?

Three checks the per-PR test pyramid can't answer (they need the live warehouse):

  1. PRESENCE  — every connected source's bronze DB has rows (> 0), and every
                 silver `class_*` metric table is non-empty. A wired-but-empty
                 source (or a silver model that produced nothing) means blank
                 dashboards with no error — the most common silent failure here.
  2. DEDUP     — every silver ReplacingMergeTree table returns the same row
                 count with and without FINAL. A mismatch means un-merged
                 duplicates are present and any reader without FINAL (e.g. some
                 gold views) will over-count, and the number drifts as merges
                 run. (See the "same period, different data" class.)
  3. FRESHNESS — the newest row per populated silver table is within the SLA.
                 Stale data = the sync stopped and nobody noticed.
  4. RESOLUTION — every gold serving view (`insight.*`, the objects analytics-api
                 selects from) resolves against the *current* silver schema. Gold
                 views are built by analytics-api migrations; silver by dbt — the
                 two can drift. A view that references a silver column the deployed
                 schema doesn't have (e.g. silver rebuilt out of lockstep) throws
                 at query time, analytics-api returns 500, and the dashboard renders
                 the section as a blank "No data" — a server error disguised as an
                 empty period. This check SELECTs `LIMIT 0` from each view so a
                 broken one surfaces here instead of in a user's dashboard.

Connects over the ClickHouse HTTP interface (stdlib only). Env:
  CH_HOST (default 127.0.0.1)  CH_PORT (8123)  CH_USER (default)  CH_PASSWORD

Usage:
  data_presence_audit.py                       # report only, exit 0
  data_presence_audit.py --check               # fail (exit 1) on DUPLICATES
  data_presence_audit.py --check --fail-on-empty --fail-on-stale
  data_presence_audit.py --max-age-hours 36 --waive-empty bronze_slack,bronze_zoom

WHERE THIS RUNS — this is a *live-warehouse monitor*, not a per-PR test.
Every property here is true by construction on a freshly seeded e2e/CI database
(data is always present, deduped, and "now"), so the checks only earn their keep
against the real accumulated warehouse. Two distinct invocations:

  * The e2e rig (e2e-bronze-to-api): RESOLUTION + DEDUP only — these hold on any
    *populated* schema, so they catch gold↔silver drift and un-merged duplicates
    regardless of dataset. They require a DB that has actually been `dbt build`-ed,
    which the e2e rig does (bronze → silver) before asserting. Run against a blank
    ClickHouse they pass vacuously and prove nothing — so this is deliberately NOT
    wired as a standalone job against a throwaway service container.
        data_presence_audit.py --check          # resolution+dedup
  * Nightly against the deployed environment (the real home): add presence and
    freshness — "a connected source wrote 0 rows" and "newest row older than the
    SLA" only mean "the real sync stopped" against live data, never in seeded CI.
        data_presence_audit.py --check --fail-on-empty --fail-on-stale

So --fail-on-empty / --fail-on-stale are opt-in on purpose: leave them OFF on the
e2e rig, turn them ON only for the scheduled deployed run + the post-sync DAG gate.
Query logic validated against the kind-insight cluster.
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
import urllib.error
import urllib.request


def ch(query: str) -> str:
    """Run a query and return the raw response.

    Two transports:
      - CH_EXEC_POD set → `kubectl exec <pod> -- clickhouse-client` (native, no
        password — for a local/kind cluster). CH_EXEC_NS defaults to `insight`.
      - otherwise → ClickHouse HTTP interface via CH_HOST/PORT/USER/PASSWORD
        (for CI against the deployed environment).
    """
    pod = os.environ.get("CH_EXEC_POD")
    if pod:
        ns = os.environ.get("CH_EXEC_NS", "insight")
        proc = subprocess.run(
            ["kubectl", "-n", ns, "exec", pod, "-c", "clickhouse", "--",
             "clickhouse-client", "--query", query],
            capture_output=True, text=True, timeout=120, check=False,
        )
        if proc.returncode != 0:
            raise RuntimeError(proc.stderr.strip() or "kubectl exec failed")
        return proc.stdout.strip()

    host = os.environ.get("CH_HOST", "127.0.0.1")
    port = os.environ.get("CH_PORT", "8123")
    # Never send credentials over cleartext: default to https whenever a password
    # is set (deployed warehouse); plain http only for the password-less local/CI
    # container. CH_SCHEME overrides if an operator really has a plaintext endpoint.
    scheme = os.environ.get("CH_SCHEME") or ("https" if os.environ.get("CH_PASSWORD") else "http")
    req = urllib.request.Request(
        f"{scheme}://{host}:{port}/",
        data=query.encode(),
        headers={
            "X-ClickHouse-User": os.environ.get("CH_USER", "default"),
            "X-ClickHouse-Key": os.environ.get("CH_PASSWORD", ""),
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:  # noqa: S310 (trusted internal CH)
        return resp.read().decode().strip()


def rows(query: str) -> list[list[str]]:
    out = ch(query + " FORMAT TSV")
    return [line.split("\t") for line in out.splitlines()] if out else []


def _view_error(name: str) -> str | None:
    """SELECT LIMIT 0 from a gold view; return the first error line, or None if it resolves.

    `LIMIT 0` still forces ClickHouse to name-resolve and type-check the whole
    view body, so an unresolved column / missing source table surfaces as an
    analysis exception without scanning any rows.
    """
    try:
        ch(f"SELECT * FROM insight.{name} LIMIT 0")
        return None
    except urllib.error.HTTPError as he:  # HTTP transport: CH error is in the body
        body = he.read().decode(errors="replace") if hasattr(he, "read") else str(he)
        return _clickhouse_error_line(body) or f"HTTP {he.code}"
    except Exception as e:  # noqa: BLE001 — kubectl transport puts the CH error in stderr
        return _clickhouse_error_line(str(e)) or "query failed"


def _clickhouse_error_line(raw: str) -> str:
    """Pull the meaningful line out of a ClickHouse error.

    ClickHouse prefixes errors with a "Received exception from server (version …):"
    banner; the actual cause ("Code: 47. DB::Exception: Identifier 'c.lines_added'
    cannot be resolved …") is on the next line. Return that, not the banner.
    """
    lines = [ln.strip() for ln in raw.splitlines() if ln.strip()]
    detail = next((ln for ln in lines if "DB::Exception" in ln or ln.startswith("Code:")), "")
    return (detail or (lines[0] if lines else ""))[:240]


def check_views() -> list[str]:
    """Every gold serving view in `insight` must resolve against the live schema."""
    print("== gold serving views (resolve against current silver schema) ==")
    broken = []
    view_rows = rows(
        "SELECT name FROM system.tables WHERE database='insight' "
        "AND engine LIKE '%View%' ORDER BY name"
    )
    for (name, *_rest) in view_rows:
        err = _view_error(name)
        if err:
            print(f"  BROKEN insight.{name}: {err}")
            broken.append(name)
        else:
            print(f"  ok     insight.{name}")
    return broken


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="exit 1 on a violation")
    ap.add_argument("--fail-on-empty", action="store_true")
    ap.add_argument("--fail-on-stale", action="store_true")
    ap.add_argument("--max-age-hours", type=int, default=48)
    ap.add_argument("--waive-empty", default="", help="comma list of bronze DBs / silver tables allowed to be empty")
    args = ap.parse_args()
    waived = {w.strip() for w in args.waive_empty.split(",") if w.strip()}

    # This tool runs against a real, populated warehouse (e2e rig or deployed) —
    # if it can't reach ClickHouse, that's a hard failure, not something to skip.
    try:
        ch("SELECT 1")
    except Exception as e:  # noqa: BLE001
        print(f"FATAL: cannot reach ClickHouse ({e}). Set CH_HOST/CH_PORT/CH_USER/CH_PASSWORD.")
        sys.exit(2)

    fail_empty, fail_dup, fail_stale = [], [], []

    # 1. PRESENCE — bronze per source
    print("== bronze write-path (rows per source) ==")
    for db, n in rows(
        r"SELECT database, sum(total_rows) FROM system.tables "
        r"WHERE database LIKE 'bronze\_%' GROUP BY database ORDER BY database"
    ):
        n = int(n or 0)
        empty = n == 0 and db not in waived
        print(f"  {'EMPTY' if empty else 'ok   '} {db}: {n} rows")
        if empty:
            fail_empty.append(db)

    # 1+2+3 — silver metric tables
    print("== silver metric tables (presence · dedup · freshness) ==")
    for name, total, engine in rows(
        r"SELECT name, total_rows, engine FROM system.tables "
        r"WHERE database='silver' AND name LIKE 'class\_%' ORDER BY name"
    ):
        total = int(total or 0)
        if total == 0:
            tag = "WAIVED" if name in waived else "EMPTY"
            print(f"  {tag:6} {name}: 0 rows")
            if name not in waived:
                fail_empty.append(f"silver.{name}")
            continue

        # dedup: ReplacingMergeTree must read the same with and without FINAL
        dup = ""
        if "ReplacingMergeTree" in engine:
            raw = int(ch(f"SELECT count() FROM silver.{name}") or 0)
            fin = int(ch(f"SELECT count() FROM silver.{name} FINAL") or 0)
            if raw != fin:
                dup = f"  DUPLICATES raw={raw} final={fin} (+{raw - fin})"
                fail_dup.append(name)

        # freshness: newest _version (epoch-ms) if the column exists
        stale = ""
        try:
            age_h = ch(
                f"SELECT round((now() - toDateTime(max(_version)/1000)) / 3600, 1) "
                f"FROM silver.{name}"
            )
            if age_h and float(age_h) > args.max_age_hours:
                stale = f"  STALE {age_h}h (SLA {args.max_age_hours}h)"
                fail_stale.append(name)
        except Exception:  # noqa: BLE001
            age_h = "n/a"  # versionless RMT (e.g. class_people) — skip freshness

        flag = "DUP " if dup else ("STALE" if stale else "ok   ")
        print(f"  {flag} {name}: {total} rows{dup}{stale}")

    # 4. RESOLUTION — gold serving views resolve against the live silver schema
    broken_views = check_views()

    # summary + gate
    print(
        f"\nsummary: empty={len(fail_empty)} duplicated={len(fail_dup)} "
        f"stale={len(fail_stale)} broken_views={len(broken_views)}"
    )
    # A broken gold view = a dashboard section serves HTTP 500 (rendered as a blank
    # "No data"), so it is always a hard violation under --check — never opt-in.
    violations = list(fail_dup) + list(broken_views)
    if args.fail_on_empty:
        violations += fail_empty
    if args.fail_on_stale:
        violations += fail_stale
    if args.check and violations:
        print(f"\nFAILED: {violations}")
        sys.exit(1)
    if args.check:
        print("\nPASSED")


if __name__ == "__main__":
    main()
