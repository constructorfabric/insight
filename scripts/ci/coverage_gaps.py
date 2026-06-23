#!/usr/bin/env python3
"""coverage_gaps.py — turn a cobertura.xml into a ranked "what to test next" worklist.

Reads a Cobertura coverage report (e.g. produced by `cargo llvm-cov report
--cobertura` or .NET coverlet) and prints, sorted by the biggest gap:

  - per FILE: covered/total lines, line-rate, uncovered-line count
  - per METHOD (when the report carries <methods>): the specific functions with
    the most uncovered lines — the concrete test targets
  - per BRANCH (--by branch): the units with the most uncovered *branches*, i.e.
    code whose lines run but whose conditionals (if/match arms) are half-tested

So test-writing is directed at the biggest holes instead of guessed.

Usage:
  coverage_gaps.py [COBERTURA_XML]              # default: cobertura.xml
  coverage_gaps.py cov.xml --by method          # rank methods (default: file)
  coverage_gaps.py cov.xml --by branch          # rank by uncovered branches
  coverage_gaps.py cov.xml --min-uncovered 10   # hide tiny gaps
  coverage_gaps.py cov.xml --top 25             # limit rows
  coverage_gaps.py cov.xml --json               # machine-readable (ratchet input)
  coverage_gaps.py cov.xml --target 85          # print lines-needed to hit a %

Exit codes:
  0  report produced (gaps may exist — this is a reporter, enforcement is the
     ratchet gate's job, so "found gaps" is NOT a failure here)
  2  could not produce a report: the input is missing, malformed, or carries no
     coverage data at all. A missing/empty report is NOT "100% covered" — failing
     loudly here stops a broken test run from masquerading as a clean gate.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field

# Cobertura reports are CI-generated (trusted), but parse with defusedxml when it
# is installed for defense-in-depth against XXE / entity-expansion — falling back
# to the stdlib so this stays a zero-dependency standalone tool.
try:
    import defusedxml.ElementTree as ET  # type: ignore[import-untyped]
except ModuleNotFoundError:
    import xml.etree.ElementTree as ET

# cobertura branch lines carry e.g. condition-coverage="50% (1/2)" — pull the (a/b).
_CONDITION = re.compile(r"\((\d+)/(\d+)\)")


def _line_branches(ln: ET.Element) -> tuple[int, int]:
    """(covered_branches, total_branches) for one <line>, (0, 0) if not a branch."""
    if ln.get("branch") != "true":
        return (0, 0)
    m = _CONDITION.search(ln.get("condition-coverage", ""))
    if not m:
        return (0, 0)
    return (int(m.group(1)), int(m.group(2)))


@dataclass
class Unit:
    name: str
    file: str
    covered: int = 0
    total: int = 0
    br_covered: int = 0
    br_total: int = 0

    @property
    def uncovered(self) -> int:
        return self.total - self.covered

    @property
    def br_uncovered(self) -> int:
        return self.br_total - self.br_covered

    @property
    def rate(self) -> float:
        return (self.covered / self.total) if self.total else 1.0

    @property
    def br_rate(self) -> float:
        return (self.br_covered / self.br_total) if self.br_total else 1.0


@dataclass
class FileCov:
    file: str
    covered: int = 0
    total: int = 0
    br_covered: int = 0
    br_total: int = 0
    methods: list[Unit] = field(default_factory=list)

    @property
    def uncovered(self) -> int:
        return self.total - self.covered

    @property
    def br_uncovered(self) -> int:
        return self.br_total - self.br_covered

    @property
    def rate(self) -> float:
        return (self.covered / self.total) if self.total else 1.0

    @property
    def br_rate(self) -> float:
        return (self.br_covered / self.br_total) if self.br_total else 1.0


def parse(path: str) -> list[FileCov]:
    root = ET.parse(path).getroot()
    files: dict[str, FileCov] = {}
    # cobertura: packages > package > classes > class(filename) > {methods, lines}
    for cls in root.iter("class"):
        fname = cls.get("filename") or cls.get("name") or "<unknown>"
        fc = files.setdefault(fname, FileCov(file=fname))
        # file-level lines (+ branch conditions carried on those lines)
        lines_el = cls.find("lines")
        if lines_el is not None:
            for ln in lines_el.findall("line"):
                fc.total += 1
                if int(ln.get("hits", "0")) > 0:
                    fc.covered += 1
                bc, bt = _line_branches(ln)
                fc.br_covered += bc
                fc.br_total += bt
        # method-level (cargo-llvm-cov and coverlet both emit <methods>)
        methods_el = cls.find("methods")
        if methods_el is not None:
            for m in methods_el.findall("method"):
                u = Unit(name=m.get("name", "?"), file=fname)
                mlines = m.find("lines")
                if mlines is not None:
                    for ln in mlines.findall("line"):
                        u.total += 1
                        if int(ln.get("hits", "0")) > 0:
                            u.covered += 1
                        bc, bt = _line_branches(ln)
                        u.br_covered += bc
                        u.br_total += bt
                if u.total:
                    fc.methods.append(u)
    return list(files.values())


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("xml", nargs="?", default="cobertura.xml")
    ap.add_argument("--by", choices=["file", "method", "branch"], default="file")
    ap.add_argument("--min-uncovered", type=int, default=1)
    ap.add_argument("--top", type=int, default=40)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--target", type=float, default=None,
                    help="report covered-lines needed to reach this overall %%")
    args = ap.parse_args()

    # Reject nonsensical bounds rather than silently mis-rank: a negative
    # --min-uncovered passes every row, --top <= 0 empties or reverse-slices the
    # list, and --target outside 0..100 asks for "lines to exceed 100%".
    if args.min_uncovered < 0:
        print("FATAL: --min-uncovered must be >= 0", file=sys.stderr)
        return 2
    if args.top <= 0:
        print("FATAL: --top must be > 0", file=sys.stderr)
        return 2
    if args.target is not None and not (0 <= args.target <= 100):
        print("FATAL: --target must be between 0 and 100", file=sys.stderr)
        return 2

    # A reporter that can't read its input must NOT look like "0 gaps". Fail loud
    # (exit 2) so a broken test run can't pass as a clean coverage report.
    try:
        files = parse(args.xml)
    except (ET.ParseError, FileNotFoundError, OSError) as e:
        print(f"FATAL: cannot read {args.xml}: {e}", file=sys.stderr)
        return 2

    tot = sum(f.total for f in files)
    cov = sum(f.covered for f in files)
    if tot == 0:
        print(f"FATAL: {args.xml} parsed but contains no coverage data "
              "(no <class>/<line> rows) — refusing to report 0/0 as 100%.",
              file=sys.stderr)
        return 2

    overall = cov / tot * 100
    br_tot = sum(f.br_total for f in files)
    br_cov = sum(f.br_covered for f in files)
    br_overall = (br_cov / br_tot * 100) if br_tot else 100.0

    # --by branch ranks by uncovered branches; file/method rank by uncovered lines.
    rank = "br_uncovered" if args.by == "branch" else "uncovered"
    if args.by == "method":
        candidates: list = [m for f in files for m in f.methods]
    elif args.by == "branch":
        # branch gaps live at method granularity when present, else file
        candidates = [m for f in files for m in f.methods if m.br_total] or list(files)
    else:
        candidates = list(files)
    rows = [r for r in candidates if getattr(r, rank) >= args.min_uncovered]
    rows.sort(key=lambda r: getattr(r, rank), reverse=True)
    rows = rows[: args.top]

    if args.json:
        out = {
            "overall_pct": round(overall, 2), "covered": cov, "total": tot,
            "branch_overall_pct": round(br_overall, 2),
            "branch_covered": br_cov, "branch_total": br_tot,
            "by": args.by,
            "rows": [
                {"file": r.file, "name": getattr(r, "name", None),
                 "covered": r.covered, "total": r.total,
                 "uncovered": r.uncovered, "rate": round(r.rate * 100, 1),
                 "br_covered": r.br_covered, "br_total": r.br_total,
                 "br_uncovered": r.br_uncovered, "br_rate": round(r.br_rate * 100, 1)}
                for r in rows],
        }
        print(json.dumps(out, indent=2))
        return 0

    hdr = f"== coverage gaps ({args.by}) — lines {overall:.1f}% ({cov}/{tot})"
    if br_tot:
        hdr += f", branches {br_overall:.1f}% ({br_cov}/{br_tot})"
    print(hdr + " ==\n")
    if args.by == "branch":
        print(f"{'BR-UNCOV':>8}  {'BR-RATE':>7}  {'BR-COV/TOT':>12}  TARGET")
        for r in rows:
            label = f"{r.file}::{r.name}" if getattr(r, "name", None) else r.file
            print(f"{r.br_uncovered:>8}  {r.br_rate*100:>6.0f}%  "
                  f"{r.br_covered:>4}/{r.br_total:<6}  {label}")
    else:
        print(f"{'UNCOV':>6}  {'RATE':>5}  {'COV/TOT':>10}  TARGET")
        for r in rows:
            label = f"{r.file}::{r.name}" if args.by == "method" else r.file
            print(f"{r.uncovered:>6}  {r.rate*100:>4.0f}%  {r.covered:>4}/{r.total:<5}  {label}")

    if args.target is not None:
        need = max(0, int(args.target / 100 * tot) - cov)
        print(f"\nto reach {args.target:.0f}%: +{need} covered lines "
              f"(now {overall:.1f}%, {cov}/{tot})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
