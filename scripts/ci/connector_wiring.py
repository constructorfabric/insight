#!/usr/bin/env python3
"""Connector wiring guard — catch half-landed connectors on the PR, not after merge.

Motivating incident (#2048): the active-directory connector merged with
`version: "1.0"` (two components). `bump-descriptors` runs only on the push to
main, so the PR went green; post-merge the job aborted on the non-semver value,
never patched `images.cdk.image`, and — because the connector was also missing
from `connectors-config.yaml` — its dbt models failed with UNKNOWN_DATABASE and
took `silver.class_people` plus `insight.metric_entity_cohorts_current` down
with them via the hard `depends_on` in class_people.sql.

Every check here is satisfiable by the PR that introduces a connector, so this
gate can be required without creating a chicken-and-egg problem.

Checks
  1. semver     — any descriptor with an `images:` block MUST carry strict
                  semver. Those are exactly the descriptors `bump-descriptors`
                  feeds to bump-descriptor-version.sh, which hard-fails on
                  anything else. Descriptors WITHOUT an images block are only
                  warned about: ADR-0015 §"Legacy non-semver values"
                  deliberately tolerates in-place legacy strings
                  (`2026.05.04`) and classifies them as `migration`.
  2. config     — EVERY connector MUST have a `connectors-config.yaml` entry,
                  or bootstrap-db never creates its bronze database and its
                  silver models drop out of any regenerated DDL snapshot.
  3. depends_on — that same connector MUST be declared in class_people.sql's
                  `depends_on` list, per the convention stated in that file.
  4. cast-types — contributors MUST NOT explicitly cast the same class_people
                  column to different types; `union_by_tag` UNION ALLs the
                  branches and ClickHouse raises Code 386 NO_COMMON_TYPE.
  5. start-date — a nocode stream that pages a dated vendor collection MUST
                  enforce the connector's start date in the REQUEST or in a
                  stop condition. Declaring it as a cursor's `start_datetime`
                  filters NOTHING — the CDK only tracks state with it — so a
                  stream can look bounded and still fetch all history. The
                  incident this catches: twelve repository listings in one
                  connector, three of which declared the bound, none of which
                  applied it; every sync cloned every repository in the
                  workspace regardless of the configured window.

                  A substream counts as bounded when a parent listing bounds
                  it, because fetching all of an in-window item's
                  sub-resources (an issue's worklogs, a pull request's
                  comments) is the intended grain. KNOWN GAP in that rule: a
                  child that pages a CONTAINER's whole dated collection —
                  every deployment of a repository, say — inherits only the
                  repository filter, not a date bound, and this check cannot
                  tell that shape from the legitimate one. Such a stream needs
                  its own bound and a reviewer to notice it is missing.

Empty `images.<key>.image` refs are reported as warnings, never errors: a
brand-new CDK connector legitimately ships with `image: ""` and is patched by
the first main build. Check 1 is what keeps that patch from silently failing.

Usage: python3 scripts/ci/connector_wiring.py [--connectors-root D] [--warnings-as-errors]
Exit:  0 clean (warnings allowed), 1 on any error.
"""

# ruff: noqa: T201  — stdout/stderr IS this script's CI report (cf. changed.py).

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

import yaml

# Strict semver per semver.org §2, identical to classify_bump.py and
# bump-descriptor-version.sh: numeric identifiers MUST NOT carry leading
# zeroes, which is what keeps `2026.05.04` distinguishable from a real triplet.
SEMVER_RE = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")

CLASS_PEOPLE_TAG = "silver:class_people"

# `CAST(NULL AS Nullable(String)) AS manager_person_id` — only explicit casts are
# comparable. Contributors that project a real source column (bamboohr's
# `supervisorEId`) carry no inferable type here and are skipped by check 4.
CAST_RE = re.compile(r"CAST\s*\(\s*NULL\s+AS\s+(?P<type>[A-Za-z0-9_()]+)\s*\)\s+AS\s+(?P<col>[a-z_]+)", re.IGNORECASE)


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def _load_yaml(path: Path):
    with path.open() as fh:
        return yaml.safe_load(fh) or {}


def _class_people_models(connector_dir: Path) -> list[Path]:
    """Staging models this connector contributes to the class_people union."""
    dbt_dir = connector_dir / "dbt"
    if not dbt_dir.is_dir():
        return []
    return [p for p in sorted(dbt_dir.glob("*.sql")) if CLASS_PEOPLE_TAG in p.read_text()]


def _resolve_ref(pointer: str, spec: dict):
    """Follow an Airbyte `$ref` pointer (`#/definitions/...`) inside one manifest."""
    node = spec
    for part in pointer.lstrip("#/").split("/"):
        if isinstance(node, dict):
            if part not in node:
                return None
            node = node[part]
        elif isinstance(node, list) and part.isdigit() and int(part) < len(node):
            # `#/streams/13` — a pointer at a positional stream, which is how a
            # substream names a parent that is declared at the top level.
            node = node[int(part)]
        else:
            return None
    return node


def _requester_text(stream: dict, spec: dict) -> str:
    """The stream's requester as text, with `$ref` pointers expanded one level.

    A bound can be written as a query param, a JQL clause or a GraphQL
    variable, so the check reads the requester whole rather than guessing which
    field carries it.
    """
    requester = (stream.get("retriever") or {}).get("requester") or {}
    chunks = [yaml.safe_dump(requester, default_flow_style=False)]
    for ref in re.findall(r"[$]ref:\s*['\"]?(#[^'\"\s]+)", chunks[0]):
        target = _resolve_ref(ref, spec)
        if target is not None:
            chunks.append(yaml.safe_dump(target, default_flow_style=False))
    return " ".join(chunks)


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--connectors-root", default="src/ingestion/connectors")
    ap.add_argument(
        "--warnings-as-errors",
        action="store_true",
        help="promote warnings (legacy non-semver, empty image refs) to errors",
    )
    args = ap.parse_args(argv[1:])

    root = _repo_root()
    connectors_root = root / args.connectors_root
    config_path = root / "src/ingestion/scripts/bootstrap-db/connectors-config.yaml"
    class_people_path = root / "src/ingestion/silver/_shared/class_people.sql"

    for required in (connectors_root, config_path, class_people_path):
        if not required.exists():
            print(f"ERROR: expected path is missing: {required}", file=sys.stderr)
            return 1

    errors: list[str] = []
    warnings: list[str] = []

    config = _load_yaml(config_path).get("connectors") or {}
    # Index by descriptor-relative path, which is what the config's `path` holds.
    configured_paths = {str(entry.get("path")) for entry in config.values() if isinstance(entry, dict)}
    class_people_sql = class_people_path.read_text()

    # col -> {type -> [connector slug]}, for check 4.
    cast_types: dict[str, dict[str, list[str]]] = {}

    for descriptor_path in sorted(connectors_root.glob("*/*/descriptor.yaml")):
        connector_dir = descriptor_path.parent
        rel = connector_dir.relative_to(connectors_root).as_posix()
        descriptor = _load_yaml(descriptor_path)
        version = str(descriptor.get("version", ""))
        images = descriptor.get("images") or {}

        # ── 1. semver ────────────────────────────────────────────────────────
        if not SEMVER_RE.match(version):
            if images:
                errors.append(
                    f"{rel}: version {version!r} is not strict semver "
                    "MAJOR.MINOR.PATCH (ADR-0015), and this descriptor has an "
                    "`images:` block — bump-descriptors WILL abort post-merge "
                    "and leave images.*.image unpatched (see #2048)."
                )
            else:
                warnings.append(
                    f"{rel}: legacy non-semver version {version!r}. Tolerated "
                    "today (no `images:` block, so bump-descriptors never sees "
                    'it), but per ADR-0015 §"Legacy non-semver values" the next '
                    "version edit MUST move it to strict semver."
                )

        for key, entry in images.items():
            if not (entry or {}).get("image"):
                warnings.append(
                    f"{rel}: images.{key}.image is empty — expected for a "
                    "brand-new connector (the first main build patches it); "
                    "stale otherwise."
                )

        # ── 2. bootstrap registration — EVERY connector, not just class_people
        # contributors. A missing entry means bootstrap-db never creates the
        # connector's bronze database, its dbt models fail Code: 81
        # UNKNOWN_DATABASE, and whatever silver class they feed (class_people,
        # class_task_*, class_collab_*, …) drops out of the snapshot.
        models = _class_people_models(connector_dir)
        if rel not in configured_paths:
            consequence = (
                "takes the shared silver.class_people union down with it"
                if models
                else "drops its silver models out of any regenerated snapshot"
            )
            errors.append(
                f"{rel}: no entry in scripts/bootstrap-db/connectors-config.yaml "
                "— bootstrap-db will not create its bronze database, so its dbt "
                f"models fail with UNKNOWN_DATABASE and it {consequence} "
                "(see #2048). Add the entry with "
                f"`./generate-connectors-config.sh '{rel}'`."
            )

        # ── 3-4. class_people contributors only ─────────────────────────────
        if not models:
            continue

        for model in models:
            stem = model.stem
            # Match the parse-time declaration line itself, not a bare
            # ref('<stem>') that could appear in a comment or unrelated macro.
            depends_on_re = re.compile(
                rf"(?m)^\s*--\s*depends_on:\s*\{{\{{\s*ref\(\s*'{re.escape(stem)}'\s*\)\s*\}}\}}"
            )
            if not depends_on_re.search(class_people_sql):
                errors.append(
                    f"{rel}: staging model {stem} is tagged "
                    f"`{CLASS_PEOPLE_TAG}` but has no "
                    f"`-- depends_on: {{{{ ref('{stem}') }}}}` line in "
                    "silver/_shared/class_people.sql — union_by_tag would "
                    "compile it into the union without dbt ordering it first."
                )
            for m in CAST_RE.finditer(model.read_text()):
                col = m.group("col").lower()
                cast_types.setdefault(col, {}).setdefault(m.group("type"), []).append(rel)

    # ── 4. cast-type agreement across contributors ───────────────────────────
    for col, by_type in sorted(cast_types.items()):
        if len(by_type) > 1:
            detail = "; ".join(f"{t} in {', '.join(sorted(set(slugs)))}" for t, slugs in sorted(by_type.items()))
            errors.append(
                f"class_people column {col!r} is explicitly cast to conflicting "
                f"types across contributors ({detail}). union_by_tag UNION ALLs "
                "these branches — ClickHouse raises Code 386 NO_COMMON_TYPE "
                "(cpt-dataflow-constraint-staging-class-column-types-match)."
            )

    # ── 5. start-date enforcement on nocode streams ──────────────────────────
    # A retriever that pages a dated collection has to put the bound in the
    # request (`q`, or a `start_time_option` injecting one) or stop paging on it
    # (`is_data_feed`), or drop the old records (`is_client_side_incremental`).
    # `incremental_sync.start_datetime` alone is state, not a filter.
    for manifest in sorted(connectors_root.glob("*/*/connector.yaml")):
        rel = manifest.relative_to(connectors_root)
        spec = _load_yaml(manifest)
        if not isinstance(spec, dict):
            continue
        start_keys = {
            k for k in ((spec.get("spec") or {}).get("connection_specification") or {}).get("properties", {})
            if k.endswith("_start_date")
        }
        if not start_keys:
            continue

        def _enforces(stream: dict) -> bool:
            inc = stream.get("incremental_sync") or {}
            request = _requester_text(stream, spec)
            return bool(
                any(key in request for key in start_keys)
                or "stream_interval.start_time" in request
                or "stream_slice.start_time" in request
                or "start_time_option" in inc
                or inc.get("is_data_feed")
                or inc.get("is_client_side_incremental")
            )

        def _parents(stream: dict) -> list:
            """Parent streams, whether declared inline or behind a `$ref`."""
            router = (stream.get("retriever") or {}).get("partition_router") or {}
            routers = router if isinstance(router, list) else [router]
            out = []
            for r in routers:
                if not isinstance(r, dict):
                    continue
                for cfg in r.get("parent_stream_configs") or []:
                    parent = (cfg or {}).get("stream")
                    if isinstance(parent, dict) and "$ref" in parent:
                        parent = _resolve_ref(parent["$ref"], spec)
                    if isinstance(parent, dict):
                        out.append(parent)
            return out

        def _bounded(stream: dict, depth: int = 0) -> bool:
            """The stream bounds itself, or a parent listing bounds it for it.

            A substream inherits its parent's window: if the parent listing is
            filtered, the child only visits in-window items, and fetching all of
            a fetched item's sub-resources is the intended grain.
            """
            if depth > 6:
                return True
            return _enforces(stream) or any(_bounded(p, depth + 1) for p in _parents(stream))

        def _walk(node):
            if isinstance(node, dict):
                if node.get("type") == "DeclarativeStream":
                    inc = node.get("incremental_sync") or {}
                    if "start_datetime" in inc and not _bounded(node):
                        errors.append(
                            f"{rel}: stream {node.get('name', '?')!r} declares a "
                            "start_datetime that nothing enforces, and no parent "
                            "stream bounds it either — put the bound in the request "
                            "(`q`, `jql`, a GraphQL variable, a start_time_option) "
                            "or stop on it (is_data_feed / "
                            "is_client_side_incremental). A declared cursor start is "
                            "state, not a filter: as written this stream fetches all "
                            "history."
                        )
                for v in node.values():
                    _walk(v)
            elif isinstance(node, list):
                for v in node:
                    _walk(v)

        _walk(spec.get("streams") or [])

    if args.warnings_as_errors:
        errors, warnings = errors + warnings, []

    for w in warnings:
        print(f"WARN:  {w}")
    # Flush before switching streams so the CI log keeps warnings above errors
    # instead of interleaving buffered stdout with unbuffered stderr.
    sys.stdout.flush()
    for e in errors:
        print(f"ERROR: {e}", file=sys.stderr)
    sys.stderr.flush()

    if errors:
        print(f"\nconnector wiring guard FAILED: {len(errors)} error(s), {len(warnings)} warning(s)", file=sys.stderr)
        return 1
    print(f"\nconnector wiring guard OK ({len(warnings)} warning(s))")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
