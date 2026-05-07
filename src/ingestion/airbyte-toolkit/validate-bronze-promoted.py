#!/usr/bin/env python3
"""
validate-bronze-promoted.py — enforce promote_bronze_to_rmt is wired correctly.

Airbyte writes bronze tables as plain MergeTree (see airbyte-toolkit/connect.sh),
so full-refresh streams accumulate duplicates across syncs. The macro
`promote_bronze_to_rmt` (see dbt/macros/promote_bronze_to_rmt.sql) migrates
each bronze table to ReplacingMergeTree on first run and is idempotent.

Every connector with a dbt/ directory MUST:
  1. Provide `<connector_snake>__bronze_promoted.sql` that calls
     `promote_bronze_to_rmt` for every Airbyte-produced bronze table.
  2. Have every other dbt staging model that reads bronze declare a
     `-- depends_on: {{ ref('<connector_snake>__bronze_promoted') }}` so
     dbt's DAG materialises the bootstrap view BEFORE downstream models run.

This script verifies both invariants for nocode (declarative manifest) and
CDK (Python) connectors.

Usage:
    validate-bronze-promoted.py <category>/<connector>
    validate-bronze-promoted.py --all
    validate-bronze-promoted.py --json [<targets>...]

Exit codes:
    0 — all checked connectors PASS
    2 — at least one FAIL
    1 — usage / filesystem error
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

try:
    import yaml
except ImportError:
    print("ERROR: PyYAML is required (pip install pyyaml)", file=sys.stderr)
    sys.exit(1)


SCRIPT_DIR = Path(__file__).resolve().parent
INGESTION_DIR = SCRIPT_DIR.parent
CONNECTORS_DIR = INGESTION_DIR / "connectors"


@dataclass
class Issue:
    rule: str           # rule id, e.g. BP-1
    severity: str       # FAIL | WARN | INFO
    message: str
    evidence: str = ""  # file:line or quoted excerpt


@dataclass
class Result:
    connector: str           # category/name
    kind: str                # nocode | cdk | unknown
    streams: list[str] = field(default_factory=list)
    issues: list[Issue] = field(default_factory=list)
    skipped_reason: str = ""

    @property
    def status(self) -> str:
        if self.skipped_reason:
            return "SKIP"
        if any(i.severity == "FAIL" for i in self.issues):
            return "FAIL"
        return "PASS"


# ----------------- helpers -----------------

def snake(name: str) -> str:
    return name.replace("-", "_")


def find_connectors() -> list[str]:
    """Return all <category>/<connector> paths under connectors/."""
    out = []
    for cat in sorted(CONNECTORS_DIR.iterdir()):
        if not cat.is_dir():
            continue
        for name in sorted(cat.iterdir()):
            if name.is_dir():
                out.append(f"{cat.name}/{name.name}")
    return out


def detect_kind(connector_dir: Path, name: str) -> str:
    if (connector_dir / "connector.yaml").is_file():
        return "nocode"
    if (connector_dir / f"source_{snake(name)}").is_dir() or (connector_dir / "Dockerfile").is_file():
        return "cdk"
    return "unknown"


# ----------------- stream discovery -----------------

def _resolve_ref(doc: dict, ref: str) -> dict | None:
    """Resolve a JSON pointer like `#/definitions/streams/users_details`."""
    if not ref.startswith("#/"):
        return None
    node: object = doc
    for part in ref[2:].split("/"):
        if not isinstance(node, dict) or part not in node:
            return None
        node = node[part]
    return node if isinstance(node, dict) else None


def streams_from_yaml(yaml_path: Path) -> list[str]:
    """Extract stream names from a declarative connector.yaml.

    Handles both inline `{name: ...}` entries and `{$ref: '#/definitions/...'}`
    references that resolve to a stream definition elsewhere in the document.
    """
    data = yaml.safe_load(yaml_path.read_text())
    streams = (data.get("streams") if isinstance(data, dict) else None) or []
    out: list[str] = []
    for s in streams:
        if not isinstance(s, dict):
            continue
        if "name" in s:
            out.append(s["name"])
            continue
        ref = s.get("$ref")
        if isinstance(ref, str):
            target = _resolve_ref(data, ref)
            if target and "name" in target:
                out.append(target["name"])
    return out


_CDK_NAME_RE = re.compile(r"""^\s+name\s*=\s*['"]([a-zA-Z0-9_-]+)['"]""", re.MULTILINE)


def streams_from_cdk(connector_dir: Path, name: str) -> tuple[list[str], str]:
    """Best-effort stream discovery for CDK connectors.

    Order of preference:
      1. configured_catalog.json (most authoritative)
      2. regex over source_<name>/streams/*.py
      3. regex over source_<name>/source.py

    Returns (streams, source_label).
    """
    cat = connector_dir / "configured_catalog.json"
    if cat.is_file():
        try:
            data = json.loads(cat.read_text())
            names = []
            for entry in data.get("streams", []):
                stream = entry.get("stream") if isinstance(entry, dict) else None
                if isinstance(stream, dict) and "name" in stream:
                    names.append(stream["name"])
            if names:
                return names, "configured_catalog.json"
        except Exception:
            pass

    src_dir = connector_dir / f"source_{snake(name)}"
    py_files: list[Path] = []
    streams_subdir = src_dir / "streams"
    if streams_subdir.is_dir():
        py_files = sorted(streams_subdir.glob("*.py"))
    if not py_files and src_dir.is_dir():
        py_files = sorted(src_dir.glob("*.py"))

    found: list[str] = []
    for p in py_files:
        text = p.read_text(errors="ignore")
        for m in _CDK_NAME_RE.finditer(text):
            n = m.group(1)
            # skip class-attribute placeholders like name = "" or non-stream classes
            if n and n not in found and not n.startswith("_"):
                found.append(n)
    if found:
        return found, f"source_{snake(name)}/streams/*.py (regex)"

    return [], "no streams discovered"


# ----------------- rule checks -----------------

_PROMOTE_RE = re.compile(
    r"""promote_bronze_to_rmt\s*\(\s*table\s*=\s*['"]([a-zA-Z0-9_]+)\.([a-zA-Z0-9_-]+)['"]"""
)
# captures `table='bronze_X.Y'` from `{% do promote_bronze_to_rmt(table='bronze_X.Y', ...) %}`

_DEPENDS_ON_RE = re.compile(
    r"""depends_on:\s*\{\{\s*ref\(\s*['"]([a-zA-Z0-9_]+__bronze_promoted)['"]\s*\)"""
)
# captures `<conn>__bronze_promoted` from `-- depends_on: {{ ref('<conn>__bronze_promoted') }}`

_SOURCE_BRONZE_RE = re.compile(
    r"""source\(\s*['"]bronze_[a-zA-Z0-9_]+['"]\s*,\s*['"][a-zA-Z0-9_-]+['"]\s*\)"""
)
# `{{ source('bronze_X', 'Y') }}` — anything reading bronze

_REF_BRONZE_PROMOTED_RE = re.compile(
    r"""ref\(\s*['"]([a-zA-Z0-9_]+__bronze_promoted)['"]\s*\)"""
)


def check_connector(rel: str) -> Result:
    name = rel.split("/", 1)[1]
    connector_dir = CONNECTORS_DIR / rel
    snake_name = snake(name)
    bronze_db = f"bronze_{snake_name}"
    bp_filename = f"{snake_name}__bronze_promoted.sql"
    bp_model_name = f"{snake_name}__bronze_promoted"
    dbt_dir = connector_dir / "dbt"

    res = Result(connector=rel, kind=detect_kind(connector_dir, name))

    # ----- discover expected streams -----
    if res.kind == "nocode":
        try:
            res.streams = streams_from_yaml(connector_dir / "connector.yaml")
        except Exception as e:
            res.issues.append(Issue("BP-PARSE", "FAIL",
                                    f"could not parse connector.yaml: {e}"))
            return res
        stream_source = "connector.yaml"
        if not res.streams:
            res.issues.append(Issue(
                "BP-PARSE", "WARN",
                "no streams discovered in connector.yaml — only existence of bronze_promoted will be checked",
                "expected: streams[].name or streams[].$ref pointing at definitions.streams.<name>"))
    elif res.kind == "cdk":
        res.streams, stream_source = streams_from_cdk(connector_dir, name)
        if not res.streams:
            res.issues.append(Issue("BP-PARSE", "WARN",
                                    "could not enumerate CDK streams; only existence of bronze_promoted will be checked",
                                    "(no configured_catalog.json and no `name = \"...\"` lines found)"))
    else:
        res.skipped_reason = "no connector.yaml and no source_<name>/ directory — not a deployable connector"
        return res

    # ----- skip if connector has no dbt/ at all -----
    if not dbt_dir.is_dir():
        # If a connector is functional (has streams) but no dbt — that's a gap, but the
        # bronze_promoted invariant only kicks in once a dbt/ directory exists.
        # Treat as INFO (not FAIL): bronze_promoted is needed only when downstream
        # dbt models read from bronze.
        res.skipped_reason = f"no dbt/ directory yet ({len(res.streams)} stream(s) declared)"
        return res

    # ----- BP-1: bronze_promoted.sql exists -----
    bp_path = dbt_dir / bp_filename
    if not bp_path.is_file():
        res.issues.append(Issue(
            "BP-1", "FAIL",
            f"missing {bp_filename} — required because dbt/ exists",
            f"expected at {bp_path.relative_to(INGESTION_DIR.parent)}"))
        return res  # downstream rules can't run without the file

    bp_text = bp_path.read_text()

    # ----- BP-2/3/4: config sanity -----
    if "materialized='view'" not in bp_text and 'materialized="view"' not in bp_text:
        res.issues.append(Issue("BP-2", "FAIL",
                                f"{bp_filename} must be materialized='view'",
                                "see existing examples e.g. m365__bronze_promoted.sql"))
    if "schema='staging'" not in bp_text and 'schema="staging"' not in bp_text:
        res.issues.append(Issue("BP-3", "FAIL",
                                f"{bp_filename} must use schema='staging'"))
    if name not in bp_text:
        res.issues.append(Issue("BP-4", "FAIL",
                                f"{bp_filename} must include connector tag '{name}'",
                                "expected tags=['<name>'] block"))

    # ----- BP-5: every stream is promoted -----
    promoted = {(db, tbl) for db, tbl in _PROMOTE_RE.findall(bp_text)}
    promoted_in_db = {tbl for db, tbl in promoted if db == bronze_db}

    if res.streams:
        missing = [s for s in res.streams if s not in promoted_in_db]
        for m in missing:
            res.issues.append(Issue(
                "BP-5", "FAIL",
                f"stream '{m}' has no promote_bronze_to_rmt call",
                f"expected `{{% do promote_bronze_to_rmt(table='{bronze_db}.{m}', order_by='unique_key') %}}` in {bp_filename}"))

        # BP-6: no spurious calls
        spurious = [tbl for tbl in promoted_in_db if tbl not in res.streams]
        for s in spurious:
            res.issues.append(Issue(
                "BP-6", "WARN",
                f"promote_bronze_to_rmt references '{s}' which is not a known stream",
                f"either remove it from {bp_filename} or add the stream to the manifest/source"))

        # BP-7: cross-database calls (caller pointed at bronze_other.something)
        for db, tbl in promoted:
            if db != bronze_db:
                res.issues.append(Issue(
                    "BP-7", "FAIL",
                    f"promote_bronze_to_rmt references '{db}.{tbl}' — wrong database",
                    f"connector's namespace is {bronze_db}, not {db}"))
    else:
        # CDK without enumerable streams — just require at least one promote call
        if not promoted:
            res.issues.append(Issue(
                "BP-5", "FAIL",
                f"{bp_filename} contains no promote_bronze_to_rmt calls"))

    # ----- BP-8: order_by present -----
    if "order_by" not in bp_text and promoted:
        res.issues.append(Issue("BP-8", "FAIL",
                                "promote_bronze_to_rmt calls must include order_by argument",
                                "convention: order_by='unique_key' (the natural-key composite injected by AddFields)"))

    # ----- BP-9: every other staging model that reads bronze declares depends_on -----
    other_sql = sorted(p for p in dbt_dir.glob("*.sql") if p.name != bp_filename)
    for sql in other_sql:
        text = sql.read_text()
        reads_bronze = bool(_SOURCE_BRONZE_RE.search(text))
        if not reads_bronze:
            continue
        declares = (
            bool(_DEPENDS_ON_RE.search(text) and bp_model_name in (_DEPENDS_ON_RE.search(text).group(1),))
            or bool(_REF_BRONZE_PROMOTED_RE.search(text) and bp_model_name in (_REF_BRONZE_PROMOTED_RE.search(text).group(1),))
        )
        # Stricter: any depends_on/ref pointing at this connector's bronze_promoted
        if not declares:
            # check more permissively — any header/ref to this connector's bp model
            if bp_model_name not in text:
                res.issues.append(Issue(
                    "BP-9", "FAIL",
                    f"{sql.name} reads bronze but doesn't depend on {bp_model_name}",
                    f"add `-- depends_on: {{{{ ref('{bp_model_name}') }}}}` as the first non-blank line above the config block"))

    return res


# ----------------- output -----------------

def render_text(results: list[Result]) -> str:
    out = []
    for r in results:
        head = f"=== {r.connector} ({r.kind}) {r.status}"
        if r.skipped_reason:
            head += f" — {r.skipped_reason}"
        out.append(head)
        if r.streams and not r.skipped_reason:
            out.append(f"  streams ({len(r.streams)}): {', '.join(r.streams)}")
        for it in r.issues:
            out.append(f"  [{it.severity}] {it.rule}: {it.message}")
            if it.evidence:
                out.append(f"           {it.evidence}")
        out.append("")
    # summary
    counts = {"PASS": 0, "FAIL": 0, "SKIP": 0}
    for r in results:
        counts[r.status] = counts.get(r.status, 0) + 1
    out.append(
        f"Summary: PASS={counts['PASS']}  FAIL={counts['FAIL']}  SKIP={counts['SKIP']}"
    )
    return "\n".join(out)


def render_json(results: list[Result]) -> str:
    return json.dumps(
        {
            "results": [
                {
                    "connector": r.connector,
                    "kind": r.kind,
                    "status": r.status,
                    "streams": r.streams,
                    "skipped_reason": r.skipped_reason,
                    "issues": [
                        {
                            "rule": i.rule,
                            "severity": i.severity,
                            "message": i.message,
                            "evidence": i.evidence,
                        }
                        for i in r.issues
                    ],
                }
                for r in results
            ],
            "summary": {
                "pass": sum(1 for r in results if r.status == "PASS"),
                "fail": sum(1 for r in results if r.status == "FAIL"),
                "skip": sum(1 for r in results if r.status == "SKIP"),
            },
        },
        indent=2,
    )


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0].strip())
    p.add_argument("targets", nargs="*",
                   help="<category>/<connector> paths; default: all under connectors/")
    p.add_argument("--all", action="store_true",
                   help="check every connector (same as omitting targets)")
    p.add_argument("--json", action="store_true",
                   help="emit JSON to stdout instead of human-readable text")
    args = p.parse_args(argv)

    targets = args.targets if (args.targets and not args.all) else find_connectors()
    if not targets:
        print("No connectors found", file=sys.stderr)
        return 1

    results = []
    for t in targets:
        if "/" not in t or not (CONNECTORS_DIR / t).is_dir():
            results.append(Result(connector=t, kind="unknown",
                                  skipped_reason=f"directory not found under {CONNECTORS_DIR}"))
            continue
        results.append(check_connector(t))

    if args.json:
        sys.stdout.write(render_json(results) + "\n")
    else:
        sys.stdout.write(render_text(results) + "\n")

    fails = sum(1 for r in results if r.status == "FAIL")
    return 2 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
