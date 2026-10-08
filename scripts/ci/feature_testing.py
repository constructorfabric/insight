#!/usr/bin/env python3
"""Derive a FEATURE's `## 7. Testing` section from test citations.

A test proves a requirement by citing it with the Studio scope-marker grammar:

    # @cpt-test:cpt-ir-fr-operator-bind:p1

This tool reads the Studio registry (`.cf-studio/config/artifacts.toml`), scans
every registered codebase entry for `@cpt-test` citations, reads the FEATURE's
section 1.2 requirement list, and renders two tables into section 7:

  7.1 Requirement verification — one row per FR/DoD: the tests citing it, their
      suite, whether any is end to end. No vector column: a functional
      requirement does not cover a quality vector.
  7.2 Quality metrics — one row per NFR: its vector (from the PRD's 6.1 table or
      `**Vector**` line), metric (the NFR heading), target (its `**Threshold**`),
      and sources (citing tests plus the DESIGN NFR-allocation verification
      cell).

`Collected today` and `Note` are the only authored cells; they are preserved
across regenerations by requirement ID.

Usage:
  feature_testing.py docs/.../FEATURE.md            print the block
  feature_testing.py docs/.../FEATURE.md --write    rewrite the block in place
  feature_testing.py docs/.../FEATURE.md --gate     exit 1 when a requirement is
                                                     neither cited nor noted
  feature_testing.py --check                        every registered FEATURE that
                                                     already carries a block must
                                                     be current; exit 1 otherwise

The engine's own query commands (`cfs where-used`, `list-ids --include-code`)
index no code citation while a system's artifacts are DOCS-ONLY, which is why
this tool scans the registry's codebase entries itself.
"""

from __future__ import annotations

import re
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

CITATION_RE = re.compile(r"@cpt-test:(?P<id>cpt-[a-z0-9][a-z0-9-]+):(?:p|ph-)\d+")
PY_TEST_RE = re.compile(r"^\s*(?:async\s+)?def\s+(test\w*)")
TS_TEST_RE = re.compile(r"\b(?:it|test)\(\s*['\"`](.+?)['\"`]")
RS_TEST_RE = re.compile(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)")
ID_DEF_RE = re.compile(r"\*\*ID\*\*:\s*`(cpt-[a-z0-9][a-z0-9-]+)`")
ID_RE = re.compile(r"`(cpt-[a-z0-9][a-z0-9-]+)`")
VECTORS = ("Efficiency", "Reliability", "Performance", "Security", "Versatility")
BEGIN = "<!-- feature-testing:begin -->"
END = "<!-- feature-testing:end -->"
E2E_SUITES = {"stand-api", "stand-ui", "identity-e2e", "metric-spec", "ingestion-e2e"}
SUITE_PREFIXES = (
    ("tests/stand/api", "stand-api"),
    ("tests/stand/ui", "stand-ui"),
    ("tests/datapath/identity", "identity-e2e"),
    ("tests/datapath", "metric-spec"),
    ("src/ingestion/tests", "ingestion-e2e"),
    ("src/backend/services/authenticator/tests", "auth-rig"),
    ("src/ingestion/connectors", "connector-tests"),
    ("src/ingestion/dbt/tests", "dbt-tests"),
    ("src/frontend/src", "fe-unit"),
)
SKIPPED_DIRS = {".venv", "node_modules", "__pycache__"}


@dataclass(frozen=True)
class Artifact:
    path: Path
    kind: str


@dataclass(frozen=True)
class CodebaseEntry:
    path: Path
    extensions: tuple[str, ...]


@dataclass
class Registry:
    artifacts: list[Artifact] = field(default_factory=list)
    codebase: list[CodebaseEntry] = field(default_factory=list)


@dataclass(frozen=True)
class Citation:
    path: Path
    line: int
    test_name: str


def load_registry(root: Path) -> Registry:
    data = tomllib.loads((root / ".cf-studio" / "config" / "artifacts.toml").read_text())
    reg = Registry()
    for system in data.get("systems", []):
        for art in system.get("artifacts", []):
            if isinstance(art, dict) and art.get("path"):
                reg.artifacts.append(Artifact(Path(art["path"]), str(art.get("kind", ""))))
        for cb in system.get("codebase", []):
            if isinstance(cb, dict) and cb.get("path") and cb.get("extensions"):
                reg.codebase.append(CodebaseEntry(Path(cb["path"]), tuple(cb["extensions"])))
    return reg


def _test_name(lines: list[str], start: int, suffix: str) -> str:
    pattern = {".py": PY_TEST_RE, ".rs": RS_TEST_RE}.get(suffix, TS_TEST_RE)
    for line in lines[start : start + 12]:
        if CITATION_RE.search(line) or not line.strip():
            continue
        m = pattern.search(line)
        if m:
            return m.group(1)
    return ""


def scan_citations(registry: Registry, root: Path) -> dict[str, list[Citation]]:
    found: dict[str, list[Citation]] = {}
    for entry in registry.codebase:
        base = root / entry.path
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*")):
            if not path.is_file() or path.suffix not in entry.extensions:
                continue
            if any(part in SKIPPED_DIRS for part in path.parts):
                continue
            lines = path.read_text(errors="replace").splitlines()
            rel = path.relative_to(root)
            for i, line in enumerate(lines):
                for m in CITATION_RE.finditer(line):
                    name = _test_name(lines, i + 1, path.suffix) or path.stem
                    found.setdefault(m.group("id"), []).append(Citation(rel, i + 1, name))
    return found


def suite_for(rel_path: Path) -> tuple[str, bool]:
    rel = rel_path.as_posix()
    for prefix, suite in SUITE_PREFIXES:
        if rel == prefix or rel.startswith(prefix + "/"):
            if suite == "fe-unit" and rel.endswith(".stories.tsx"):
                suite = "fe-component"
            return suite, suite in E2E_SUITES
    return "unit", False


@dataclass(frozen=True)
class Definition:
    id: str
    path: Path
    line: int
    heading: str
    threshold: str
    vector: str


def feature_requirements(text: str) -> list[str]:
    lines = text.splitlines()
    ids: list[str] = []
    start = next((n for n, line in enumerate(lines) if line.startswith("**Requirements**")), None)
    if start is None:
        return ids
    ids.extend(ID_RE.findall(lines[start]))
    for line in lines[start + 1 :]:
        if line.startswith(("**", "#")):
            break
        if line.strip().startswith("-"):
            ids.extend(ID_RE.findall(line))
    seen: set[str] = set()
    return [x for x in ids if not (x in seen or seen.add(x))]


def _vector_rows(lines: list[str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for line in lines:
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) >= 2 and cells[0] in VECTORS:
            for nfr in ID_RE.findall(line):
                out.setdefault(nfr, cells[0])
    return out


def load_definitions(registry: Registry, root: Path) -> dict[str, Definition]:
    defs: dict[str, Definition] = {}
    for art in registry.artifacts:
        if art.kind not in ("PRD", "FEATURE"):
            continue
        path = root / art.path
        if not path.is_file():
            continue
        lines = path.read_text(errors="replace").splitlines()
        by_row = _vector_rows(lines)
        heading = ""
        for i, line in enumerate(lines):
            if line.startswith("#"):
                heading = line.lstrip("#").strip()
            m = ID_DEF_RE.search(line)
            if not m:
                continue
            threshold, vector = "", ""
            for body in lines[i + 1 :]:
                if body.startswith("#"):
                    break
                if body.startswith("**Threshold**:"):
                    threshold = body.split(":", 1)[1].strip()
                if body.startswith("**Vector**"):
                    vector = body.split(":", 1)[1].strip()
            rid = m.group(1)
            defs.setdefault(rid, Definition(rid, art.path, i + 1, heading, threshold, by_row.get(rid, vector)))
    return defs


def verification_for(nfr_id: str, registry: Registry, root: Path) -> str:
    for art in registry.artifacts:
        if art.kind != "DESIGN" or not (root / art.path).is_file():
            continue
        header: list[str] = []
        for line in (root / art.path).read_text(errors="replace").splitlines():
            if not line.strip().startswith("|"):
                header = []
                continue
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if not header:
                header = cells
                continue
            if cells and f"`{nfr_id}`" in cells[0] and "Verification Approach" in header:
                return cells[-1]
    return ""
