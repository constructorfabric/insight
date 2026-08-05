#!/usr/bin/env python3
"""
Check field-level parity between every staging contributor and its silver union target.

WHY
  A silver `class_*` model is a UNION ALL of every staging model tagged
  `silver:<target>` (see the `union_by_tag` macro). ClickHouse matches UNION
  branches BY POSITION and takes the column names from the first branch, so a
  contributor that renames or reorders a column does NOT fail the build — it
  silently writes values into the wrong silver column. Types are not checked
  either: ClickHouse widens to the branch supertype, so one contributor
  declaring `Int64` where another declares `Nullable(Int64)` quietly changes the
  published silver type depending on which connectors happen to be enabled.

  Neither drift is visible in the dbt DAG or in the connectors-ddl snapshot
  (which carries a single staging table by design). It is visible in
  `system.columns` of a warehouse where every model has been materialised —
  exactly what `bootstrap-db.sh` produces.

WHAT IT CHECKS
  1. coverage  — every non-ephemeral model in the manifest has a relation in the
                 warehouse. Without this a connector whose `discover` failed
                 would silently shrink the comparison and the audit would pass
                 for the wrong reason.
  2. columns   — contributor and target expose the same column names.
  3. order     — identical column positions (the UNION is positional).
  4. types     — `system.columns.type`, byte for byte.

  Everything above is a FAILURE (exit 1) except one benign shape, reported as a
  WARNing: the target published `Nullable(T)` where this contributor declares a
  plain `T`. ClickHouse widens like that when another branch is nullable, every
  value from this branch still fits, and readers already handle NULLs from the
  other branches. See classify_type_divergence for why the mirror image —
  contributor `Nullable(T)`, target `T` — is a failure instead.

  There is no baseline file: a failure is a failure on the run it appears in.

  A per-column breakdown of which contributors disagree with each other is
  printed after the findings — that is the root cause behind most type findings
  (the target type is merely the supertype the branches forced).

LIMITATIONS
  * An ephemeral contributor creates no relation of its own. When it is a plain
    pass-through over a single `source()`/`ref()` — the shape of
    `jira__task_field_history`, whose physical table is owned by the `jira-enrich`
    Rust binary and by the `create_task_field_history_staging` macro rather than
    by the dbt DAG — the audit follows that dependency and checks the underlying
    relation instead. An ephemeral model that transforms its input publishes
    columns no relation holds; it is listed as UNCHECKED and does not fail.
  * The audit only sees the connectors that were seeded into this warehouse.
    Check 1 guards the model-level hole, not a connector missing from
    `connectors-config.yaml` altogether.

USAGE
  export CLICKHOUSE_HOST=... CLICKHOUSE_PORT=... CLICKHOUSE_PROTOCOL=http
  export CLICKHOUSE_USER=... CLICKHOUSE_PASSWORD=...
  ./check-field-parity.py [--manifest PATH]

EXIT: 0 clean (warnings only is still clean), 1 failures, 2 usage/connection error.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from collections import defaultdict
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_MANIFEST = SCRIPT_DIR.parent.parent / "dbt" / "target" / "manifest.json"

# One row of `system.columns`, per relation, in positional order.
Column = tuple[int, str, str]  # (position, name, type)
Relation = tuple[str, str]  # (schema, identifier)


CH_VARS = ("CLICKHOUSE_HOST", "CLICKHOUSE_PORT", "CLICKHOUSE_PROTOCOL", "CLICKHOUSE_USER", "CLICKHOUSE_PASSWORD")


def load_env_file(path: Path) -> bool:
    """Apply the `.env` next to the scripts, exactly as bootstrap-db.sh sources it.

    Values in the file win over the inherited environment (`set -a; source .env`
    semantics) — otherwise a stale export from a previous shell silently beats the
    file the rest of the pipeline runs on. bash does the sourcing so expansions
    like `CLICKHOUSE_HOST=$(ipconfig getifaddr en0)` behave the same way here.
    """
    if not path.is_file():
        return False
    script = 'set -a; . "$1"; ' + "".join(f'printf "%s\\0" "${{{var}-}}"; ' for var in CH_VARS)
    result = subprocess.run(["bash", "-c", script, "_", str(path)], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        sys.exit(f"could not read {path}: {result.stderr.strip()}")
    for var, value in zip(CH_VARS, result.stdout.split("\0"), strict=False):
        if value:
            os.environ[var] = value
    return True


def env(name: str) -> str:
    """Required environment variable — no silent default (see dump-ddl.sh)."""
    value = os.environ.get(name)
    if not value:
        sys.exit(f"{name} must be set (in the .env next to this script, or exported)")
    return value


def query(sql: str) -> str:
    protocol = env("CLICKHOUSE_PROTOCOL")
    # Fail fast on a typo'd .env; also pins the urllib scheme (no file:// etc).
    if protocol not in ("http", "https"):
        sys.exit(f"CLICKHOUSE_PROTOCOL must be http or https, got {protocol!r}")
    url = f"{protocol}://{env('CLICKHOUSE_HOST')}:{env('CLICKHOUSE_PORT')}/"
    request = urllib.request.Request(
        url,
        data=sql.encode(),
        headers={"X-ClickHouse-User": env("CLICKHOUSE_USER"), "X-ClickHouse-Key": env("CLICKHOUSE_PASSWORD")},
    )
    try:
        with urllib.request.urlopen(request) as response:
            return response.read().decode()
    except urllib.error.HTTPError as exc:
        sys.exit(f"ClickHouse rejected the query: {exc.read().decode().strip()}")
    except OSError as exc:
        sys.exit(f"ClickHouse at {url} is unreachable: {exc}")


def rows(sql: str) -> list[dict]:
    payload = query(f"{sql} FORMAT JSONEachRow").strip()
    return [json.loads(line) for line in payload.splitlines() if line]


def relation_of(node: dict) -> Relation:
    return node["schema"], node.get("alias") or node["name"]


# An ephemeral pass-through: nothing but `SELECT * FROM <one source()/ref()>`, so
# the upstream relation's columns ARE the columns the model contributes.
PASSTHROUGH = re.compile(r"^SELECT\s+\*\s+FROM\s+\{\{\s*(?:source|ref)\((?P<args>[^()]*)\)\s*\}\}$", re.I)


def passthrough_relation(node: dict, manifest: dict) -> Relation | None:
    """Relation an ephemeral pass-through contributes, or None if it transforms its input."""
    body = re.sub(r"\{\{\s*config\(.*?\)\s*\}\}", " ", node["raw_code"], flags=re.S)
    body = re.sub(r"--[^\n]*", " ", body)
    match = PASSTHROUGH.match(" ".join(body.split()).rstrip(";"))
    if not match:
        return None
    # Last quoted argument of source('<source>', '<table>') / ref('<model>') names the
    # relation. Resolving it against `depends_on` rather than the whole raw_code matters:
    # `-- depends_on: {{ ref(...) }}` build-order hints put extra entries in there.
    quoted = re.findall(r"'([^']+)'|\"([^\"]+)\"", match.group("args"))
    if not quoted:
        return None
    wanted = quoted[-1][0] or quoted[-1][1]
    upstream = [
        entry
        for unique_id in node["depends_on"]["nodes"]
        if (entry := manifest["sources"].get(unique_id) or manifest["nodes"].get(unique_id)) and entry["name"] == wanted
    ]
    if len(upstream) != 1:
        return None
    return upstream[0]["schema"], upstream[0].get("identifier") or upstream[0].get("alias") or upstream[0]["name"]


def classify_type_divergence(source: str, target: str) -> tuple[str, bool]:
    """(label, is_failure) for a contributor type that differs from the target's.

    Only one shape is benign: the target widened a non-nullable branch to
    `Nullable(T)`. That is what ClickHouse does when ANOTHER contributor declares
    the column nullable — every value from this branch still fits the published
    type, and readers already have to handle NULLs from the other branches.

    The mirror image is not benign. A contributor declaring `Nullable(T)` against
    a target that publishes plain `T` cannot happen while the target is the
    supertype of its branches, so it means the silver table is stale (built
    before this contributor, kept by on_schema_change=ignore) and is about to
    truncate NULLs into zeroes.
    """
    if target == f"Nullable({source})":
        return "nullable-widening", False
    if source == f"Nullable({target})":
        return "nullable-narrowing", True
    return "representation", True


def load_manifest(path: Path) -> dict:
    if not path.exists():
        sys.exit(f"manifest not found: {path}\nRun dbt (bootstrap-db.sh does) or pass --manifest")
    return json.loads(path.read_text())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST, help=f"default: {DEFAULT_MANIFEST}")
    parser.add_argument(
        "--no-env-file",
        action="store_true",
        help="ignore the .env next to this script and use the inherited CLICKHOUSE_* variables "
        "(for pointing the audit at another cluster)",
    )
    args = parser.parse_args()

    if not args.no_env_file and load_env_file(SCRIPT_DIR / ".env"):
        print(f"using CLICKHOUSE_* from {SCRIPT_DIR / '.env'}")  # noqa: T201

    manifest = load_manifest(args.manifest)
    models = [node for node in manifest["nodes"].values() if node["resource_type"] == "model"]
    by_name = {node["name"]: node for node in models}

    def contributed_relation(node: dict) -> Relation | None:
        """Where a contributor's columns physically live, or None if nothing can be inspected."""
        if node["config"]["materialized"] != "ephemeral":
            return relation_of(node)
        return passthrough_relation(node, manifest)

    # Union groups: tag `silver:<target model name>` on each contributing model.
    groups: dict[str, list[dict]] = defaultdict(list)
    for node in models:
        for tag in node["config"].get("tags", []):
            if tag.startswith("silver:"):
                groups[tag.split(":", 1)[1]].append(node)

    schemas = sorted({node["schema"] for node in models})
    schema_list = ", ".join(f"'{schema}'" for schema in schemas)
    structure: dict[Relation, list[Column]] = defaultdict(list)
    for row in rows(
        "SELECT database, table, position, name, type FROM system.columns "
        f"WHERE database IN ({schema_list}) ORDER BY database, table, position"
    ):
        structure[(row["database"], row["table"])].append((row["position"], row["name"], row["type"]))

    findings: list[str] = []
    warnings: list[str] = []
    unchecked: list[str] = []

    # --- check 1: coverage -------------------------------------------------
    for node in sorted(models, key=lambda n: (n["schema"], n["name"])):
        if node["config"]["materialized"] == "ephemeral":
            continue
        schema, identifier = relation_of(node)
        if (schema, identifier) not in structure:
            findings.append(f"coverage  {schema}.{identifier}: model in the manifest has no relation in the warehouse")

    # --- checks 2-4: contributor vs its union target ------------------------
    for target in sorted(groups):
        target_node = by_name.get(target)
        if target_node is None:
            findings.append(f"coverage  {target}: tagged as a union target but absent from the manifest")
            continue
        target_relation = relation_of(target_node)
        target_columns = structure.get(target_relation)
        target_label = f"{target_relation[0]}.{target_relation[1]}"
        if not target_columns:
            # Already reported by check 1; nothing to compare against.
            continue

        target_types = {name: type_ for _, name, type_ in target_columns}
        target_order = [name for _, name, _ in target_columns]

        contributors = [node for node in groups[target] if relation_of(node) != target_relation]
        for node in sorted(contributors, key=lambda n: n["name"]):
            relation = contributed_relation(node)
            if relation is None:
                unchecked.append(
                    f"{node['schema']}.{node['name']} -> {target_label}: ephemeral model that transforms its "
                    "input — it publishes columns no relation holds"
                )
                continue
            label = f"{relation[0]}.{relation[1]}"
            if node["config"]["materialized"] == "ephemeral":
                label += " (via ephemeral pass-through)"
            source_columns = structure.get(relation)
            if not source_columns:
                # Ephemeral models are exempt from check 1 (they have no relation of
                # their own), so a missing pass-through target must be reported here.
                if node["config"]["materialized"] == "ephemeral":
                    findings.append(
                        f"coverage  {relation[0]}.{relation[1]}: read by the ephemeral contributor "
                        f"{node['name']} but absent from the warehouse"
                    )
                continue  # otherwise already reported by check 1

            source_types = {name: type_ for _, name, type_ in source_columns}
            source_order = [name for _, name, _ in source_columns]

            for name in target_order:
                if name not in source_types:
                    findings.append(f"columns   {label} -> {target_label}: missing `{name} {target_types[name]}`")
            for name in source_order:
                if name not in target_types:
                    findings.append(f"columns   {label} -> {target_label}: extra `{name} {source_types[name]}`")
            for name in source_order:
                if name in target_types and source_types[name] != target_types[name]:
                    kind, is_failure = classify_type_divergence(source_types[name], target_types[name])
                    message = (
                        f"types     {label} -> {target_label}: `{name}` is {source_types[name]}, "
                        f"target publishes {target_types[name]} [{kind}]"
                    )
                    (findings if is_failure else warnings).append(message)
            common_source = [name for name in source_order if name in target_types]
            common_target = [name for name in target_order if name in source_types]
            if common_source != common_target:
                findings.append(
                    f"order     {label} -> {target_label}: positional UNION mismatch\n"
                    f"                staging {common_source}\n"
                    f"                target  {common_target}"
                )

    for finding in findings:
        print(f"FAIL  {finding}")  # noqa: T201  — this IS the script's output
    for warning in warnings:
        print(f"WARN  {warning}")  # noqa: T201
    for note in unchecked:
        print(f"UNCHECKED  {note}")  # noqa: T201

    # --- diagnostics: which contributors disagree with each other ----------
    disagreements: list[str] = []
    for target in sorted(groups):
        target_node = by_name.get(target)
        target_relation = relation_of(target_node) if target_node else ("silver", target)
        contributors = [
            relation
            for node in groups[target]
            if relation_of(node) != target_relation
            and (relation := contributed_relation(node)) is not None
            and structure.get(relation)
        ]
        if len(contributors) < 2:
            continue
        variants: dict[str, dict[str, list[str]]] = defaultdict(lambda: defaultdict(list))
        for relation in contributors:
            for _, name, type_ in structure[relation]:
                variants[name][type_].append(relation[1])
        lines = [
            f"    {name}: " + " | ".join(f"{type_} [{', '.join(tables)}]" for type_, tables in sorted(kinds.items()))
            for name, kinds in variants.items()
            if len(kinds) > 1
        ]
        if lines:
            header = f"  {target_relation[0]}.{target_relation[1]} ({len(contributors)} contributors)"
            disagreements.append("\n".join([header, *lines]))
    if disagreements:
        print("\ncontributors disagreeing with each other (root cause of the type findings):")  # noqa: T201
        print("\n".join(disagreements))  # noqa: T201

    by_category: dict[str, int] = defaultdict(int)
    for finding in findings:
        by_category[finding.split(" ", 1)[0]] += 1
    for kind in ("nullable-narrowing", "representation"):
        by_category[kind] = sum(1 for finding in findings if f"[{kind}]" in finding)
    breakdown = ", ".join(f"{count} {category}" for category, count in sorted(by_category.items()) if count)
    print(  # noqa: T201
        f"\n{len(findings)} failure(s) ({breakdown or 'none'}), {len(warnings)} warning(s), "
        f"{len(unchecked)} unchecked, {len(groups)} union target(s), "
        f"{len(structure)} relation(s) in the warehouse"
    )
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
