#!/usr/bin/env python3
"""Insight coverage gate — processes Cobertura reports and reports coverage per
component. It runs no tests and does no change detection: collection happens in
the CI producer jobs, and the changed-component matrix is built by changed.py.
The component registry lives in components.py (shared SSOT). This script's one
job is to check already-collected reports against two gates:
  1. each measured component's overall line coverage >= OVERALL_MIN
  2. new-code (patch) line coverage >= NEW_CODE_MIN  (via diff-cover)

Usage: python3 scripts/ci/coverage.py gate [--no-patch] [--reports-dir D] [--summary F]
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from shutil import which

# Reports are PR-derived, so parse defensively against XML entity attacks.
# defusedxml is installed in the CI gate job; local runs fall back to stdlib.
try:
    from defusedxml.ElementTree import parse as _xml_parse
except ImportError:
    from xml.etree.ElementTree import parse as _xml_parse

from components import ROOT, COMPARE_BRANCH, COMPONENTS, component_for

COVERAGE_DIR = ROOT / "coverage"

# The two gate thresholds (percent).
OVERALL_MIN = 80    # each component's overall line coverage
NEW_CODE_MIN = 80   # new-code (patch) line coverage


# --------------------------------------------------------------------------- #
# Cobertura parsing. Class filenames are normalized to repo-relative POSIX
# paths; _normalize probes the filesystem (the gate job checks out the full
# tree) to disambiguate multiple <source> roots, with a string-only fallback.
# --------------------------------------------------------------------------- #
def _normalize(filename: str, sources: list[str], root: Path) -> str:
    """Resolve a Cobertura class filename to a repo-relative POSIX path.

    A report may list several <source> roots, so a source-relative filename is
    ambiguous. Prefer a candidate that exists on disk (the gate job checks out
    the full tree); fall back to the first that lands under the repo root.
    """
    fn = filename.replace("\\", "/")
    p = Path(fn)
    candidates = [p] if p.is_absolute() else \
        [Path(s) / fn for s in sources] + [root / fn]

    under_root: list[tuple[str, bool]] = []
    for c in candidates:
        ac = Path(os.path.normpath(str(c if c.is_absolute() else root / c)))
        try:
            rel = ac.relative_to(root).as_posix()
        except ValueError:
            continue  # not under the repo root — skip this candidate
        under_root.append((rel, ac.exists()))

    for rel, exists in under_root:
        if exists:
            return rel
    return under_root[0][0] if under_root else fn


def parse_cobertura(path: Path, root: Path) -> dict[str, dict[int, int]]:
    """Return {repo_relative_path: {line_no: max_hits}} for one Cobertura file."""
    cov = _xml_parse(path).getroot()
    sources = [s.text.strip() for s in cov.findall("./sources/source") if s.text]
    files: dict[str, dict[int, int]] = {}
    for cls in cov.iterfind(".//class"):
        rel = _normalize(cls.get("filename", ""), sources, root)
        bucket = files.setdefault(rel, {})
        for line in cls.iterfind("./lines/line"):
            try:
                no = int(line.get("number"))
                hits = int(line.get("hits"))
            except (TypeError, ValueError):
                continue
            bucket[no] = max(bucket.get(no, 0), hits)  # dedup overlapping classes
    return files


def load_all_reports(reports_dir: Path, root: Path) -> dict[str, dict[int, int]]:
    """Merge every Cobertura *.xml in reports_dir. Empty dict if none (e.g. no
    component changed, so no producer ran)."""
    merged: dict[str, dict[int, int]] = {}
    report_files = sorted(glob.glob(str(reports_dir / "*.xml")))
    for rf in report_files:
        for rel, lines in parse_cobertura(Path(rf), root).items():
            dst = merged.setdefault(rel, {})
            for no, hits in lines.items():
                dst[no] = max(dst.get(no, 0), hits)
    return merged


# --------------------------------------------------------------------------- #
# Measurement (bucket covered files into components via the shared component_for)
# --------------------------------------------------------------------------- #
def measure(files: dict[str, dict[int, int]], components: list[dict]) -> tuple[dict, int]:
    """Return ({component_name: (covered, total, pct)}, unbucketed_file_count)."""
    agg = {c["name"]: [0, 0] for c in components}
    unbucketed = 0
    for rel, lines in files.items():
        name = component_for(rel, components)
        if name is None:
            unbucketed += 1
            continue
        agg[name][0] += sum(1 for h in lines.values() if h > 0)
        agg[name][1] += len(lines)
    result = {name: (cov, tot, (cov / tot * 100.0) if tot else 0.0)
              for name, (cov, tot) in agg.items()}
    return result, unbucketed


# --------------------------------------------------------------------------- #
# Reporting
# --------------------------------------------------------------------------- #
def print_table(measured: dict) -> bool:
    """Print the per-component table against OVERALL_MIN; return True if all pass."""
    name_w = max([len("Component")] + [len(n) for n in measured])
    header = f"{'Component':<{name_w}}  {'Lines':>13}  {'Coverage':>9}  {'Min':>5}  Result"
    print(header)
    print("-" * len(header))
    all_pass = True
    for name in sorted(measured):
        cov, tot, pct = measured[name]
        ok = pct >= OVERALL_MIN
        all_pass &= ok
        lines_cell = f"{cov}/{tot}"
        print(f"{name:<{name_w}}  {lines_cell:>13}  {pct:>8.1f}%  {OVERALL_MIN:>4}%  "
              f"{'PASS' if ok else 'FAIL'}"
              + ("" if ok else f"  (< min by {OVERALL_MIN - pct:.1f})"))
    print("-" * len(header))
    return all_pass


def _icon(ok: bool) -> str:
    return "✅" if ok else "🔴"


# (lang key, section heading) — controls the order of report sections.
LANG_SECTIONS = [("rust", "Rust"), ("python", "Python"), ("dotnet", ".NET")]


def markdown_report(measured: dict, components: list[dict], comp_pass: bool,
                    patch_pass: bool, patch_per_comp: dict, patch_output: str,
                    no_patch: bool, unbucketed: int, missing: list[str] = ()) -> str:
    """Render the gate result as a GitHub-flavored markdown report, one table per language."""
    by_name = {c["name"]: c for c in components}
    overall = comp_pass and (no_patch or patch_pass) and not missing
    patch_state = "skipped" if no_patch else _icon(patch_pass)

    out = [
        "## Coverage gate",
        "",
        f"**Overall: {_icon(overall)} {'PASS' if overall else 'FAIL'}** — "
        f"per-component (≥ {OVERALL_MIN}%) {_icon(comp_pass)}, "
        f"new-code (≥ {NEW_CODE_MIN}%) {patch_state}",
    ]
    if missing:
        out += ["", f"> 🔴 **Changed component(s) produced no coverage report** "
                f"(build/test failed, or no tests where some are required): "
                f"{', '.join(missing)}"]

    def table(names: list[str]) -> list[str]:
        rows = ["| Component | Lines | Coverage | Min | Result |",
                "| --- | ---: | ---: | ---: | :---: |"]
        for name in names:
            cov, tot, pct = measured[name]
            rows.append(f"| {name} | {cov}/{tot} | {pct:.1f}% | {OVERALL_MIN}% | {_icon(pct >= OVERALL_MIN)} |")
        return rows

    seen_langs = {lang for lang, _ in LANG_SECTIONS}
    for lang, heading in LANG_SECTIONS:
        names = sorted(n for n in measured if by_name[n]["lang"] == lang)
        if names:
            out += ["", f"### {heading}", ""] + table(names)
    other = sorted(n for n in measured if by_name[n]["lang"] not in seen_langs)
    if other:
        out += ["", "### Other", ""] + table(other)

    if unbucketed:
        out += ["", f"_{unbucketed} covered file(s) matched no component (ignored)._"]

    if not no_patch:
        out += ["", f"### New code (≥ {NEW_CODE_MIN}%)", ""]
        if patch_per_comp:
            def patch_table(names: list[str]) -> list[str]:
                rows = ["| Component | New lines | Coverage | Min | Result |",
                        "| --- | ---: | ---: | ---: | :---: |"]
                for name in names:
                    cov, tot, pct = patch_per_comp[name]
                    rows.append(f"| {name} | {cov}/{tot} | {pct:.1f}% | {NEW_CODE_MIN}% | {_icon(pct >= NEW_CODE_MIN)} |")
                return rows

            lang_of = {c["name"]: c["lang"] for c in components}
            for lang, heading in LANG_SECTIONS:
                names = sorted(n for n in patch_per_comp if lang_of.get(n) == lang)
                if names:
                    out += ["", f"#### {heading}", ""] + patch_table(names)
            other = sorted(n for n in patch_per_comp if lang_of.get(n) not in seen_langs)
            if other:
                out += ["", "#### Other", ""] + patch_table(other)
        else:
            out.append("_No changed coverable lines in this diff._")
        out += [
            "",
            "<details><summary>diff-cover output (per file)</summary>",
            "",
            "```",
            patch_output or "(no output)",
            "```",
            "",
            "</details>",
        ]
    out.append("")
    return "\n".join(out)


# --------------------------------------------------------------------------- #
# New-code (patch) gate
# --------------------------------------------------------------------------- #
def bucket_patch(src_stats: dict, components: list[dict]) -> dict:
    """Bucket diff-cover's per-file stats into components.

    Returns {component: (covered, total, pct)} for components with changed
    coverable lines. Files under no component are skipped — same as `measure`,
    so files outside every component path (CI scripts, docs) never gate a PR.
    """
    agg: dict[str, list[int]] = {}
    for fpath, st in src_stats.items():
        covered = len(st.get("covered_lines", []))
        total = covered + len(st.get("violation_lines", []))
        if total == 0:
            continue
        name = component_for(fpath, components)
        if name is None:
            continue
        b = agg.setdefault(name, [0, 0])
        b[0] += covered
        b[1] += total
    return {name: (cov, tot, cov / tot * 100.0) for name, (cov, tot) in agg.items()}


def run_patch_gate(reports_dir: Path, components: list[dict]) -> tuple[bool, dict, str]:
    """Run diff-cover and bucket new-line coverage per component.

    Returns (passed, per_component, raw_output). `passed` is True when every
    component with changed coverable lines meets NEW_CODE_MIN.
    """
    if which("diff-cover") is None:
        sys.exit("diff-cover not found — `pip install diff-cover` (required for the new-code gate).")
    reports = sorted(glob.glob(str(reports_dir / "*.xml")))
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
        json_path = tf.name
    # --fail-under 0: diff-cover never fails here; we decide per-component below.
    cmd = ["diff-cover", *reports, "--compare-branch", COMPARE_BRANCH,
           "--json-report", json_path, "--fail-under", "0"]
    print(f"\nNew-code gate: {' '.join(cmd)}\n")
    proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    sys.stdout.write(proc.stdout)
    if proc.stderr:
        sys.stderr.write(proc.stderr)
    # With --fail-under 0, diff-cover exits 0 at any coverage level, so a non-zero
    # exit means a real tool/input failure — stop before trusting its JSON.
    if proc.returncode != 0:
        Path(json_path).unlink(missing_ok=True)
        sys.exit(f"diff-cover failed (exit {proc.returncode}) — see output above")
    try:
        data = json.loads(Path(json_path).read_text())
    except (OSError, json.JSONDecodeError):
        sys.exit("diff-cover produced no JSON report")
    finally:
        Path(json_path).unlink(missing_ok=True)
    per_component = bucket_patch(data.get("src_stats", {}), components)
    passed = all(pct >= NEW_CODE_MIN for _, _, pct in per_component.values())
    return passed, per_component, proc.stdout.strip()


def print_patch_table(per_component: dict, components: list[dict]) -> None:
    print(f"\n=== New-code coverage gate (>= {NEW_CODE_MIN}%) ===\n")
    if not per_component:
        print("No changed coverable lines in this diff.")
        return
    lang_of = {c["name"]: c["lang"] for c in components}
    seen_langs = {lang for lang, _ in LANG_SECTIONS}
    name_w = max([len("Component")] + [len(n) for n in per_component])
    header = f"{'Component':<{name_w}}  {'New lines':>11}  {'Coverage':>9}  {'Min':>5}  Result"
    print(header)
    print("-" * len(header))

    def rows(names: list[str]) -> None:
        for name in names:
            cov, tot, pct = per_component[name]
            ok = pct >= NEW_CODE_MIN
            print(f"{name:<{name_w}}  {f'{cov}/{tot}':>11}  {pct:>8.1f}%  {NEW_CODE_MIN:>4}%  "
                  f"{'PASS' if ok else 'FAIL'}")

    for lang, heading in LANG_SECTIONS:
        names = sorted(n for n in per_component if lang_of.get(n) == lang)
        if names:
            print(f"[{heading}]")
            rows(names)
    other = sorted(n for n in per_component if lang_of.get(n) not in seen_langs)
    if other:
        print("[Other]")
        rows(other)
    print("-" * len(header))


# --------------------------------------------------------------------------- #
# Gate
# --------------------------------------------------------------------------- #
def cmd_gate(args) -> int:
    reports_dir = Path(args.reports_dir) if args.reports_dir else COVERAGE_DIR
    files = load_all_reports(reports_dir, ROOT)
    measured_all, unbucketed = measure(files, COMPONENTS)
    # Only judge components that actually ran (produced a report → total > 0).
    # Unchanged components are skipped by CI, so they have no report — don't fail
    # them at 0/0; they were gated when they last changed. (CI separately fails
    # the gate job if a *changed* component's producer job errored, so a missing
    # report can only mean "not changed", never "build broke".)
    measured = {n: v for n, v in measured_all.items() if v[1] > 0}
    skipped = sorted(n for n, v in measured_all.items() if v[1] == 0)

    # Components CI says changed MUST have produced a report. If a changed
    # component emitted none (build/test failed, or a producer succeeded but
    # wrote nothing), it would otherwise be silently "skipped" → fail instead.
    required = [n.strip() for n in (getattr(args, "require", None) or "").split(",") if n.strip()]
    missing = sorted(n for n in required if measured_all.get(n, (0, 0, 0.0))[1] == 0)

    print(f"\n=== Per-component overall coverage gate (>= {OVERALL_MIN}%) ===\n")
    if measured:
        comp_pass = print_table(measured)
    else:
        comp_pass = True
        print("No component reports — nothing to gate.")
    if skipped:
        print(f"\nskipped (no diff / not run): {', '.join(skipped)}")
    if unbucketed:
        print(f"\nnote: {unbucketed} covered file(s) matched no component (ignored).")
    if missing:
        print(f"\n::error::changed component(s) produced no coverage report "
              f"(build/test failed?): {', '.join(missing)}")

    patch_pass, patch_per_comp, patch_output = True, {}, ""
    if not args.no_patch and files:
        patch_pass, patch_per_comp, patch_output = run_patch_gate(reports_dir, COMPONENTS)
        print_patch_table(patch_per_comp, COMPONENTS)

    ok = comp_pass and patch_pass and not missing
    print(f"\nOverall: {'PASS' if ok else 'FAIL'} "
          f"(components: {'PASS' if comp_pass else 'FAIL'}, "
          f"new-code: {'PASS' if patch_pass else 'FAIL' if not args.no_patch else 'SKIPPED'}"
          f"{f', missing-reports: {len(missing)}' if missing else ''})")

    if getattr(args, "summary", None):
        md = markdown_report(measured, COMPONENTS, comp_pass, patch_pass,
                             patch_per_comp, patch_output, args.no_patch, unbucketed, missing)
        with open(args.summary, "a", encoding="utf-8") as f:
            f.write(md + "\n")
        print(f"\nMarkdown report written to {args.summary}")
    return 0 if ok else 2


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #
def main() -> int:
    p = argparse.ArgumentParser(
        description="Insight coverage gate — checks Cobertura reports; runs no tests.")
    sub = p.add_subparsers(dest="cmd", required=True)

    pg = sub.add_parser("gate", help="evaluate coverage gates against Cobertura reports")
    pg.add_argument("--no-patch", action="store_true", help="skip the new-code (diff-cover) gate")
    pg.add_argument("--reports-dir", help="dir of Cobertura *.xml (default: ./coverage)")
    pg.add_argument("--require", help="comma-separated component names that MUST have a "
                    "report (the changed set); fail if any produced none")
    pg.add_argument("--summary", help="append a markdown report to this file (e.g. $GITHUB_STEP_SUMMARY)")
    pg.set_defaults(func=cmd_gate)

    args = p.parse_args()
    return args.func(args) or 0


if __name__ == "__main__":
    sys.exit(main())
