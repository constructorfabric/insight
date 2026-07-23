from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

import yaml


_E2E_ROOT = Path(__file__).resolve().parents[1]
METRICS_DIR = _E2E_ROOT / "metrics"
_VIEW_FIELDS = {
    "period": {"value"},
    "peer": {"target_value", "p25", "median", "p75", "min", "max", "n"},
    "timeseries": {"points"},
    "breakdown": {"value"},
    "histogram": {"bins"},
}


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


def _asserted_fields(rule: dict) -> set[str]:
    fields = set((rule.get("equal") or {}).keys())
    fields.update((rule.get("contains") or {}).keys())
    fields.update(rule.get("nonempty") or [])
    expression = rule.get("assert") or ""
    fields.update(re.findall(r"\bit\.([a-zA-Z_][a-zA-Z0-9_]*)\b", expression))
    fields.update(re.findall(r"\bit\[['\"]([a-zA-Z_][a-zA-Z0-9_]*)['\"]\]", expression))
    return fields


def _covers_view(rule: dict, view: str) -> bool:
    required = _VIEW_FIELDS[view]
    if "find" in rule:
        return required <= _asserted_fields(rule)
    expression = rule.get("assert") or ""
    if view == "timeseries":
        return ".points" in expression
    if view == "breakdown":
        return (
            ".dimensions" in expression and ".value" in expression
        ) or bool(re.search(r"size\(items\)\s*==\s*0", expression))
    if view == "histogram":
        return ".bins" in expression
    return False


def coverage_from_tests(
    metrics_dir: Path = METRICS_DIR,
) -> tuple[dict[str, dict[str, set[str]]], set[str]]:
    asserted: dict[str, dict[str, set[str]]] = {}
    requested: set[str] = set()
    for path in sorted(metrics_dir.rglob("*.test.yaml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        for case in document.get("cases") or []:
            body = (case.get("request") or {}).get("body") or {}
            for metric in body.get("metrics") or []:
                key = metric.get("metric_key")
                if key:
                    requested.add(str(key))
            for rule in case.get("expect") or []:
                key = rule.get("metric")
                view = rule.get("view")
                if not key or view not in _VIEW_FIELDS:
                    continue
                if _covers_view(rule, view):
                    asserted.setdefault(str(key), {}).setdefault(view, set()).add(path.name)
    return asserted, requested


def build_report(
    universe: dict[str, MetricDefinition], metrics_dir: Path = METRICS_DIR
) -> CoverageReport:
    asserted, requested = coverage_from_tests(metrics_dir)
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
    parser.add_argument("--metrics-dir", type=Path, default=METRICS_DIR)
    parser.add_argument("--md", action="store_true")
    args = parser.parse_args(argv)
    report = build_report(universe_from_file(args.universe_file), args.metrics_dir)
    output = render_markdown(report)
    print(output, end="")
    for violation in gate_violations(report):
        print(violation, file=sys.stderr)
    return 0 if report.passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
