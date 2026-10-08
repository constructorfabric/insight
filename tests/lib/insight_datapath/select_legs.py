"""Print the data-path legs a change needs, as GitHub step outputs.

    python -m insight_datapath.select_legs --manifest target/manifest.json --changed changed.txt
    python -m insight_datapath.select_legs ... --base-manifest base/manifest.json
    python -m insight_datapath.select_legs --full "no merge base"

Outputs: `matrix` (the leg matrix), `scope` (`full`, `partial`, or `none` when no
metric class runs), `classes` (the metric classes that run) and `known` (every
metric class). Why each changed path landed where it did goes to stderr.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from pathlib import Path

from insight_datapath.dbt_graph import DbtGraph, from_manifest
from insight_datapath.leg_selection import (
    Full,
    Leg,
    Partial,
    Verdict,
    combine,
    plan_legs,
    reach,
    verdict_for,
)
from insight_datapath.suite_scan import scan_suites

REPO_ROOT = Path(__file__).resolve().parents[3]


def _graph(manifest: Path) -> DbtGraph:
    return from_manifest(json.loads(manifest.read_text(encoding="utf-8")))


def _explain(paths: Sequence[str], verdicts: Sequence[Verdict]) -> None:
    for path, verdict in zip(paths, verdicts, strict=True):
        if isinstance(verdict, Full):
            sys.stderr.write(f"  {path}: full — {verdict.reason}\n")
        else:
            sys.stderr.write(f"  {path}: {', '.join(sorted(verdict.suites)) or 'meta only'}\n")


def outputs(legs: Sequence[Leg], verdict: Verdict, metric_classes: Sequence[str]) -> dict[str, str]:
    ran = sorted({name for leg in legs for name in leg.metric_classes})
    scope = "full" if isinstance(verdict, Full) else ("partial" if ran else "none")
    matrix = {
        "include": [
            {
                "shard": leg.shard,
                "trees": " ".join(f"--tree={tree}" for tree in leg.trees),
                "metrics": bool(leg.metric_classes),
            }
            for leg in legs
        ]
    }
    return {
        "matrix": json.dumps(matrix, separators=(",", ":")),
        "scope": scope,
        "classes": ",".join(ran),
        "known": ",".join(metric_classes),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--base-manifest", type=Path, help="the base revision's manifest")
    parser.add_argument("--changed", type=Path, help="one repository path per line")
    parser.add_argument("--full", metavar="REASON", help="run every leg without reading a diff")
    args = parser.parse_args(argv)

    suites = scan_suites(REPO_ROOT)
    if args.full:
        verdict: Verdict = Full(args.full)
        sys.stderr.write(f"full — {args.full}\n")
    elif suites.opaque:
        verdict = Full(f"{', '.join(suites.opaque)} seed a relation the planner cannot name")
        sys.stderr.write(f"full — {verdict.reason}\n")
    else:
        if args.manifest is None or args.changed is None:
            parser.error("--manifest and --changed are required unless --full is given")
        world = reach(_graph(args.manifest), suites)
        previous = reach(_graph(args.base_manifest), suites) if args.base_manifest else None
        paths = [line for line in args.changed.read_text(encoding="utf-8").splitlines() if line]
        verdicts = [verdict_for(path, world, previous) for path in paths]
        _explain(paths, verdicts)
        verdict = combine(verdicts) if verdicts else Partial(frozenset())

    legs = plan_legs(verdict, suites.metric_classes)
    for key, value in outputs(legs, verdict, suites.metric_classes).items():
        sys.stdout.write(f"{key}={value}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
