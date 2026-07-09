"""Metric-coverage gate: every metric_key the catalog exposes has its value tested.

Cross-checks, **by metric_key**, the metric universe — read over HTTP from a
running analytics (`POST /v1/catalog/get_metrics`: the enabled product
metric_keys, each a `<storage_table>.<column>` seeded by the analytics
migrations) — against the metric_keys whose VALUE the tests assert
(`find: {metric_key: …}` paired with `equal`/`assert` in the same rule). Binary
verdict per metric_key:

  • value-asserted by a test       → PASS
  • skip-listed (SKIP_LIST below)   → PASS (baseline)
  • neither                         → FAIL  (a number nobody validates)

Catalog keys are dotted (`collab_bullet_rows.m365_emails_sent`); a test asserts
either the bare response key (`m365_emails_sent`, the bullet paths) or the full
dotted key (`collab_person_counter_daily.messages_sent`, the unified-metrics
path). A dotted assertion matches the catalog directly; a bare one maps to its
dotted key by the column suffix, which is unique across the catalog (a future
collision raises — see `CoverageReport.__post_init__`).

The skip list is the accepted baseline — inline `SKIP_LIST` (single source of
truth, no side-car file). Kept honest: a STALE entry (key no longer in the
catalog) or a REDUNDANT one (now value-tested) also fails. PASS iff no FAILs.

This module never spawns analytics. CI: the `metric-coverage-gate` job reads
the universe from `--universe-file catalog_metrics.json` — the artifact the e2e
run collects (lib/collect_metrics.py) — so no app boot. Locally:
`./e2e.sh test` then `./e2e.sh gates`. Ad hoc against a running API:
`ANALYTICS_URL=http://… python3 lib/metric_coverage.py [--md]`.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass, field
from pathlib import Path

import yaml

# The tenant header the API requires (mirrors lib.config.TENANT_HEADER). Any
# non-nil tenant resolves the middleware; the catalog rows are tenant-NULL
# (global), so `get_metrics` returns them for any resolved tenant.
TENANT_HEADER = "X-Insight-Tenant-Id"
DEFAULT_TENANT_ID = "00000000-0000-0000-0000-000000000001"

# lib/metric_coverage.py -> lib/ -> e2e/
_E2E_ROOT = Path(__file__).resolve().parents[1]
METRICS_DIR = _E2E_ROOT / "metrics"
_WHERE = "SKIP_LIST / SKIP_TABLES in lib/metric_coverage.py"

# ── SKIP RULES (the accepted baseline; single source of truth) ───────────────
# A served metric_key neither value-asserted by a test nor skipped here FAILS the
# gate. Two layers, both keyed by the catalog's `<table>.<column>`:
#
#   SKIP_TABLES — a whole storage table (vector) with NO connector in the e2e rig:
#     EVERY key under it is auto-skipped, so a new column needs no hand-edit.
#   SKIP_LIST   — per-key skips for MIXED tables (the vector also has tested or
#     differently-blocked keys). `reachable — …` rows are the actionable backlog.
#
# The reason is shown verbatim as the skip's status, so keep it a concise phrase.
# Hygiene (FAIL): an explicit SKIP_LIST entry that is now value-tested, or no
# longer a catalog key. (A table-covered key becoming tested is fine — it's just
# covered; the table rule still covers the rest of the vector.)
SKIP_TABLES: dict[str, str] = {
    "ai_bullet_rows": "needs Cursor/Claude/ChatGPT connector",
    "ai_person_counter_daily": "needs Cursor/Claude/ChatGPT connector",
    "code_quality_bullet_rows": "needs Bitbucket/CI connector",
    "crm_bullet_rows": "needs HubSpot connector",
    "git_bullet_rows": "needs Bitbucket connector",
    "ic_kpis": "composite KPI — needs Cursor+Bitbucket",
    "support_bullet_rows": "needs Zendesk connector",
    "wiki_bullet_rows": "needs Confluence/Outline connector",
}

SKIP_LIST: list[tuple[str, str]] = [
    # collab_bullet_rows — Slack has no connector; Zoom fixtures exist (reachable)
    ("collab_bullet_rows.slack_active_days", "needs Slack connector"),
    ("collab_bullet_rows.slack_channel_posts", "needs Slack connector"),
    ("collab_bullet_rows.slack_messages_sent", "needs Slack connector"),
    ("collab_bullet_rows.slack_msgs_per_active_day", "needs Slack connector"),
    ("collab_bullet_rows.slack_dm_ratio", "needs Slack connector"),
    ("collab_bullet_rows.zoom_meeting_hours", "reachable — Zoom fixtures exist"),
    ("collab_bullet_rows.zoom_meetings", "reachable — Zoom fixtures exist"),
    # collab_person_counter_daily — collaboration messaging (M365 Teams / Zulip)
    ("collab_person_counter_daily.channel_posts", "reachable — Teams/Zulip fixtures exist"),
    ("collab_person_counter_daily.messages_sent", "reachable — Teams/Zulip fixtures exist"),
    # task_delivery_bullet_rows — Jira fixtures exist (reachable backlog)
    ("task_delivery_bullet_rows.avg_slip", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.estimation_accuracy", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.flow_efficiency", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.mean_time_to_resolution", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.on_time_delivery", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.overrun_ratio", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.pickup_time", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.scope_completion", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.scope_creep", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.stale_in_progress", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.task_dev_time", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.task_reopen_rate", "reachable — Jira fixtures exist"),
    ("task_delivery_bullet_rows.worklog_logging_accuracy", "reachable — Jira fixtures exist"),
]


def suffix(metric_key: str) -> str:
    """The `<column>` part of a `<table>.<column>` catalog key (or the bare key)."""
    return metric_key.split(".", 1)[-1]


def resolve_skips(universe: dict[str, str]) -> tuple[dict[str, str], set[str]]:
    """`({metric_key: reason}, explicit_keys)` — the skip baseline for `universe`.

    SKIP_LIST is per-key; SKIP_TABLES auto-skips every universe key under a
    connector-less vector. `explicit_keys` (the SKIP_LIST keys) is what drives the
    redundant/stale hygiene — a table-covered key is not hand-maintained, so it
    never goes redundant or stale. Raises on a duplicate SKIP_LIST key, or one
    whose table is already a SKIP_TABLES rule (doubly-specified).
    """
    explicit: dict[str, str] = {}
    for key, reason in SKIP_LIST:
        if key in explicit:
            raise ValueError(f"duplicate metric_key in SKIP_LIST: {key}")
        if _vector(key) in SKIP_TABLES:
            raise ValueError(
                f"SKIP_LIST key {key} is already covered by SKIP_TABLES[{_vector(key)!r}] — "
                f"drop the per-key entry."
            )
        explicit[key] = reason
    resolved = dict(explicit)
    for k in universe:
        if _vector(k) in SKIP_TABLES:
            resolved.setdefault(k, SKIP_TABLES[_vector(k)])
    return resolved, set(explicit)


def _universe_from_body(body: object) -> dict[str, str]:
    """Parse a `POST /v1/catalog/get_metrics` body into `{metric_key: label}`.

    Response shape: `{"metrics": [{"metric_key", "label", ...}]}`.
    """
    metrics = body.get("metrics", []) if isinstance(body, dict) else []
    return {str(m["metric_key"]): str(m.get("label", "")) for m in metrics}


def universe_from_url(base_url: str, tenant_id: str = DEFAULT_TENANT_ID) -> dict[str, str]:
    """`{metric_key: label}` from `POST {base_url}/v1/catalog/get_metrics` — the
    enabled product metric_keys (dotted `<table>.<column>`).

    Sourced from the API (not a raw `metric_catalog` SELECT) so the gate checks
    the contract consumers see; the endpoint already returns exactly the enabled
    catalog rows.
    """
    import httpx  # local import: keeps the pure logic importable without httpx

    with httpx.Client(base_url=base_url, timeout=30.0, headers={TENANT_HEADER: tenant_id}) as c:
        resp = c.post("/v1/catalog/get_metrics", json={})
        resp.raise_for_status()
        body = resp.json()
    return _universe_from_body(body)


def universe_from_file(path: str | Path) -> dict[str, str]:
    """`{metric_key: label}` from a saved `POST /v1/catalog/get_metrics` response
    — the `catalog_metrics.json` artifact the e2e run collects. Lets the CI gate
    analyse the universe from a file without booting analytics.
    """
    return _universe_from_body(json.loads(Path(path).read_text(encoding="utf-8")))


def asserted_keys_from_tests(metrics_dir: Path = METRICS_DIR) -> dict[str, set[str]]:
    """`{metric_key: {test files}}` — keys whose VALUE a test checks (bare or dotted).

    A key counts only when a `find: {metric_key: …}` selector is paired with an
    `equal` or `assert` in the SAME expect rule (i.e. the value is validated, not
    merely selected). Plain `safe_load` — a metric_key is always a literal.

    Recurses (`rglob`) so nested fixtures are seen — the rig discovers tests as
    `metrics/**/*.test.yaml` (`lib.fixture_loader.discover_tests`); a flat glob
    here would miss a nested fixture and flag its key as falsely uncovered.
    """
    out: dict[str, set[str]] = {}
    for path in sorted(metrics_dir.rglob("*.test.yaml")):
        doc = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        for case in doc.get("cases") or []:
            for rule in case.get("expect") or []:
                mk = (rule.get("find") or {}).get("metric_key")
                if mk and ("equal" in rule or "assert" in rule):
                    out.setdefault(str(mk), set()).add(path.name)
    return out


@dataclass
class CoverageReport:
    universe: dict[str, str]  # metric_key (dotted) -> label
    asserted: dict[str, set[str]]  # bare metric_key -> {files}
    skips: dict[str, str]  # metric_key (dotted) -> reason (resolved: SKIP_LIST + SKIP_TABLES)
    explicit_skips: set[str] = field(default_factory=set)  # hand-maintained SKIP_LIST keys (drive hygiene)

    # Derived sets (dotted metric_keys unless noted), populated in __post_init__.
    covered: set[str] = field(default_factory=set)  # PASS (value-tested)
    skipped_active: set[str] = field(default_factory=set)  # PASS (baseline)
    uncovered: set[str] = field(default_factory=set)  # FAIL (a number nobody validates)
    redundant_skips: set[str] = field(default_factory=set)  # FAIL (skip-listed AND tested)
    stale_skips: set[str] = field(default_factory=set)  # FAIL (skip for a non-existent key)
    stale_tables: set[str] = field(default_factory=set)  # FAIL (SKIP_TABLES vector no longer in the catalog)
    unknown_asserted: set[str] = field(default_factory=set)  # FAIL (bare key, no catalog match)

    def __post_init__(self) -> None:
        # Map the catalog's dotted keys by their unique column suffix so a test's
        # bare assertion key resolves to one catalog key.
        by_suffix: dict[str, str] = {}
        for k in self.universe:
            s = suffix(k)
            if s in by_suffix:
                raise ValueError(
                    f"catalog suffix collision {s!r} ({by_suffix[s]} vs {k}) — "
                    f"bare→dotted suffix mapping is unsafe; scope by metric_id instead."
                )
            by_suffix[s] = k

        for bare in self.asserted:
            # A fully-qualified dotted key (the response `metric_key` of the
            # unified-metrics path, e.g. `collab_person_counter_daily.messages_sent`)
            # matches the catalog directly; bare column keys resolve via the
            # unique suffix. Without the direct match, dotted assertions were
            # miscounted as unasserted and their keys flagged MISSING.
            full = bare if bare in self.universe else by_suffix.get(bare)
            (self.covered if full else self.unknown_asserted).add(full or bare)

        u, s = set(self.universe), set(self.skips)
        # redundant/stale apply only to hand-maintained SKIP_LIST entries; a
        # table-covered key that gets tested is simply covered (its SKIP_TABLES
        # rule stays valid for the rest of the vector).
        self.redundant_skips = self.explicit_skips & self.covered
        self.stale_skips = self.explicit_skips - u
        # A whole SKIP_TABLES vector can also go stale: if the catalog serves NO
        # key under it, the table rule is dead and must be removed. (A table whose
        # keys are all now tested is NOT flagged — the rule still exists to
        # auto-cover future keys of that connector-less vector.)
        self.stale_tables = {t for t in SKIP_TABLES if not any(_vector(k) == t for k in u)}
        self.skipped_active = (s & u) - self.covered
        self.uncovered = u - self.covered - s

    @property
    def passed(self) -> bool:
        return not (
            self.uncovered
            or self.redundant_skips
            or self.stale_skips
            or self.stale_tables
            or self.unknown_asserted
        )

    def files_for(self, full_key: str) -> set[str]:
        return self.asserted.get(suffix(full_key), set())


def build_report(universe: dict[str, str], metrics_dir: Path = METRICS_DIR) -> CoverageReport:
    """Assemble the report. `universe` comes from `universe_from_url` (the catalog
    metric_keys the API serves); asserted + skips are local to the rig."""
    resolved, explicit = resolve_skips(universe)
    return CoverageReport(
        universe=universe,
        asserted=asserted_keys_from_tests(metrics_dir),
        skips=resolved,
        explicit_skips=explicit,
    )


def gate_violations(r: CoverageReport) -> list[str]:
    """Human-readable FAIL reasons. Empty list == gate PASS."""
    out: list[str] = []
    for k in sorted(r.uncovered):
        out.append(
            f"FAIL `{k}` — served by the catalog but no test asserts its value and it is "
            f"not skip-listed. Add a `find: {{metric_key: {suffix(k)}}}` + `equal`/`assert`, "
            f"or add it to {_WHERE}."
        )
    for k in sorted(r.redundant_skips):
        files = ", ".join(sorted(r.files_for(k)))
        out.append(
            f"FAIL `{k}` — skip-listed but now value-tested by [{files}]. Remove its entry "
            f"from {_WHERE}."
        )
    for k in sorted(r.stale_skips):
        out.append(
            f"FAIL `{k}` — skip-listed but no longer a catalog metric_key (removed/renamed). "
            f"Remove it from {_WHERE}."
        )
    for t in sorted(r.stale_tables):
        out.append(
            f"FAIL SKIP_TABLES[`{t}`] — the catalog serves no metric_key under this vector "
            f"(connector/table removed or renamed). Remove the table rule from {_WHERE}."
        )
    for bare in sorted(r.unknown_asserted):
        files = ", ".join(sorted(r.asserted[bare]))
        out.append(
            f"FAIL `{bare}` — asserted by [{files}] but is not a catalog metric_key (typo, or "
            f"an unseeded key that matches 0 rows)."
        )
    return out


# Friendly vector names for the storage tables (display only).
_VECTOR_NAMES = {
    "collab_bullet_rows": "Collaboration",
    "task_delivery_bullet_rows": "Task Delivery",
    "ai_bullet_rows": "AI Adoption",
    "git_bullet_rows": "Git Activity",
    "code_quality_bullet_rows": "Code Quality",
    "crm_bullet_rows": "CRM / Sales",
    "support_bullet_rows": "Support",
    "wiki_bullet_rows": "Wiki / Knowledge",
    "ic_kpis": "IC KPIs (heatmap)",
}


def _vector(metric_key: str) -> str:
    return metric_key.split(".", 1)[0]


def _vector_name(table: str) -> str:
    return _VECTOR_NAMES.get(table, table)


def _by_table(keys) -> dict[str, list[str]]:
    groups: dict[str, list[str]] = {}
    for k in keys:
        groups.setdefault(_vector(k), []).append(k)
    return groups


def _pct(n: int, d: int) -> str:
    return f"{round(100 * n / d)}%" if d else "—"


def _is_reachable(reason: str) -> bool:
    """A skip whose fixtures already exist — the actionable backlog."""
    return reason.lower().startswith("reachable")


def _skips_by_reason(r: CoverageReport) -> list[tuple[str, int]]:
    """`[(reason, count)]` over active skips, most-common first."""
    counts: dict[str, int] = {}
    for k in r.skipped_active:
        counts[r.skips[k]] = counts.get(r.skips[k], 0) + 1
    return sorted(counts.items(), key=lambda x: (-x[1], x[0]))


def render_markdown(r: CoverageReport) -> str:
    """Markdown report: a per-vector summary + the reachable backlog up top, then
    the full per-key detail (collapsed), then a skip-list-hygiene footer."""
    cov, skp, tot, miss = (
        len(r.covered), len(r.skipped_active), len(r.universe), len(r.uncovered),
    )
    out = [
        "# Metric coverage — by metric_key",
        "",
        f"**Gate: {'✅ PASS' if r.passed else '❌ FAIL'}.** "
        f"{cov}/{tot} numbers validated ({_pct(cov, tot)}) · {skp} baseline-skipped · "
        f"**{miss} missing**.",
    ]

    # ── Per-vector summary ───────────────────────────────────────────────────
    tables = _by_table(r.universe)
    out += ["", "## Coverage by vector", "",
            "| vector | tested | skipped | missing | coverage |",
            "|---|--:|--:|--:|--:|"]
    for t in sorted(tables, key=lambda x: (-sum(1 for k in tables[x] if k in r.covered), x)):
        keys = tables[t]
        c = sum(1 for k in keys if k in r.covered)
        s = sum(1 for k in keys if k in r.skipped_active)
        m = sum(1 for k in keys if k in r.uncovered)
        out.append(f"| {_vector_name(t)} | {c} | {s} | {m} | {_pct(c, len(keys))} |")
    out.append(f"| **Total** | **{cov}** | **{skp}** | **{miss}** | **{_pct(cov, tot)}** |")

    # ── Why the skips are skipped ────────────────────────────────────────────
    by_reason = _skips_by_reason(r)
    if by_reason:
        out += ["", "## Skipped — by reason", "", "| reason | keys |", "|---|--:|"]
        for reason, n in by_reason:
            out.append(f"| {reason} | {n} |")

    # ── Reachable backlog (fixtures exist — just write the assertion) ─────────
    backlog = sorted(k for k in r.skipped_active if _is_reachable(r.skips[k]))
    if backlog:
        out += ["", f"## Reachable now — backlog ({len(backlog)})",
                "_Fixtures already exist; each just needs a `find:`+`equal` assertion in a test._",
                ""]
        for k in backlog:
            out.append(f"- **{r.universe[k] or suffix(k)}** — `{suffix(k)}` ({_vector_name(_vector(k))})")

    # ── Full per-key detail (collapsed) ──────────────────────────────────────
    out += ["", "<details><summary>Per-key detail (all "
            f"{tot})</summary>", ""]
    for t in sorted(tables):
        keys = sorted(tables[t])
        c = sum(1 for k in keys if k in r.covered)
        out += ["", f"### {_vector_name(t)} (`{t}`) — {c}/{len(keys)}", "",
                "| status | metric | key | detail |", "|---|---|---|---|"]
        for k in keys:
            col, label = suffix(k), (r.universe[k] or suffix(k))
            if k in r.uncovered:
                out.append(f"| ❌ MISSING | {label} | `{col}` | no value assertion, not skip-listed |")
            elif k in r.covered:
                out.append(f"| ✅ tested | {label} | `{col}` | {', '.join(sorted(r.files_for(k)))} |")
            else:
                out.append(f"| ⏭️ {r.skips[k]} | {label} | `{col}` | |")
    out += ["", "</details>"]

    # ── Skip-list hygiene (these also fail the gate) ─────────────────────────
    hygiene: list[str] = []
    for k in sorted(r.redundant_skips):
        hygiene.append(f"- `{k}` skip-listed but now tested by [{', '.join(sorted(r.files_for(k)))}]; remove from SKIP_LIST.")
    for k in sorted(r.stale_skips):
        hygiene.append(f"- `{k}` skip-listed but no longer in the catalog; remove from SKIP_LIST.")
    for bare in sorted(r.unknown_asserted):
        hygiene.append(f"- `{bare}` asserted by [{', '.join(sorted(r.asserted[bare]))}] is not a catalog metric_key (typo/unseeded).")
    if hygiene:
        out += ["", "## Skip-list issues (also fail the gate)", *hygiene]
    return "\n".join(out) + "\n"


def main(argv: list[str] | None = None) -> int:
    """CLI: print the coverage table/report; exit non-zero on any gate failure.

    Two universe sources:
      --universe-file <catalog_metrics.json>  the artifact the e2e run collects
                                               (CI gate — no analytics boot)
      else $ANALYTICS_URL                  fetch live (local standalone runs)
    This module never spawns analytics itself.
    """
    p = argparse.ArgumentParser(description="Metric-coverage gate (by metric_key).")
    p.add_argument(
        "--universe-file",
        help="catalog_metrics.json (a saved POST /v1/catalog/get_metrics response); "
        "default: fetch from $ANALYTICS_URL",
    )
    args = p.parse_args(argv)

    if args.universe_file:
        universe = universe_from_file(args.universe_file)
    else:
        url = os.environ.get("ANALYTICS_URL")
        if not url:
            print(
                "metric coverage: pass --universe-file <catalog_metrics.json> or set "
                "ANALYTICS_URL to a running analytics.",
                file=sys.stderr,
            )
            return 2
        universe = universe_from_url(url, os.environ.get("ANALYTICS_TENANT_ID", DEFAULT_TENANT_ID))

    report = build_report(universe)
    if not report.universe:
        print(
            "metric coverage: empty universe — the catalog isn't seeded (live) or the "
            "collected catalog_metrics.json is empty.",
            file=sys.stderr,
        )
        return 1
    print(render_markdown(report))
    return 0 if report.passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
