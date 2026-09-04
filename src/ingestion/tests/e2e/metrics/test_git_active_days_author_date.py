"""A commit counts on the day its work was written, not the day it last took shape.

Rebase, amend and cherry-pick rewrite the committer date and keep the author date,
so a branch rebased before merging carries one committer date across every commit
on it. Alice writes on three days and rebases on the fourth; bob writes three
commits on one day and never rebases. Alice has three active days, bob has one, and
a cherry-pick copy sharing its original's patch id counts once, by the earlier carrier.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_active_days_author_date"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"


def test_a_rebased_branch_keeps_the_days_its_work_was_written_on(spec: SpecRun) -> None:
    """Three author dates under one committer date are three active days; bob's one day
    stays one, so a metric that merely counts commits cannot pass."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, BOB]},
                "period": {"from": "2026-10-01", "to": "2026-10-05"},
                "metrics": [
                    {"metric_key": "git.active_days", "views": [{"view": "period"}]},
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.active_days", "period", entity_id=ALICE).equals(value=3)
    r.row("git.active_days", "period", entity_id=BOB).equals(value=1)
    r.row("git.commits", "period", entity_id=ALICE).equals(value=3)
    r.row("git.commits", "period", entity_id=BOB).equals(value=3)


def test_a_window_before_the_rebase_holds_the_days_work_was_written_on(spec: SpecRun) -> None:
    """The window closes two days before alice's only committer date; dated by the author
    she has the 1st and the 2nd."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.active_days", "views": [{"view": "period"}]},
                    {"metric_key": "git.code_lines", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.active_days", "period", entity_id=ALICE).equals(value=2)
    r.row("git.code_lines", "period", entity_id=ALICE).equals(value=20)


def test_a_cherry_pick_copy_counts_once_and_the_earlier_carrier_counts(spec: SpecRun) -> None:
    """Two commits with one patch id and one author date are one commit and seven lines;
    the earlier carrier survives, so the lines read non_default."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-10-01", "to": "2026-10-05"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.code_lines",
                        "views": [{"view": "period"}, {"view": "breakdown", "dimensions": ["branch_scope"]}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "period", entity_id=CAROL).equals(value=1)
    r.row("git.code_lines", "period", entity_id=CAROL).equals(value=7)
    r.row(
        "git.code_lines", "breakdown", entity_id=CAROL, dimensions={"key": "branch_scope", "value": "non_default"}
    ).equals(value=7)
