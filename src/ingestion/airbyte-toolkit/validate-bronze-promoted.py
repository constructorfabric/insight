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
import re
import sys
from collections import Counter
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
        except (json.JSONDecodeError, OSError) as e:
            # Malformed or unreadable catalog → fall through to source-code regex.
            # Surfacing the error keeps surprises out of CI logs without
            # aborting the validator (the regex fallback may still succeed).
            print(
                f"warning: could not parse {cat.relative_to(INGESTION_DIR.parent)}: {e}; "
                "falling back to source-code stream discovery",
                file=sys.stderr,
            )
        else:
            names = []
            for entry in data.get("streams", []):
                stream = entry.get("stream") if isinstance(entry, dict) else None
                if isinstance(stream, dict) and "name" in stream:
                    names.append(stream["name"])
            if names:
                return names, "configured_catalog.json"

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

_PROMOTE_CALL_RE = re.compile(
    r"""promote_bronze_to_rmt\s*\((.*?)\)""",
    re.DOTALL,
)
# captures the entire argument blob of one `promote_bronze_to_rmt(...)` call —
# used by BP-8 to verify order_by appears on EVERY call site, not just somewhere
# in the file. Non-greedy: stops at the first `)`, which is fine for the
# established `order_by='unique_key'` convention (no nested parens in args).
# IMPORTANT: run on comment-stripped text — see _strip_dbt_comments — otherwise
# the regex matches the macro name when it's mentioned in a {# ... #} doc block.

_DBT_BLOCK_COMMENT_RE = re.compile(r"\{#.*?#\}", re.DOTALL)
_SQL_LINE_COMMENT_RE = re.compile(r"--[^\n]*")

_CONFIG_BLOCK_RE = re.compile(r"\{\{\s*config\((.*?)\)\s*\}\}", re.DOTALL)
# Captures the argument blob of `{{ config(materialized='view', schema='staging', tags=[...]) }}`

_MATERIALIZED_RE = re.compile(r"materialized\s*=\s*['\"]([^'\"]+)['\"]")
_SCHEMA_KW_RE = re.compile(r"schema\s*=\s*['\"]([^'\"]+)['\"]")
_TAGS_RE = re.compile(r"tags\s*=\s*\[([^\]]*)\]", re.DOTALL)
# Whitespace-tolerant matchers for the three keyword arguments validated by
# BP-2/3/4. Operate on the args blob captured by _CONFIG_BLOCK_RE.


def _strip_dbt_comments(text: str) -> str:
    """Remove Jinja {# ... #} blocks and SQL `-- ...` line comments.

    Used to keep BP-5/6/7/8 regexes from matching `promote_bronze_to_rmt(...)`
    occurrences that exist only as documentation/examples inside comments
    (e.g. jira's docstring lists "Append a promote_bronze_to_rmt(...) call below").
    """
    text = _DBT_BLOCK_COMMENT_RE.sub("", text)
    text = _SQL_LINE_COMMENT_RE.sub("", text)
    return text

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
        if not res.streams:
            res.issues.append(Issue(
                "BP-PARSE", "WARN",
                "no streams discovered in connector.yaml — only existence of bronze_promoted will be checked",
                "expected: streams[].name or streams[].$ref pointing at definitions.streams.<name>"))
    elif res.kind == "cdk":
        # streams_from_cdk also returns a label of where it found the streams
        # (catalog vs source-code regex); we don't surface it today, drop it.
        res.streams, _ = streams_from_cdk(connector_dir, name)
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

    # Strip dbt block comments and SQL line comments before pattern matching:
    # the {# ... #} doc blocks at the top of bronze_promoted.sql often mention
    # `promote_bronze_to_rmt(...)` and example `{{ config(...) }}` snippets
    # that would otherwise satisfy the rules without effective code.
    bp_text = _strip_dbt_comments(bp_path.read_text())

    # ----- BP-2/3/4: config sanity -----
    # Locate the {{ config(...) }} block in the comment-stripped text and parse
    # its argument string. Each invariant is checked *inside* the block so that:
    #   - whitespace variants (`materialized = 'view'`) are accepted;
    #   - the connector-name check (BP-4) is scoped to `tags=[...]` rather than
    #     accidentally satisfied by an unrelated mention elsewhere in the file;
    #   - a commented-out config block fails BP-2 (the strip removes it).
    config_match = _CONFIG_BLOCK_RE.search(bp_text)
    if not config_match:
        res.issues.append(Issue(
            "BP-2", "FAIL",
            f"{bp_filename} has no {{{{ config(...) }}}} block",
            "see existing examples e.g. m365__bronze_promoted.sql"))
    else:
        config_args = config_match.group(1)

        # BP-2: materialized='view'
        mat = _MATERIALIZED_RE.search(config_args)
        if not mat or mat.group(1) != "view":
            actual = mat.group(1) if mat else "<absent>"
            res.issues.append(Issue(
                "BP-2", "FAIL",
                f"{bp_filename} must declare materialized='view' (found: {actual!r})",
                "see existing examples e.g. m365__bronze_promoted.sql"))

        # BP-3: schema='staging'
        sch = _SCHEMA_KW_RE.search(config_args)
        if not sch or sch.group(1) != "staging":
            actual = sch.group(1) if sch else "<absent>"
            res.issues.append(Issue(
                "BP-3", "FAIL",
                f"{bp_filename} must use schema='staging' (found: {actual!r})"))

        # BP-4: connector name appears inside tags=[...]
        tags = _TAGS_RE.search(config_args)
        tags_inner = tags.group(1) if tags else ""
        if f"'{name}'" not in tags_inner and f'"{name}"' not in tags_inner:
            res.issues.append(Issue(
                "BP-4", "FAIL",
                f"{bp_filename} tags=[...] must include the connector name '{name}'",
                f"current tags=[{tags_inner.strip()}]" if tags else "no tags=[...] argument"))

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

    # ----- BP-8: order_by argument present on EVERY promote_bronze_to_rmt call -----
    # Walks each call site individually rather than checking the file as a whole —
    # a single missing order_by anywhere in the file is still a violation.
    for m in _PROMOTE_CALL_RE.finditer(bp_text):
        args = m.group(1)
        if "order_by" not in args:
            line_no = bp_text.count("\n", 0, m.start()) + 1
            # truncate long arg blob for the evidence line
            preview = args.strip().replace("\n", " ")[:80]
            res.issues.append(Issue(
                "BP-8", "FAIL",
                f"promote_bronze_to_rmt call at {bp_filename}:{line_no} is missing order_by",
                f"convention: order_by='unique_key' (the natural-key composite injected by AddFields). args=`{preview}`"))

    # ----- BP-9: every other staging model that reads bronze declares depends_on -----
    # The depends_on header MUST be the first non-blank line above the {{ config(...) }}
    # block — that's the dbt mechanism that guarantees the bootstrap view runs first.
    # A loose "bp_model_name appears anywhere" check would false-pass models that
    # mention the bootstrap only in a comment far from the dependency contract.
    other_sql = sorted(p for p in dbt_dir.glob("*.sql") if p.name != bp_filename)
    expected_header = f"-- depends_on: {{{{ ref('{bp_model_name}') }}}}"
    for sql in other_sql:
        text = sql.read_text()
        if not _SOURCE_BRONZE_RE.search(text):
            continue  # model doesn't read bronze; depends_on not required

        # Find the {{ config( ... block — that's the anchor
        lines = text.splitlines()
        config_idx = next(
            (i for i, ln in enumerate(lines) if "{{ config(" in ln),
            None,
        )

        # First non-blank line above the config block (or above EOF if no config)
        first_above: str | None = None
        upper_bound = config_idx if config_idx is not None else len(lines)
        for i in range(upper_bound - 1, -1, -1):
            ln = lines[i].strip()
            if ln:
                first_above = ln
                break

        # Accept either the canonical depends_on comment OR a real ref(...) call
        # to bronze_promoted somewhere among the first few non-blank lines (some
        # models put `-- Bronze → Silver step 1` first then depends_on second).
        # We allow the depends_on line in the first 5 non-blank lines for tolerance,
        # but flag anything looser.
        head_lines: list[str] = []
        for ln in lines[:upper_bound]:
            s = ln.strip()
            if s:
                head_lines.append(s)
            if len(head_lines) >= 5:
                break

        ok = False
        for hl in head_lines:
            dep_match = _DEPENDS_ON_RE.search(hl)
            if dep_match and dep_match.group(1) == bp_model_name:
                ok = True
                break

        if not ok:
            res.issues.append(Issue(
                "BP-9", "FAIL",
                f"{sql.name} reads bronze but lacks the depends_on header for {bp_model_name}",
                f"add `{expected_header}` as a comment in the first 5 lines, ideally directly above the `{{{{ config(... }}}}` block. "
                f"first non-blank line currently above config: {first_above!r}"))

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
    # Summary line. Counter avoids the literal `{"PASS": 0, ...}` dict that
    # Bandit's B105 password-heuristic flags as a hardcoded credential.
    status_counts = Counter(r.status for r in results)
    out.append(
        f"Summary: PASS={status_counts['PASS']}  "
        f"FAIL={status_counts['FAIL']}  "
        f"SKIP={status_counts['SKIP']}"
    )
    return "\n".join(out)


def render_json(results: list[Result]) -> str:
    status_counts = Counter(r.status for r in results)
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
                "pass": status_counts["PASS"],
                "fail": status_counts["FAIL"],
                "skip": status_counts["SKIP"],
            },
        },
        indent=2,
    )


def main(argv: list[str]) -> int:
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

    results: list[Result] = []
    had_invalid_target = False
    for t in targets:
        if "/" not in t or not (CONNECTORS_DIR / t).is_dir():
            had_invalid_target = True
            results.append(Result(connector=t, kind="unknown",
                                  skipped_reason=f"directory not found under {CONNECTORS_DIR}"))
            continue
        results.append(check_connector(t))

    if args.json:
        sys.stdout.write(render_json(results) + "\n")
    else:
        sys.stdout.write(render_text(results) + "\n")

    # Exit-code contract: 0=PASS, 1=usage/filesystem error, 2=at least one FAIL.
    # Invalid target paths are usage errors, not validation failures — even if
    # all *valid* targets PASS, return 1 so typos don't masquerade as success.
    if had_invalid_target:
        return 1
    fails = sum(1 for r in results if r.status == "FAIL")
    return 2 if fails else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
