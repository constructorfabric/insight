#!/usr/bin/env python3
"""Silver-contract consistency gate (connectors <-> silver class).

Catches cross-connector schema drift *at PR time*, without a warehouse:

    Connectors (e.g. github, bitbucket-cloud) feed one silver class via
    `union_by_tag('silver:class_git_commits')` -> `SELECT * ... UNION ALL`.
    A dev adds/removes/reorders/retypes a column in one connector's staging
    model and forgets the others. On a cluster the UNION ALL then fails
    (ClickHouse `Code: 386 NO_COMMON_TYPE` / column-count mismatch) or — worse,
    when two same-typed columns swap positions — silently cross-wires data.
    PR CI never catches it because it never materialises every connector at once.

Opt-in by design. A silver class is checked ONLY when its silver model declares
`contract: enforced: true`. That contract is the single source of truth: every
connector tagged `silver:<class>` MUST emit exactly the columns the silver model
declares — same NAMES, same ORDER, same data_types — and must itself carry an
enforced contract (so dbt build keeps its declared columns honest; this gate
only compares declarations). Classes without an enforced silver contract are
reported and skipped, so contracts roll out one class at a time.

ORDER matters: `union_by_tag` emits positional `SELECT * ... UNION ALL`, and dbt
contracts do not enforce column order, so two connectors with the same column
SET but a different ORDER would pass a set-only check yet cross-wire columns at
runtime. This gate compares the ordered column list.

Convention: the silver class model for tag `silver:<class>` is the model named
`<class>` (e.g. tag `silver:class_git_commits` -> model `class_git_commits`).

Source of truth is the dbt manifest (`dbt parse` -> target/manifest.json), fully
offline.

Usage:
    dbt parse  # produces target/manifest.json, no DB connection needed
    python scripts/check_silver_contract_parity.py \
        --manifest src/ingestion/dbt/target/manifest.json

Exit code 0 = all enforced silver classes consistent. 1 = drift found (CI fails).
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

SILVER_TAG_PREFIX = "silver:"


def load_manifest(path: Path) -> dict:
    if not path.exists():
        sys.exit(
            f"manifest not found: {path}\n"
            f"Run `dbt parse` first (offline, no warehouse needed)."
        )
    with path.open(encoding="utf-8") as fh:
        return json.load(fh)


def _ordered_columns(node: dict) -> list[tuple[str, str]]:
    """[(name, data_type), ...] preserving the manifest's declaration order."""
    return [
        (name, col.get("data_type") or "<no data_type>")
        for name, col in node.get("columns", {}).items()
    ]


def index_manifest(manifest: dict):
    """Return (connectors_by_tag, model_index).

    connectors_by_tag: tag -> [model_name, ...] for models carrying a
        `silver:*` tag (the per-connector staging models).
    model_index: model_name -> {"columns": [(name, type), ...], "enforced": bool}
        for every model; used to resolve the silver class model per tag.
    """
    connectors_by_tag: dict[str, list[str]] = defaultdict(list)
    model_index: dict[str, dict] = {}
    for node in manifest.get("nodes", {}).values():
        if node.get("resource_type") != "model":
            continue
        name = node["name"]
        model_index[name] = {
            "columns": _ordered_columns(node),
            "enforced": bool(node.get("contract", {}).get("enforced", False)),
        }
        for tag in node.get("tags", []):
            if tag.startswith(SILVER_TAG_PREFIX):
                connectors_by_tag[tag].append(name)
    return connectors_by_tag, model_index


def check_connector(
    reference: list[tuple[str, str]], connector_name: str, connector: dict
) -> list[str]:
    """Compare one connector against the silver reference (name + order + type)."""
    problems: list[str] = []

    if not connector["enforced"]:
        problems.append(
            f"{connector_name}: contract not enforced — its declared columns are "
            f"unverified against its SQL (add `contract: enforced: true`)"
        )
    cols = connector["columns"]
    if not cols:
        problems.append(f"{connector_name}: no typed columns declared in schema.yml")
        return problems

    ref_names = [c[0] for c in reference]
    col_names = [c[0] for c in cols]
    ref_set, col_set = set(ref_names), set(col_names)

    missing = [n for n in ref_names if n not in col_set]
    extra = [n for n in col_names if n not in ref_set]
    if missing:
        problems.append(f"{connector_name}: MISSING columns {missing}")
    if extra:
        problems.append(f"{connector_name}: UNEXPECTED columns {extra} (not in silver contract)")

    # Type mismatches on shared columns.
    ref_types = dict(reference)
    for name, dtype in cols:
        if name in ref_types and dtype != ref_types[name]:
            problems.append(
                f"{connector_name}.{name}: type {dtype} != silver {ref_types[name]}"
            )

    # Order mismatch — only meaningful once the sets match (otherwise the
    # missing/extra messages already explain the divergence). UNION ALL is
    # positional, so identical sets in a different order still cross-wire.
    if not missing and not extra and col_names != ref_names:
        problems.append(
            f"{connector_name}: column ORDER differs from silver contract\n"
            f"        silver:    {ref_names}\n"
            f"        connector: {col_names}"
        )

    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--manifest",
        type=Path,
        default=Path("src/ingestion/dbt/target/manifest.json"),
        help="path to dbt manifest.json (run `dbt parse` to produce it)",
    )
    args = parser.parse_args()

    connectors_by_tag, model_index = index_manifest(load_manifest(args.manifest))
    if not connectors_by_tag:
        print("No `silver:*`-tagged models found — nothing to check.")
        return 0

    failed = False
    checked = skipped = 0
    for tag in sorted(connectors_by_tag):
        silver_model = tag[len(SILVER_TAG_PREFIX):]  # e.g. "class_git_commits"
        anchor = model_index.get(silver_model)
        connector_names = sorted(connectors_by_tag[tag])
        label = f"{tag}  ({len(connector_names)} connector(s): {', '.join(connector_names)})"

        # Opt-in: only enforce classes whose silver model has an enforced contract.
        if anchor is None or not anchor["enforced"]:
            skipped += 1
            reason = "no silver model found" if anchor is None else "silver contract not enforced"
            print(f"· {tag}  [skipped — {reason}]")
            continue

        checked += 1
        problems: list[str] = []
        for name in connector_names:
            problems.extend(check_connector(anchor["columns"], name, model_index[name]))

        if problems:
            failed = True
            print(f"\n✗ {label}  [vs silver contract `{silver_model}`]")
            for p in problems:
                print(f"    - {p}")
        else:
            print(f"✓ {label}  [vs silver contract `{silver_model}`]")

    print(f"\nChecked {checked} enforced class(es); skipped {skipped} without an enforced contract.")
    if failed:
        print(
            "Contract drift detected. Every connector feeding an enforced silver "
            "class must emit exactly the columns the silver contract declares — "
            "same names, order and types.\nFix: align the `columns:` blocks in "
            "each connector's schema.yml to the silver class contract.",
            file=sys.stderr,
        )
        return 1

    print("All enforced silver classes are connector-consistent.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
