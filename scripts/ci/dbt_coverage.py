#!/usr/bin/env python3
"""dbt_coverage.py — data-contract & test-completeness gate (the path to dbt 100%).

Walks the compiled dbt manifest and asserts that every model in the path to an
user-facing metric (schema silver/gold, plus staging models that feed a
`silver:*` tag-union) carries:

  1. an ENFORCED contract  — config.contract.enforced == true, i.e. declared
     column names + data_types. This is what turns a type drift like issue
     #1318 (Float64 vs Int64) into a BUILD failure instead of a runtime
     ClickHouse NO_COMMON_TYPE error; and
  2. key data tests        — not_null + unique on its key.

"100%" = zero metric-path models below this bar.

Usage:
  dbt_coverage.py [MANIFEST]            # report coverage, exit 0
  dbt_coverage.py [MANIFEST] --check    # exit 1 if any metric-path model has a gap

Needs a compiled manifest (default src/ingestion/dbt/target/manifest.json) —
run `dbt parse` (or any build/compile) in the dbt project first.
"""
import json
import pathlib
import sys

DEFAULT_MANIFEST = pathlib.Path("src/ingestion/dbt/target/manifest.json")


def is_metric_path(node: dict) -> bool:
    """A dbt model on the path to an user-facing metric: silver/gold, the
    silver:* tag-union feeders, AND bronze-promotion views (the first typed
    boundary). NOTE: gold serving tables here are ClickHouse views built by
    analytics-api migrations, not dbt — their contract lives in the API-contract
    track (openapi/schemathesis), not this gate."""
    schema = (node.get("config") or {}).get("schema") or node.get("schema") or ""
    name = node.get("name") or ""
    tags = node.get("tags") or []
    return (
        schema in ("silver", "gold")
        or name.endswith("__bronze_promoted")
        or any(t == "silver" or t.startswith("silver:") for t in tags)
    )


def main() -> None:
    check = "--check" in sys.argv
    positional = [a for a in sys.argv[1:] if not a.startswith("--")]
    manifest_path = pathlib.Path(positional[0]) if positional else DEFAULT_MANIFEST

    if not manifest_path.exists():
        print(f"✗ manifest not found: {manifest_path}")
        print("  compile it first:  (cd src/ingestion/dbt && dbt parse)")
        sys.exit(2)

    manifest = json.loads(manifest_path.read_text())
    nodes = manifest.get("nodes", {})

    # --- Sources: every declared source should carry a freshness check ---
    sources = manifest.get("sources", {})
    if sources:
        fresh = sum(1 for s in sources.values() if s.get("freshness") and s.get("loaded_at_field"))
        print(f"sources: {len(sources)} — freshness declared: {fresh}/{len(sources)} "
              f"({100 * fresh // len(sources)}%)")
        for s in sorted(sources.values(), key=lambda s: s.get("name", "")):
            if not (s.get("freshness") and s.get("loaded_at_field")):
                print(f"  ✗ source {s.get('source_name')}.{s.get('name')}: no freshness check")

    models = {
        uid: n
        for uid, n in nodes.items()
        if n.get("resource_type") == "model" and is_metric_path(n)
    }

    # (model uid, column) -> set of test kinds, so we can check the ACTUAL key
    # column is tested, not just that the model has *some* not_null + *some*
    # unique somewhere (which a model can satisfy on unrelated columns).
    col_tests: dict[tuple, set] = {}
    for n in nodes.values():
        if n.get("resource_type") != "test":
            continue
        kind = (n.get("test_metadata") or {}).get("name") or ""
        col = n.get("column_name") or (
            (n.get("test_metadata") or {}).get("kwargs") or {}
        ).get("column_name")
        if not col:
            continue
        for dep in (n.get("depends_on") or {}).get("nodes", []):
            col_tests.setdefault((dep, col), set()).add(kind)

    def key_columns(node: dict) -> list:
        key = (node.get("config") or {}).get("unique_key")
        if isinstance(key, str):
            return [key]
        if isinstance(key, list) and key:
            return key
        return ["unique_key"]

    gaps, contract_ok, tests_ok = [], 0, 0
    for uid, n in sorted(models.items(), key=lambda kv: kv[1].get("name", "")):
        # contract.enforced lives at the node top level AND under config in
        # current dbt; read both so the check is correct across versions.
        enforced = bool(
            (n.get("contract") or {}).get("enforced")
            or ((n.get("config") or {}).get("contract") or {}).get("enforced")
        )
        keys = key_columns(n)
        has_keys = all(
            "not_null" in col_tests.get((uid, col), set())
            and "unique" in col_tests.get((uid, col), set())
            for col in keys
        )
        contract_ok += enforced
        tests_ok += has_keys
        problems = []
        if not enforced:
            problems.append("no enforced contract")
        if not has_keys:
            problems.append(f"missing not_null+unique on key column(s) {keys}")
        if problems:
            gaps.append((n.get("name"), problems))

    total = len(models)
    print(f"metric-path models: {total}")
    if total:
        print(f"  enforced contract: {contract_ok}/{total} ({100 * contract_ok // total}%)")
        print(f"  key tests        : {tests_ok}/{total} ({100 * tests_ok // total}%)")
    for name, problems in gaps:
        print(f"  ✗ {name}: {', '.join(problems)}")

    if check and gaps:
        print(f"\n✗ data-contract gate FAILED: {len(gaps)} metric-path model(s) below the contract.")
        sys.exit(1)
    if check:
        print("✓ data-contract gate passed — 100% of metric-path models contracted + key-tested")


if __name__ == "__main__":
    main()
