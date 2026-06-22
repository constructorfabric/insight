#!/usr/bin/env python3
"""coverage_gaps.py — turn a cobertura.xml into a ranked "what to test next" worklist.

Reads a Cobertura coverage report (e.g. produced by `cargo llvm-cov report
--cobertura` or .NET coverlet) and prints, sorted by the most uncovered lines:

  - per FILE: covered/total lines, line-rate, uncovered-line count
  - per METHOD (when the report carries <methods>): the specific functions with
    the most uncovered lines — the concrete test targets

So test-writing is directed at the biggest holes instead of guessed.

Usage:
  coverage_gaps.py [COBERTURA_XML]              # default: cobertura.xml
  coverage_gaps.py cov.xml --by method          # rank methods (default: file)
  coverage_gaps.py cov.xml --min-uncovered 10   # hide tiny gaps
  coverage_gaps.py cov.xml --top 25             # limit rows
  coverage_gaps.py cov.xml --json               # machine-readable (ratchet input)
  coverage_gaps.py cov.xml --target 85          # print lines-needed to hit a %

Exit: 0 always (reporting tool). Pair with the ratchet gate for enforcement.
"""
from __future__ import annotations

import argparse
import json
import sys
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field


@dataclass
class Unit:
    name: str
    file: str
    covered: int = 0
    total: int = 0

    @property
    def uncovered(self) -> int:
        return self.total - self.covered

    @property
    def rate(self) -> float:
        return (self.covered / self.total) if self.total else 1.0


@dataclass
class FileCov:
    file: str
    covered: int = 0
    total: int = 0
    methods: list[Unit] = field(default_factory=list)

    @property
    def uncovered(self) -> int:
        return self.total - self.covered

    @property
    def rate(self) -> float:
        return (self.covered / self.total) if self.total else 1.0


def parse(path: str) -> list[FileCov]:
    root = ET.parse(path).getroot()
    files: dict[str, FileCov] = {}
    # cobertura: packages > package > classes > class(filename) > {methods, lines}
    for cls in root.iter("class"):
        fname = cls.get("filename") or cls.get("name") or "<unknown>"
        fc = files.setdefault(fname, FileCov(file=fname))
        # file-level lines
        lines_el = cls.find("lines")
        if lines_el is not None:
            for ln in lines_el.findall("line"):
                fc.total += 1
                if int(ln.get("hits", "0")) > 0:
                    fc.covered += 1
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
                if u.total:
                    fc.methods.append(u)
    return list(files.values())


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("xml", nargs="?", default="cobertura.xml")
    ap.add_argument("--by", choices=["file", "method"], default="file")
    ap.add_argument("--min-uncovered", type=int, default=1)
    ap.add_argument("--top", type=int, default=40)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--target", type=float, default=None,
                    help="report covered-lines needed to reach this overall %%")
    args = ap.parse_args()

    try:
        files = parse(args.xml)
    except (ET.ParseError, FileNotFoundError) as e:
        print(f"FATAL: cannot read {args.xml}: {e}", file=sys.stderr)
        return 0

    tot = sum(f.total for f in files)
    cov = sum(f.covered for f in files)
    overall = (cov / tot * 100) if tot else 100.0

    if args.by == "method":
        rows = [m for f in files for m in f.methods if m.uncovered >= args.min_uncovered]
    else:
        rows = [f for f in files if f.uncovered >= args.min_uncovered]
    rows.sort(key=lambda r: r.uncovered, reverse=True)
    rows = rows[: args.top]

    if args.json:
        out = {
            "overall_pct": round(overall, 2), "covered": cov, "total": tot,
            "rows": [
                {"file": r.file, "name": getattr(r, "name", None),
                 "covered": r.covered, "total": r.total,
                 "uncovered": r.uncovered, "rate": round(r.rate * 100, 1)}
                for r in rows],
        }
        print(json.dumps(out, indent=2))
        return 0

    print(f"== coverage gaps ({args.by}) — overall {overall:.1f}% ({cov}/{tot}) ==\n")
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
