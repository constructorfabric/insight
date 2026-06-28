"""Metric-coverage gate: every metric the API serves is tested, or baseline-skipped.

Cross-checks, by metric_id, the metric universe — read over HTTP from a running
analytics-api (`GET /v1/metrics`: the enabled metric_ids `POST /v1/metrics/queries`
serves, seeded by the analytics-api migrations) — against the metric_ids each
`metrics/*.test.yaml` sends. The verdict per metric is **binary**:

  • covered    (a test queries it)        → PASS
  • skip-listed (in SKIP_LIST below)       → PASS (baseline)
  • neither                                → FAIL  (a new / unlisted metric)

The skip list is the accepted baseline — inline `SKIP_LIST` (single source of
truth, no side-car file). It is kept honest: a STALE entry (id no longer served)
or a REDUNDANT one (now covered by a test) also fails the gate, prompting you to
remove it. The overall gate is PASS iff there are no FAILs.

This module never spawns analytics-api — it reads the universe over HTTP only.
Entry point: `scripts/ci/metric_coverage.sh` boots MariaDB + analytics-api and
runs this with `ANALYTICS_API_URL` set (host needs only pyyaml + httpx). Ad hoc:
`ANALYTICS_API_URL=http://… python3 lib/metric_coverage.py [--md]`.
"""

from __future__ import annotations

import os
import sys
from dataclasses import dataclass, field
from pathlib import Path

import yaml

# The tenant header the API requires (mirrors lib.config.TENANT_HEADER). Any
# non-nil tenant resolves the middleware; migration-seeded metrics live under
# the GLOBAL (nil) tenant and `GET /v1/metrics` returns rows for
# `tenant_id IN [request_tenant, global]`, so the exact value doesn't matter.
TENANT_HEADER = "X-Insight-Tenant-Id"
DEFAULT_TENANT_ID = "00000000-0000-0000-0000-000000000001"

# lib/metric_coverage.py -> lib/ -> e2e/
_E2E_ROOT = Path(__file__).resolve().parents[1]
METRICS_DIR = _E2E_ROOT / "metrics"

# ── SKIP LIST (single source of truth) ───────────────────────────────────────
# metric_ids intentionally NOT covered by a test — the accepted baseline. Each
# `(metric_id, name, reason)`. A served metric_id that is neither tested nor
# listed here FAILS the gate. When you add a test for one of these, DELETE its
# row (a now-covered skip fails the gate — see `redundant_skips`).
SKIP_LIST: list[tuple[str, str, str]] = [
    # Blocked: bullet metrics with no seedable connector in the rig.
    ("00000000-0000-0000-0001-000000000006", "Team Bullet AI Adoption",
     "needs cursor / claude_code / chatgpt_team bronze fixtures."),
    ("00000000-0000-0000-0001-000000000013", "IC Bullet AI",
     "needs cursor / claude_code / chatgpt_team bronze fixtures."),
    ("00000000-0000-0000-0001-000000000004", "Team Bullet Code Quality",
     "needs bitbucket / CI bronze fixtures."),
    ("00000000-0000-0000-0001-000000000007", "Team Bullet Git",
     "needs bitbucket bronze fixtures."),
    ("00000000-0000-0000-0001-000000000018", "IC Bullet Git",
     "needs bitbucket bronze fixtures."),
    ("00000000-0000-0000-0001-000000000008", "IC Bullet Support",
     "needs zendesk bronze fixtures."),
    ("00000000-0000-0000-0001-000000000040", "Team Bullet Wiki",
     "needs confluence / outline bronze fixtures."),
    ("00000000-0000-0000-0001-000000000041", "IC Bullet Wiki",
     "needs confluence / outline bronze fixtures."),
    ("00000000-0000-0000-0001-000000000020", "CRM KPIs",
     "needs hubspot bronze fixtures."),
    ("00000000-0000-0000-0001-000000000021", "CRM Chart Deal Flow",
     "needs hubspot bronze fixtures."),
    ("00000000-0000-0000-0001-000000000022", "CRM Bullet Velocity Quality",
     "needs hubspot bronze fixtures."),
    ("00000000-0000-0000-0001-000000000023", "CRM Bullet Activity",
     "needs hubspot bronze fixtures."),
    ("00000000-0000-0000-0001-000000000028", "CRM Pipeline Now",
     "needs hubspot bronze fixtures."),
    ("00000000-0000-0000-0001-000000000010", "IC KPIs",
     "composite heatmap — needs cursor + bitbucket fixtures alongside jira/m365."),

    # Non-bullet query shapes (charts / drills / distributions / members / summary).
    ("00000000-0000-0000-0001-000000000001", "Executive Summary",
     "summary aggregate, not a value bullet."),
    ("00000000-0000-0000-0001-000000000002", "Team Members",
     "roster query, not a metric value."),
    ("00000000-0000-0000-0001-000000000014", "IC Chart LOC Trend",
     "time-series chart shape, not a bullet."),
    ("00000000-0000-0000-0001-000000000015", "IC Chart Delivery Trend",
     "time-series chart shape, not a bullet."),
    ("00000000-0000-0000-0001-000000000016", "IC Drill Detail",
     "drill-down detail rows, not a bullet."),
    ("00000000-0000-0000-0001-000000000017", "IC Time Off",
     "time-off calendar shape, not a bullet."),
    ("00000000-0000-0000-0001-000000000030", "IC Histogram",
     "distribution histogram shape, not a bullet."),
    ("00000000-0000-0000-0001-000000000036", "IC Section Trend",
     "section-level trend shape, not a bullet."),
    ("00000000-0000-0000-0001-000000000043", "Member PRs Merged",
     "per-member value row, not a team/IC bullet."),
    ("00000000-0000-0000-0001-000000000042", "Team Member Values — Git",
     "per-member value rows, not a bullet."),
    ("00000000-0000-0000-0001-000000000049", "Team Member Values — AI",
     "per-member value rows, not a bullet."),
    ("00000000-0000-0000-0001-000000000044", "Dept Distribution — Task Delivery",
     "department distribution shape, not a bullet."),
    ("00000000-0000-0000-0001-000000000045", "Dept Distribution — Collaboration",
     "department distribution shape, not a bullet."),
    ("00000000-0000-0000-0001-000000000046", "Dept Distribution — Git",
     "department distribution shape, not a bullet."),
    ("00000000-0000-0000-0001-000000000047", "Dept Distribution — Heatmap KPIs",
     "department distribution shape, not a bullet."),
    ("00000000-0000-0000-0001-000000000048", "Dept Distribution — AI",
     "department distribution shape, not a bullet."),
]

_WHERE = "SKIP_LIST in lib/metric_coverage.py"


def normalize_id(raw: str) -> str:
    """Canonicalize a metric_id to dash-less lowercase hex for comparison.

    The API returns dashed UUIDs; test files and `SKIP_LIST` use dashed UUIDs
    too — compare on the dash-less lowercase form to be representation-agnostic.
    """
    return raw.replace("-", "").strip().lower()


def skip_index() -> dict[str, dict]:
    """`{normalized_id: {metric_id, name, reason}}` from the inline `SKIP_LIST`.

    Raises on a duplicate metric_id so the list can't silently double-list one.
    """
    out: dict[str, dict] = {}
    for metric_id, name, reason in SKIP_LIST:
        key = normalize_id(metric_id)
        if key in out:
            raise ValueError(f"duplicate metric_id in SKIP_LIST: {metric_id}")
        out[key] = {"metric_id": metric_id, "name": name, "reason": reason}
    return out


def universe_from_url(base_url: str, tenant_id: str = DEFAULT_TENANT_ID) -> dict[str, str]:
    """`{normalized_id: name}` from `GET {base_url}/v1/metrics` — the metric
    universe, read over HTTP from a running analytics-api.

    The endpoint already returns exactly the enabled metrics for the request
    tenant (`is_enabled = true` AND `tenant_id IN [tenant, global]`), so no
    client-side filtering is needed. Response shape: `{"items": [{"id","name"}]}`.
    """
    import httpx  # local import: keeps the pure logic importable without httpx

    with httpx.Client(base_url=base_url, timeout=30.0, headers={TENANT_HEADER: tenant_id}) as c:
        resp = c.get("/v1/metrics")
        resp.raise_for_status()
        body = resp.json()
    items = body.get("items", []) if isinstance(body, dict) else (body or [])
    return {normalize_id(str(it["id"])): str(it.get("name", "")) for it in items}


def covered_from_tests(metrics_dir: Path = METRICS_DIR) -> dict[str, set[str]]:
    """`{normalized_id: {test files that query it}}` from request bodies.

    Plain `safe_load` (no $ref resolution): a metric_id is always a literal in
    `cases[].request.body.queries[].metric_id`, never a reference.
    """
    covered: dict[str, set[str]] = {}
    for path in sorted(metrics_dir.glob("*.test.yaml")):
        doc = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        for case in doc.get("cases") or []:
            body = (case.get("request") or {}).get("body") or {}
            for q in body.get("queries") or []:
                mid = q.get("metric_id")
                if mid:
                    covered.setdefault(normalize_id(str(mid)), set()).add(path.name)
    return covered


@dataclass
class CoverageReport:
    universe: dict[str, str]  # id -> name
    covered: dict[str, set[str]]  # id -> {files}
    skips: dict[str, dict]  # id -> skip entry

    # Derived sets (ids), populated in __post_init__.
    covered_in_universe: set[str] = field(default_factory=set)  # PASS (tested)
    skipped_active: set[str] = field(default_factory=set)  # PASS (baseline)
    uncovered: set[str] = field(default_factory=set)  # FAIL (new / unlisted)
    redundant_skips: set[str] = field(default_factory=set)  # FAIL (skip-listed AND covered)
    stale_skips: set[str] = field(default_factory=set)  # FAIL (skip for a non-existent id)
    unknown_covered: set[str] = field(default_factory=set)  # FAIL (test queries non-existent id)

    def __post_init__(self) -> None:
        u = set(self.universe)
        c = set(self.covered)
        s = set(self.skips)
        self.covered_in_universe = c & u
        self.unknown_covered = c - u
        self.redundant_skips = s & c
        self.stale_skips = s - u
        self.skipped_active = (s & u) - c
        self.uncovered = u - c - s

    @property
    def passed(self) -> bool:
        return not (self.uncovered or self.redundant_skips or self.stale_skips or self.unknown_covered)


def build_report(universe: dict[str, str], metrics_dir: Path = METRICS_DIR) -> CoverageReport:
    """Assemble the report. `universe` comes from `universe_from_url` (the
    metric_ids the API serves); covered + skips are local to the rig."""
    return CoverageReport(
        universe=universe,
        covered=covered_from_tests(metrics_dir),
        skips=skip_index(),
    )


def gate_violations(r: CoverageReport) -> list[str]:
    """Human-readable FAIL reasons. Empty list == gate PASS."""
    out: list[str] = []
    for i in sorted(r.uncovered):
        out.append(
            f"FAIL {r.universe[i]!r} ({i}) — served by the API but has no test and is not "
            f"skip-listed. Add a metrics/*.test.yaml that queries it, or add it to {_WHERE}."
        )
    for i in sorted(r.redundant_skips):
        files = ", ".join(sorted(r.covered[i]))
        out.append(
            f"FAIL {r.skips[i]['name']!r} ({i}) — skip-listed but now covered by [{files}]. "
            f"Remove its entry from {_WHERE}."
        )
    for i in sorted(r.stale_skips):
        out.append(
            f"FAIL {r.skips[i]['name']!r} ({i}) — skip-listed but no longer a served metric_id "
            f"(removed/disabled/renamed). Remove it from {_WHERE}."
        )
    for i in sorted(r.unknown_covered):
        files = ", ".join(sorted(r.covered[i]))
        out.append(
            f"FAIL metric_id {i} queried by [{files}] is not a served metric_id — typo in the "
            f"test, or the metric was removed/disabled."
        )
    return out


def render_text(r: CoverageReport) -> str:
    npass = len(r.covered_in_universe) + len(r.skipped_active)
    lines = [
        f"Metric coverage: {'PASS' if r.passed else 'FAIL'}  "
        f"({npass}/{len(r.universe)} pass — {len(r.covered_in_universe)} tested, "
        f"{len(r.skipped_active)} baseline; {len(r.uncovered)} fail)",
    ]
    for i in sorted(r.uncovered):
        lines.append(f"  ❌ FAIL     {i}  {r.universe[i]}  (no test, not skip-listed)")
    for i in sorted(r.covered_in_universe):
        lines.append(f"  ✅ tested   {i}  {r.universe[i]}  [{', '.join(sorted(r.covered[i]))}]")
    for i in sorted(r.skipped_active):
        lines.append(f"  ✅ baseline {i}  {r.universe[i]}  ({r.skips[i]['reason']})")
    for v in gate_violations(r):
        lines.append(f"  ✗ {v}")
    return "\n".join(lines)


def render_markdown(r: CoverageReport) -> str:
    """Single markdown table — binary verdict per metric_id, plus a skip-list
    hygiene footer for redundant/stale/unknown entries (which also fail)."""
    npass = len(r.covered_in_universe) + len(r.skipped_active)
    out = [
        "# Metric coverage — by metric_id",
        "",
        f"**Gate: {'✅ PASS' if r.passed else '❌ FAIL'}.** "
        f"{npass}/{len(r.universe)} pass "
        f"({len(r.covered_in_universe)} tested, {len(r.skipped_active)} baseline-skipped), "
        f"**{len(r.uncovered)} fail**.",
        "",
        "| verdict | basis | metric_id | name | detail |",
        "|---|---|---|---|---|",
    ]
    for i in sorted(r.uncovered):
        out.append(f"| ❌ FAIL | missing | `{i}` | {r.universe[i]} | no test, not in skip list |")
    for i in sorted(r.covered_in_universe):
        out.append(f"| ✅ PASS | tested | `{i}` | {r.universe[i]} | {', '.join(sorted(r.covered[i]))} |")
    for i in sorted(r.skipped_active):
        out.append(f"| ✅ PASS | baseline | `{i}` | {r.universe[i]} | {r.skips[i]['reason']} |")

    hygiene: list[str] = []
    for i in sorted(r.redundant_skips):
        files = ", ".join(sorted(r.covered[i]))
        hygiene.append(f"- `{i}` {r.skips[i]['name']} — skip-listed but now covered by [{files}]; remove from SKIP_LIST.")
    for i in sorted(r.stale_skips):
        hygiene.append(f"- `{i}` {r.skips[i]['name']} — skip-listed but no longer served; remove from SKIP_LIST.")
    for i in sorted(r.unknown_covered):
        files = ", ".join(sorted(r.covered[i]))
        hygiene.append(f"- `{i}` queried by [{files}] is not a served metric_id; fix the test.")
    if hygiene:
        out += ["", "## Skip-list issues (also fail the gate)", *hygiene]
    return "\n".join(out) + "\n"


def main(argv: list[str] | None = None) -> int:
    """CLI: print the coverage table/report; exit non-zero on any gate failure.

    `--md` prints the markdown status table (default: the plain-text report).

    Reads the universe over HTTP from a running analytics-api: set
    `ANALYTICS_API_URL` (and optionally `ANALYTICS_TENANT_ID`). The standalone CI
    script `scripts/ci/metric_coverage.sh` boots MariaDB + analytics-api and sets
    these for you. This module never spawns analytics-api itself.
    """
    args = argv if argv is not None else sys.argv[1:]
    url = os.environ.get("ANALYTICS_API_URL")
    if not url:
        print(
            "metric coverage: set ANALYTICS_API_URL to a running analytics-api, then "
            "re-run. The standalone gate `scripts/ci/metric_coverage.sh` does this for you.",
            file=sys.stderr,
        )
        return 2
    universe = universe_from_url(url, os.environ.get("ANALYTICS_TENANT_ID", DEFAULT_TENANT_ID))

    report = build_report(universe)
    if not report.universe:
        print(
            "metric coverage: GET /v1/metrics returned no enabled metrics — the "
            "catalog isn't seeded. Check analytics-api startup / migrations.",
            file=sys.stderr,
        )
        return 1
    as_md = "--md" in args
    print(render_markdown(report) if as_md else render_text(report))
    return 0 if report.passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
