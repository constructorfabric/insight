"""Typical hours from opening a pull request to merging it, served per person.

One value per MERGED request, dated by the merge. A merged request is measured to
its MERGE and never to an earlier close the source reports beside it. On Bitbucket
the merge time is not on the request, so the close is taken from the terminal entry
in its activity where there is one and recovered from the request's own last update
otherwise — the first is a measurement, the second an approximation.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_pr_cycle_time"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"
ERIN = "erin@example.com"
HEIDI = "heidi@example.com"

SOURCE_GITHUB = {"key": "source", "value": "github"}


def test_the_median_cycle_sits_on_the_merge_day(spec: SpecRun) -> None:
    """[2, 6, 10, 30, 100] medians to 10, not the mean 29.6.

    All five open at the same instant on 2026-11-01, so every bucket boundary here is
    a merge: 2026-11-01 holds [2, 6, 10] and serves 6, and the 100 h request lands on
    2026-11-05 rather than the day it was opened.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-11-01", "to": "2026-11-05"},
                "metrics": [
                    {
                        "metric_key": "git.pr_cycle_time_h",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                            {"view": "histogram"},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.pr_cycle_time_h", "period", entity_id=ALICE).equals(value=10)
    series = r.row("git.pr_cycle_time_h", "timeseries", entity_id=ALICE)
    series.contains(points={"bucket_start": "2026-11-01", "value": 6})
    series.contains(points={"bucket_start": "2026-11-02", "value": 30})
    series.contains(points={"bucket_start": "2026-11-05", "value": 100})
    r.row("git.pr_cycle_time_h", "breakdown", entity_id=ALICE, dimensions=SOURCE_GITHUB).equals(
        value=10
    )
    r.row("git.pr_cycle_time_h", "histogram", entity_id=ALICE).nonempty("bins")


def test_a_github_merge_is_measured_to_the_merge_not_an_earlier_close(
    spec: SpecRun,
) -> None:
    """dave's request closed 2 h in and merged 10 h in, so its cycle is 10 h.

    The source reports both times. A cycle ends when the change lands, so the merge
    decides; taking the close would report an interval that ended while the request
    was still open.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE]},
                "period": {"from": "2026-11-10", "to": "2026-11-10"},
                "metrics": [{"metric_key": "git.pr_cycle_time_h", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_cycle_time_h", "period", entity_id=DAVE).equals(value=10)


def test_a_gitlab_merge_is_measured_to_the_merge_not_an_earlier_close(
    spec: SpecRun,
) -> None:
    """bob's requests run 8 h and 9 h; the second reports a close 1 h in and is not 1.

    GitLab reports a closing time on a request that was closed without merging, and
    a request closed, reopened and then merged keeps it. It is not the merge.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-11-15", "to": "2026-11-16"},
                "metrics": [
                    {
                        "metric_key": "git.pr_cycle_time_h",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    series = r.row("git.pr_cycle_time_h", "timeseries", entity_id=BOB)
    series.contains(points={"bucket_start": "2026-11-15", "value": 8})
    series.contains(points={"bucket_start": "2026-11-16", "value": 9})
    r.row("git.pr_cycle_time_h", "period", entity_id=BOB).equals(value=8.5)


def test_a_recovered_bitbucket_close_counts_the_merge_but_reports_no_cycle(
    spec: SpecRun,
) -> None:
    """901 runs 12 h from its terminal activity entry; 902 reports no cycle at all.

    902 was merged by pushing its head, so its activity holds no terminal entry and
    its close time is recovered from the request's own last update. The recovery
    settles which DAY the merge landed on, which is why 902 still counts as merged —
    but an interval measured to it would be one nobody observed, so the duration is
    absent rather than approximate. Both requests merged inside the window, and the
    merged count says so.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": {"from": "2026-11-25", "to": "2026-11-26"},
                "metrics": [
                    {
                        "metric_key": "git.pr_cycle_time_h",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    },
                    {"metric_key": "git.prs_merged", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    series = r.row("git.pr_cycle_time_h", "timeseries", entity_id=HEIDI)
    series.contains(points={"bucket_start": "2026-11-25", "value": 12})
    series.contains(points={"bucket_start": "2026-11-26", "value": None})
    r.row("git.pr_cycle_time_h", "period", entity_id=HEIDI).equals(value=12)
    r.row("git.prs_merged", "period", entity_id=HEIDI).equals(value=2)


def test_a_request_that_never_merged_contributes_nothing(spec: SpecRun) -> None:
    """carol's open request and her closed-unmerged one have no cycle between them.

    Her created count is asserted beside it: a null cycle must mean "these never
    merged", not "carol never opened anything".
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-11-18", "to": "2026-11-19"},
                "metrics": [
                    {"metric_key": "git.pr_cycle_time_h", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_cycle_time_h", "period", entity_id=CAROL).equals(value=None)
    r.row("git.prs_created", "period", entity_id=CAROL).equals(value=2)


def test_an_interval_that_runs_backwards_is_dropped(spec: SpecRun) -> None:
    """erin's merge is recorded before her opening, so there is no cycle at all.

    Not a negative duration and not a zero — the source contradicts itself and the
    honest answer is silence. Her created count proves the request itself arrived.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-11-30"},
                "metrics": [
                    {"metric_key": "git.pr_cycle_time_h", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_cycle_time_h", "period", entity_id=ERIN).equals(value=None)
    r.row("git.prs_created", "period", entity_id=ERIN).equals(value=1)


def test_empty_window(spec: SpecRun) -> None:
    """A window with no merged requests serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "git.pr_cycle_time_h", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_cycle_time_h", "period", entity_id=ALICE).equals(value=None)
