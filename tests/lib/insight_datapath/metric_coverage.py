from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from dataclasses import dataclass, field
from pathlib import Path

_E2E_ROOT = Path(__file__).resolve().parents[1]
LEDGER_FILE = _E2E_ROOT / ".artifacts" / "metric_assertions.json"

# The gate runs as a bare `python3 <path>`, so this module may import no sibling.


@dataclass(frozen=True)
class MetricDefinition:
    metric_key: str
    label: str
    computation: str
    dimensions: tuple[str, ...]
    peer_cohort_key: str | None

    @property
    def required_views(self) -> set[str]:
        views = {"period", "timeseries"}
        if self.peer_cohort_key:
            views.add("peer")
        if self.dimensions:
            views.add("breakdown")
        if self.computation == "median":
            views.add("histogram")
        return views


@dataclass
class CoverageReport:
    universe: dict[str, MetricDefinition]
    asserted: dict[str, dict[str, set[str]]]
    requested: set[str]
    missing: dict[str, set[str]] = field(default_factory=dict)
    unknown_asserted: set[str] = field(default_factory=set)
    unknown_requested: set[str] = field(default_factory=set)

    def __post_init__(self) -> None:
        universe_keys = set(self.universe)
        self.unknown_asserted = set(self.asserted) - universe_keys
        self.unknown_requested = self.requested - universe_keys
        for key, definition in self.universe.items():
            covered = set(self.asserted.get(key, {}))
            absent = definition.required_views - covered
            if absent:
                self.missing[key] = absent

    @property
    def passed(self) -> bool:
        return not self.missing and not self.unknown_asserted and not self.unknown_requested


def universe_from_file(path: str | Path) -> dict[str, MetricDefinition]:
    body = json.loads(Path(path).read_text(encoding="utf-8"))
    metrics = body.get("metrics", []) if isinstance(body, dict) else []
    return {
        str(metric["metric_key"]): MetricDefinition(
            metric_key=str(metric["metric_key"]),
            label=str(metric.get("label", "")),
            computation=str(metric["computation"]),
            dimensions=tuple(str(value) for value in metric.get("dimensions", [])),
            peer_cohort_key=metric.get("peer_cohort_key"),
        )
        for metric in metrics
    }


def coverage_from_ledgers(paths: Sequence[Path]) -> tuple[dict[str, dict[str, set[str]]], set[str]]:
    """What the spec modules asserted, merged across every ledger given.

    One ledger per suite run: a sharded lane produces several, each covering the
    metrics its own shard requested, and coverage is their union.
    """
    asserted: dict[str, dict[str, set[str]]] = {}
    requested: set[str] = set()
    for path in paths:
        document = json.loads(path.read_text(encoding="utf-8"))
        for key, views in document.get("asserted", {}).items():
            for view, names in views.items():
                asserted.setdefault(key, {}).setdefault(view, set()).update(names)
        requested.update(document.get("requested", []))
    return asserted, requested


def build_report(universe: dict[str, MetricDefinition], ledgers: Sequence[Path]) -> CoverageReport:
    asserted, requested = coverage_from_ledgers(ledgers)
    return CoverageReport(universe=universe, asserted=asserted, requested=requested)


def gate_violations(report: CoverageReport) -> list[str]:
    violations = [
        f"FAIL `{key}` — missing assertions for views: {', '.join(sorted(views))}"
        for key, views in sorted(report.missing.items())
    ]
    violations.extend(
        f"FAIL `{key}` — asserted but absent from the builtin metric registry"
        for key in sorted(report.unknown_asserted)
    )
    violations.extend(
        f"FAIL `{key}` — requested but absent from the builtin metric registry"
        for key in sorted(report.unknown_requested)
    )
    return violations


def render_markdown(report: CoverageReport) -> str:
    covered = len(report.universe) - len(report.missing)
    lines = [
        "# Unified builtin metric coverage",
        "",
        f"**Gate: {'PASS' if report.passed else 'FAIL'}.** {covered}/{len(report.universe)} metrics cover every supported view.",
        "",
        "| metric | computation | required views | covered views |",
        "|---|---|---|---|",
    ]
    for key, definition in sorted(report.universe.items()):
        covered_views = sorted(report.asserted.get(key, {}))
        lines.append(
            f"| `{key}` | {definition.computation} | {', '.join(sorted(definition.required_views))} | {', '.join(covered_views)} |"
        )
    violations = gate_violations(report)
    if violations:
        lines.extend(["", "## Violations", "", *[f"- {violation}" for violation in violations]])
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--universe-file", required=True)
    parser.add_argument(
        "--ledger", type=Path, nargs="+", default=[LEDGER_FILE], help="one per suite run"
    )
    parser.add_argument("--md", action="store_true")
    args = parser.parse_args(argv)
    missing = [path for path in args.ledger if not path.is_file()]
    if missing:
        parser.error(f"no assertion ledger at {', '.join(str(path) for path in missing)}")
    report = build_report(universe_from_file(args.universe_file), args.ledger)
    output = render_markdown(report)
    sys.stdout.write(output)
    for violation in gate_violations(report):
        sys.stderr.write(violation + "\n")
    return 0 if report.passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
