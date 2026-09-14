from __future__ import annotations

import datetime as _dt
import statistics
from typing import Any

import pytest

from insight_seed import profiles
from insight_seed.generators import base, ci, ci_deployments, git_history

_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"
_ANCHOR = _dt.date(2026, 8, 11)
# INVARIANT: must stay wider than ci.CI_WINDOW_DAYS (90), or the history built
# from it never reaches past the clamp and test_the_ci_window_is_clamped_to_
# the_vendor_retention below passes vacuously — see
# test_the_fixture_history_extends_past_the_ci_window.
_DAYS = 180

_NO_CLIENT: Any = None

Rows = dict[str, list[dict[str, Any]]]
Runs = list[dict[str, Any]]


@pytest.fixture(autouse=True)
def pinned_anchor(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(base, "_anchor_cache", _ANCHOR)


@pytest.fixture
def rows(monkeypatch: pytest.MonkeyPatch) -> Rows:
    captured: Rows = {}

    def capture(
        client: Any, schema: str, table: str, cols: list[str], data: list[tuple[Any, ...]]
    ) -> int:
        captured[f"{schema}.{table}"] = [dict(zip(cols, row, strict=True)) for row in data]
        return len(data)

    monkeypatch.setattr(ci, "truncate", lambda client, schema, table: None)
    monkeypatch.setattr(ci, "bulk_insert", capture)
    monkeypatch.setattr(ci_deployments, "truncate", lambda client, schema, table: None)
    monkeypatch.setattr(ci_deployments, "bulk_insert", capture)

    roster = profiles.build_roster("dev@company.nonpresent")
    history = git_history.build_history(roster, _DAYS)
    ci.generate(_NO_CLIENT, roster, _TENANT, _DAYS, history)
    return captured


@pytest.fixture
def runs(rows: Rows) -> Runs:
    return rows["bronze_github.workflow_runs"]


def test_both_gate_and_non_gate_triggers_are_present(runs: Runs) -> None:
    events = {r["event"] for r in runs}
    assert {"push", "pull_request", "merge_group"} <= events
    assert {"schedule", "workflow_dispatch"} <= events


@pytest.mark.parametrize(
    "conclusion",
    ["success", "failure", "timed_out", "cancelled", "skipped", "action_required", None],
)
def test_every_outcome_including_undecided_is_present(runs: Runs, conclusion: str | None) -> None:
    present = [r for r in runs if r["conclusion"] == conclusion]
    assert present, f"should emit runs concluding {conclusion!r}"


def test_retries_appear_both_with_and_without_the_earlier_attempt(runs: Runs) -> None:
    by_run: dict[int, set[int]] = {}
    for r in runs:
        by_run.setdefault(r["id"], set()).add(r["run_attempt"])
    retried = {rid: att for rid, att in by_run.items() if max(att) > 1}
    assert retried, "no retried runs"
    assert any(att == {2} for att in retried.values()), "no retry with the first attempt dropped"
    assert any(att == {1, 2} for att in retried.values()), "no retry with both attempts present"


def test_some_runs_have_zero_duration(runs: Runs) -> None:
    zero = [r for r in runs if r["run_started_at"] == r["updated_at"]]
    assert 0.01 < len(zero) / len(runs) < 0.10, (
        f"zero-duration share is {len(zero) / len(runs):.3f}"
    )


def test_durations_are_fat_tailed(runs: Runs) -> None:
    seconds = sorted(
        (
            _dt.datetime.fromisoformat(r["updated_at"])
            - _dt.datetime.fromisoformat(r["run_started_at"])
        ).total_seconds()
        for r in runs
        if r["conclusion"] is not None and r["run_started_at"] != r["updated_at"]
    )
    median = statistics.median(seconds)
    p90 = seconds[int(len(seconds) * 0.9)]
    assert p90 > 4 * median, f"p90 {p90} is not a tail over median {median}"


def test_repository_volume_is_skewed_and_one_repository_has_no_runs(runs: Runs) -> None:
    from insight_seed.generators import ci_topology

    counts: dict[str, int] = {}
    for r in runs:
        counts[r["repo_full_name"]] = counts.get(r["repo_full_name"], 0) + 1
    ordered = sorted(counts.values(), reverse=True)
    assert ordered[0] > 2 * ordered[-1], "run volume is flat across repositories"
    grid = ci_topology.repo_grid(profiles.build_roster("dev@company.nonpresent"))
    assert "acme/legacy-archive" not in counts
    assert all(r.full_name not in counts for r in grid if r.vendor != "github")


def test_runs_are_anchored_to_the_seeded_commits(runs: Runs) -> None:
    roster = profiles.build_roster("dev@company.nonpresent")
    hashes = {c.hash for d in git_history.build_history(roster, _DAYS) for c in d.commits}

    matched = [r for r in runs if r["head_sha"] in hashes]
    assert matched, "no run joins a seeded commit — the coverage panel would read zero"
    assert len(matched) < len(runs), "every run joins — the coverage gap is invisible"
    assert all(r["head_sha"] not in hashes for r in runs if r["event"] == "pull_request")


def test_the_fixture_history_extends_past_the_ci_window() -> None:
    """Guards the clamp test below: unless the seeded history reaches past
    `CI_WINDOW_DAYS`, `in_window` has nothing to reject and that test would
    pass even if the clamp were removed entirely."""
    roster = profiles.build_roster("dev@company.nonpresent")
    history = git_history.build_history(roster, _DAYS)
    earliest = min(d.date for d in history)
    assert earliest < _ANCHOR - _dt.timedelta(days=ci.CI_WINDOW_DAYS)


def test_the_ci_window_is_clamped_to_the_vendor_retention(runs: Runs) -> None:
    oldest = min(_dt.datetime.fromisoformat(r["run_started_at"]).date() for r in runs)
    assert oldest >= _ANCHOR - _dt.timedelta(days=ci.CI_WINDOW_DAYS)


def test_run_start_hours_spread_across_the_day(runs: Runs) -> None:
    hours = {_dt.datetime.fromisoformat(r["run_started_at"]).hour for r in runs}
    assert len(hours) >= 8, f"runs cluster into {len(hours)} hours; the hour panels need spread"


def test_deployments_cover_production_preview_pending_and_superseded(rows: Rows) -> None:
    deployments = rows["bronze_github.deployments"]
    statuses = rows["bronze_github.deployment_statuses"]

    environments = {d["environment"] for d in deployments}
    assert "production" in environments
    assert any(e.startswith("preview-") for e in environments)
    assert any(d["is_transient_environment"] for d in deployments)
    assert any(d["is_production_environment"] for d in deployments)

    with_status = {s["deployment_id"] for s in statuses}
    assert [d for d in deployments if d["id"] not in with_status], "no pending deployment"
    assert any(s["state"] == "inactive" for s in statuses), "no superseded deployment"
    assert {"success", "failure", "error"} <= {s["state"] for s in statuses}
