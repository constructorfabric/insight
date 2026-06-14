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

Connects over the ClickHouse HTTP interface (stdlib only). Env:
  CH_HOST (default 127.0.0.1)  CH_PORT (8123)  CH_USER (default)  CH_PASSWORD

Usage:
  data_presence_audit.py                       # report only, exit 0
  data_presence_audit.py --check               # fail (exit 1) on DUPLICATES
  data_presence_audit.py --check --fail-on-empty --fail-on-stale
  data_presence_audit.py --max-age-hours 36 --waive-empty bronze_slack,bronze_zoom

Designed to run nightly against the deployed environment and as a gate on the
production sync DAG. Query logic validated against the kind-insight cluster.
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
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
    req = urllib.request.Request(
        f"http://{host}:{port}/",
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


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="exit 1 on a violation")
    ap.add_argument("--fail-on-empty", action="store_true")
    ap.add_argument("--fail-on-stale", action="store_true")
    ap.add_argument("--max-age-hours", type=int, default=48)
    ap.add_argument("--waive-empty", default="", help="comma list of bronze DBs / silver tables allowed to be empty")
    args = ap.parse_args()
    waived = {w.strip() for w in args.waive_empty.split(",") if w.strip()}

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

    # summary + gate
    print(
        f"\nsummary: empty={len(fail_empty)} duplicated={len(fail_dup)} "
        f"stale={len(fail_stale)}"
    )
    violations = list(fail_dup)
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
