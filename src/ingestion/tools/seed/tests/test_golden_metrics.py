"""The manifest's golden numbers are the numbers the generator actually writes.

`golden_metrics` promises exact per-person task totals; the only thing that
makes the promise safe is that its derivation and the row emitter read the SAME
deterministic plan (`generators.task.plan_issues`). These tests hold the two
ends together:

* the document lands in the manifest, versioned and internally consistent;
* the totals equal what `seed_task_field_history` writes, counted from the
  captured rows themselves — so a change to the emitter that bypasses the plan
  fails here rather than as a wrong exact assertion on a live stand.

Run against the installed package (see the README's develop section):

    uv run --extra dev pytest tests
"""

from __future__ import annotations

import datetime as _dt
import json
import pathlib

import pytest

from insight_seed import config, manifest, profiles
from insight_seed.generators import base, task
from insight_seed.golden_metrics import GOLDEN_METRICS_VERSION, task_totals

from .test_manifest_contract import _load_stand_reader

_EMAIL = "dev@company.nonpresent"
_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"
_ANCHOR = _dt.date(2026, 6, 30)
_DAYS = 60

_ENV = {
    "DEV_USER_EMAIL": _EMAIL,
    "TENANT_DEFAULT_ID": _TENANT,
    "SEED_ANCHOR_DATE": _ANCHOR.isoformat(),
    "SEED_DAYS": str(_DAYS),
    config.CROSS_TENANT_FIXTURE_ENV: "1",
}


def test_golden_metrics_land_in_the_manifest_and_add_up() -> None:
    doc = manifest.build_manifest(_ENV)
    golden = doc["golden_metrics"]

    assert golden["version"] == GOLDEN_METRICS_VERSION
    assert golden["tasks"]["window"] == doc["data_window"], (
        "the golden window and the seeded window must be the same range, "
        "or an exact assertion queries a period the numbers do not describe"
    )

    per_person = golden["tasks"]["per_person"]
    assert per_person, "no task person produced a golden entry"
    persona_emails = {p["email"] for p in doc["personas"]}
    for email, totals in per_person.items():
        assert email in persona_emails, f"golden entry for a non-persona: {email!r}"
        assert totals["bugs_fixed"] + totals["closed_non_bug"] <= totals["tasks_closed"], (
            f"should be a subset split of tasks_closed: {email!r} carries {totals}"
        )
    assert sum(t["tasks_closed"] for t in per_person.values()) > 0, (
        "a 60-day window with the demo roster closes issues; all-zero golden "
        "totals mean the derivation lost the plan"
    )


def test_a_manifest_without_golden_metrics_still_loads_in_the_stand_suite() -> None:
    """The field is additive: a stand seeded before it existed must not brick
    the suite, so absence parses to None rather than raising."""
    reader = _load_stand_reader()
    doc = json.loads(json.dumps(manifest.build_manifest(_ENV)))
    del doc["golden_metrics"]

    parsed = reader.Manifest.parse(doc, source_path=pathlib.Path("contract-test"))

    assert parsed.golden_metrics is None


def test_a_newer_golden_metrics_version_parses_as_payload_absent() -> None:
    """Self-versioned: a future shape must not brick older readers — they see
    the version and skip, never misread the payload."""
    reader = _load_stand_reader()
    doc = json.loads(json.dumps(manifest.build_manifest(_ENV)))
    doc["golden_metrics"] = {"version": GOLDEN_METRICS_VERSION + 1, "tasks": "reshaped"}

    parsed = reader.Manifest.parse(doc, source_path=pathlib.Path("contract-test"))

    assert parsed.golden_metrics is not None
    assert parsed.golden_metrics.version == GOLDEN_METRICS_VERSION + 1
    assert parsed.golden_metrics.tasks is None


def test_golden_coverage_equals_the_roster_task_seeding_runs_over() -> None:
    """Silver task seeding runs over the DEFAULT tenant's roster only, while the
    manifest builder hands `build_golden_metrics` a roster that also carries the
    other-tenant fixture. Golden entries and seeded task rows must name the same
    people exactly — an entry for an unseeded persona is a promise the stand
    cannot keep, and a seeded task person without an entry is a lost oracle.
    Today the other-tenant lead is teamless and `task_persons` drops them; this
    pins that the exclusion holds however the fixture roster evolves.
    """
    doc = manifest.build_manifest(_ENV)
    golden_emails = set(doc["golden_metrics"]["tasks"]["per_person"])

    default_roster = profiles.build_seeded_roster(_EMAIL, config.DEFAULT_ORG_HEADCOUNT)
    seeded_task_emails = {p.email for p in task.task_persons(default_roster)}
    other_tenant_emails = {p.email for p in profiles.build_other_tenant_roster()}

    assert golden_emails == seeded_task_emails, (
        "golden per_person and the task generator's roster diverged: "
        f"golden-only={sorted(golden_emails - seeded_task_emails)}, "
        f"seeded-only={sorted(seeded_task_emails - golden_emails)}"
    )
    assert not golden_emails & other_tenant_emails, (
        "golden totals advertised for other-tenant personas that task seeding "
        f"never writes rows for: {sorted(golden_emails & other_tenant_emails)}"
    )


def test_two_builds_at_the_same_anchor_agree() -> None:
    roster = profiles.build_seeded_roster(_EMAIL, config.DEFAULT_ORG_HEADCOUNT)

    assert task_totals(roster, _DAYS, _ANCHOR) == task_totals(roster, _DAYS, _ANCHOR)


def test_golden_totals_equal_the_rows_the_generator_writes(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Counted from the emitted rows, not from the shared plan.

    The derivation and the emitter both call `plan_issues`; what this pins is
    the remaining gap — that the emitter turns every planned close into exactly
    one changelog row and attributes it to the planning person.
    """
    monkeypatch.setattr(base, "_anchor_cache", _ANCHOR)
    captured: dict[str, object] = {}

    def _capture(
        client: object,
        schema: str,
        table: str,
        cols: list[str],
        rows: list[tuple[object, ...]],
    ) -> int:
        captured["cols"] = cols
        captured["rows"] = rows
        return len(rows)

    monkeypatch.setattr(task, "truncate", lambda *args: None)
    monkeypatch.setattr(task, "bulk_insert", _capture)

    roster = profiles.build_seeded_roster(_EMAIL, config.DEFAULT_ORG_HEADCOUNT)
    task.seed_task_field_history(None, roster, _TENANT, _DAYS)  # type: ignore[arg-type]

    cols = captured["cols"]
    assert isinstance(cols, list)
    kind = cols.index("event_kind")
    field = cols.index("field_id")
    issue = cols.index("issue_id")
    author = cols.index("author_id")
    display = cols.index("delta_value_display")

    issue_type: dict[str, str] = {}
    issue_author: dict[str, str] = {}
    closed: set[str] = set()
    rows = captured["rows"]
    assert isinstance(rows, list) and rows
    for row in rows:
        if row[kind] == "synthetic_initial" and row[field] == "issuetype":
            issue_type[row[issue]] = row[display]
            issue_author[row[issue]] = row[author]
        elif row[kind] == "changelog" and row[field] == "status":
            closed.add(row[issue])

    observed: dict[str, dict[str, int]] = {}
    for issue_id in closed:
        totals = observed.setdefault(
            issue_author[issue_id],
            {"tasks_closed": 0, "bugs_fixed": 0, "closed_non_bug": 0},
        )
        totals["tasks_closed"] += 1
        issue_kind = task._ISSUE_TYPE_DIM[issue_type[issue_id]][1]
        totals["bugs_fixed" if issue_kind == "bug" else "closed_non_bug"] += 1

    expected = {
        email: {
            "tasks_closed": t.tasks_closed,
            "bugs_fixed": t.bugs_fixed,
            "closed_non_bug": t.closed_non_bug,
        }
        for email, t in task_totals(roster, _DAYS, _ANCHOR).items()
        if t.tasks_closed
    }
    assert observed == expected
