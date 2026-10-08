#!/usr/bin/env python3
"""Derive a FEATURE's `## 7. Testing` section from test citations.

A test proves a requirement by citing it with the Studio scope-marker grammar:

    # @cpt-test:cpt-ir-fr-operator-bind:p1

This tool reads the Studio registry (`.cf-studio/config/artifacts.toml`), scans
every registered codebase entry for `@cpt-test` citations, reads the FEATURE's
section 1.2 requirement list, and renders two tables into section 7:

  7.1 Requirement verification — one row per FR/DoD: the tests citing it, their
      suite, whether any is end to end.
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
"""

# ruff: noqa: T201
from __future__ import annotations

import argparse
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

CITATION_RE = re.compile(r"@cpt-test:(?P<id>cpt-[a-z0-9][a-z0-9-]+):(?:p|ph-)\d+")
PY_TEST_RE = re.compile(r"^\s*(?:async\s+)?def\s+(test\w*)")
TS_TEST_RE = re.compile(r"\b(?:it|test)\(\s*['\"`](.+?)['\"`]")
RS_TEST_RE = re.compile(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)")
ID_DEF_RE = re.compile(r"\*\*ID\*\*:\s*`(cpt-[a-z0-9][a-z0-9-]+)`")
ID_RE = re.compile(r"`(cpt-[a-z0-9][a-z0-9-]+)`")
CELL_SPLIT_RE = re.compile(r"(?<!\\)\|")
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


def _cells(line: str) -> list[str]:
    return [c.strip().replace("\\|", "|") for c in CELL_SPLIT_RE.split(line.strip().strip("|"))]


def _vector_rows(lines: list[str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for line in lines:
        cells = _cells(line)
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
            body_lines = lines[i + 1 :]
            for j, body in enumerate(body_lines):
                if body.startswith("#"):
                    break
                if body.startswith("**Threshold**:"):
                    parts = [body.split(":", 1)[1].strip()]
                    for cont in body_lines[j + 1 :]:
                        if not cont.strip() or cont.startswith(("**", "#", "-", "|")):
                            break
                        parts.append(cont.strip())
                    threshold = " ".join(parts)
                if body.startswith("**Vector**"):
                    vector = body.split(":", 1)[1].strip()
            rid = m.group(1)
            defs.setdefault(rid, Definition(rid, art.path, i + 1, heading, threshold, by_row.get(rid, vector)))
    return defs


def verification_for(nfr_id: str, registry: Registry, root: Path) -> str:
    found: list[str] = []
    for art in registry.artifacts:
        if art.kind != "DESIGN" or not (root / art.path).is_file():
            continue
        header: list[str] = []
        for line in (root / art.path).read_text(errors="replace").splitlines():
            if not line.strip().startswith("|"):
                header = []
                continue
            cells = _cells(line)
            if not header:
                header = cells
                continue
            if cells and f"`{nfr_id}`" in cells[0] and "Verification Approach" in header:
                found.append(cells[-1])
    return "; ".join(found)


SUITE_ORDER = {suite: i for i, (_, suite) in enumerate(SUITE_PREFIXES)}
INTRO = (
    "Generated by `scripts/ci/feature_testing.py` from `@cpt-test` citations in the registered test trees. "
    "Edit only the `Collected today` and `Note` cells; rerun with `--write` after adding a citation."
)


def _cell(text: str) -> str:
    return text.replace("|", "\\|").replace("\n", " ").strip()


def _preserved(text: str) -> dict[str, tuple[str, str]]:
    out: dict[str, tuple[str, str]] = {}
    if BEGIN not in text or END not in text:
        return out
    block = text.split(BEGIN, 1)[1].split(END, 1)[0]
    for line in block.splitlines():
        if not line.startswith("| ") or line.startswith(("| Requirement", "| Vector")):
            continue
        cells = _cells(line)
        ids = ID_RE.findall(line)
        if not ids:
            continue
        if len(cells) == 5:
            out[ids[0]] = ("", cells[4])
        elif len(cells) == 7:
            out[ids[0]] = (cells[4], cells[6])
    return out


def _tests_cell(cites: list[Citation]) -> str:
    return ", ".join(f"{c.path.name}::{c.test_name}" for c in cites) if cites else "none"


def render(feature_path: Path, root: Path) -> tuple[str, list[str]]:
    registry = load_registry(root)
    text = (root / feature_path).read_text()
    reqs = feature_requirements(text)
    cites = scan_citations(registry, root)
    defs = load_definitions(registry, root)
    kept = _preserved(text)
    failures: list[str] = []
    functional = [r for r in reqs if "-nfr-" not in r]
    here = (root / feature_path).resolve()
    functional += [
        rid
        for rid, d in defs.items()
        if "-dod-" in rid and rid in cites and rid not in functional and (root / d.path).resolve() == here
    ]
    quality = [r for r in reqs if "-nfr-" in r]

    rows_a = ["| Requirement | Tests citing it | Suite | End to end | Note |", "|---|---|---|---|---|"]
    for rid in functional:
        note = kept.get(rid, ("", ""))[1]
        if rid not in defs:
            rows_a.append(f"| `{rid}` | unknown id |  | no | {_cell(note)} |")
            failures.append(f"{rid}: unknown id")
            continue
        found = cites.get(rid, [])
        suites = sorted({suite_for(c.path)[0] for c in found}, key=lambda s: SUITE_ORDER.get(s, 99))
        e2e = "yes" if any(suite_for(c.path)[1] for c in found) else "no"
        rows_a.append(f"| `{rid}` | {_cell(_tests_cell(found))} | {', '.join(suites)} | {e2e} | {_cell(note)} |")
        if not found and not note:
            failures.append(f"{rid}: no test and no note")

    rows_b = ["| Vector | NFR | Metric | Target | Collected today | Source | Note |", "|---|---|---|---|---|---|---|"]
    vector_order = {v: i for i, v in enumerate(VECTORS)}

    def quality_key(rid: str) -> tuple[int, int]:
        rank = vector_order.get(defs[rid].vector, 99) if rid in defs else 99
        return rank, quality.index(rid)

    for rid in sorted(quality, key=quality_key):
        collected, note = kept.get(rid, ("", ""))
        if rid not in defs:
            rows_b.append(f"| unassigned | `{rid}` |  |  | {_cell(collected)} | unknown id | {_cell(note)} |")
            failures.append(f"{rid}: unknown id")
            continue
        d = defs[rid]
        sources = [_tests_cell(cites[rid])] if cites.get(rid) else []
        approach = verification_for(rid, registry, root)
        if approach:
            sources.append(approach)
        rows_b.append(
            f"| {d.vector or 'unassigned'} | `{rid}` | {_cell(d.heading)} | {_cell(d.threshold)} "
            f"| {_cell(collected)} | {_cell('; '.join(sources) or 'none')} | {_cell(note)} |"
        )
        if not sources and not note:
            failures.append(f"{rid}: no source and no note")

    block = "\n".join(
        [
            BEGIN,
            INTRO,
            "",
            "### 7.1 Requirement verification",
            "",
            *rows_a,
            "",
            "### 7.2 Quality metrics",
            "",
            *rows_b,
            END,
        ]
    )
    return block, failures


def _splice(text: str, block: str) -> str:
    if BEGIN in text and END in text:
        head, rest = text.split(BEGIN, 1)
        _, tail = rest.split(END, 1)
        return head + block + tail
    heading = re.search(r"^## 7\. Testing[^\n]*\n", text, re.MULTILINE)
    if heading is None:
        return text.rstrip("\n") + "\n\n## 7. Testing\n\n" + block + "\n"
    after = text[heading.end() :]
    nxt = re.search(r"^## ", after, re.MULTILINE)
    cut = heading.end() + (nxt.start() if nxt else len(after))
    return text[:cut].rstrip("\n") + "\n\n" + block + "\n" + ("\n" + text[cut:] if nxt else "")


def apply(feature_path: Path, root: Path, write: bool) -> bool:
    path = root / feature_path
    old = path.read_text()
    block, _ = render(feature_path, root)
    new = _splice(old, block)
    if new == old:
        return False
    if write:
        path.write_text(new)
    return True


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("feature", nargs="?", help="FEATURE.md path relative to the repo root")
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--gate", action="store_true")
    args = ap.parse_args(argv)
    root = args.root.resolve()
    if args.check:
        stale = []
        for art in load_registry(root).artifacts:
            file = root / art.path
            if (
                art.kind == "FEATURE"
                and file.is_file()
                and BEGIN in file.read_text()
                and apply(art.path, root, write=False)
            ):
                stale.append(str(art.path))
        for s in stale:
            print(f"stale testing block: {s} (run feature_testing.py {s} --write)")
        return 1 if stale else 0
    if not args.feature:
        ap.error("a FEATURE path is required unless --check is given")
    feature = Path(args.feature)
    if args.write:
        changed = apply(feature, root, write=True)
        print(f"{'updated' if changed else 'unchanged'}: {feature}")
    block, failures = render(feature, root)
    if not args.write:
        print(block)
    for f in failures:
        print(f"gap: {f}", file=sys.stderr)
    return 1 if (args.gate and failures) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
