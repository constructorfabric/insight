"""Typical diff size per pull request, served per person over a window.

Lines added plus lines removed, one value per request, dated by the day the request
was OPENED and taken whatever state the request reached. A request whose line counts
were never collected contributes nothing; a request whose counts were collected and
are zero contributes a real zero. Bitbucket reports one row per changed file, and only
the parent request's last-update stamp says which rows belong to the diff it has now.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_pr_size"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
HEIDI = "heidi@example.com"

#: The whole window every one of the six department members has a value in.
COHORT_PERIOD = {"from": "2026-12-01", "to": "2026-12-13"}

SOURCE_GITHUB = {"key": "source", "value": "github"}


def test_the_median_takes_every_request_whatever_it_became(spec: SpecRun) -> None:
    """[12, 24 open, 36 closed-unmerged, 48, 100] medians to 36, not the mean 44.

    The 2026-12-01 bucket holds the even set [12, 24, 36, 48] and serves 30, both
    middle values averaged; answering with the upper middle alone would read 36.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-12-01", "to": "2026-12-02"},
                "metrics": [
                    {
                        "metric_key": "git.pr_size",
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

    r.row("git.pr_size", "period", entity_id=ALICE).equals(value=36)
    series = r.row("git.pr_size", "timeseries", entity_id=ALICE)
    series.contains(points={"bucket_start": "2026-12-01", "value": 30})
    series.contains(points={"bucket_start": "2026-12-02", "value": 100})
    r.row("git.pr_size", "breakdown", entity_id=ALICE, dimensions=SOURCE_GITHUB).equals(value=36)
    r.row("git.pr_size", "histogram", entity_id=ALICE).nonempty("bins")


def test_a_collected_zero_is_a_value_not_an_absence(spec: SpecRun) -> None:
    """carol's [0, 10, 30] medians to 10; dropping the observed zero would serve 30.

    Her zero-line request had its counts collected — the source answered, and the
    answer was that no lines changed. That is an observation, and the class columns
    are nullable so that a request whose counts were never collected can say so
    instead.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-12-01", "to": "2026-12-01"},
                "metrics": [{"metric_key": "git.pr_size", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_size", "period", entity_id=CAROL).equals(value=10)


def test_gitlab_serves_the_diff_summary_its_own_stream_reported(spec: SpecRun) -> None:
    """602's summary was computed, so 18 + 6 reaches the metric as 24 on its open day."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-12-01", "to": "2026-12-01"},
                "metrics": [
                    {"metric_key": "git.pr_size", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_size", "period", entity_id=BOB).equals(value=24)
    r.row("git.prs_created", "period", entity_id=BOB).equals(value=1)


def test_gitlab_reports_the_request_whose_summary_is_still_pending_but_no_size(
    spec: SpecRun,
) -> None:
    """601 has no diff-stats row yet, so its day carries a request and no size.

    The created count is asserted beside the null: without it, a null size would
    equally describe a request that never reached the metric at all. The two days
    together then median to 24 rather than the 12 a pending summary read as a zero
    would give.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-12-02", "to": "2026-12-02"},
                "metrics": [
                    {"metric_key": "git.pr_size", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_size", "period", entity_id=BOB).equals(value=None)
    r.row("git.prs_created", "period", entity_id=BOB).equals(value=1)

    both = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-12-01", "to": "2026-12-02"},
                "metrics": [
                    {"metric_key": "git.pr_size", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert both.status == 200
    both.row("git.pr_size", "period", entity_id=BOB).equals(value=24)
    both.row("git.prs_created", "period", entity_id=BOB).equals(value=2)


def test_bitbucket_takes_the_current_file_rows_and_not_a_stale_one(spec: SpecRun) -> None:
    """801 sums its three current files to 40; 802's diff is 30 lines, not 100.

    802 holds a 70-line row for a file that left the diff in a rebase, written under
    an earlier update stamp. Size is the newest stamp's rows taken whole — resolving
    each file to its own newest row would keep the dropped file, which has no newer
    row to displace it. The period medians [30, 40] to 35; summing the stale row
    would reach 70.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": {"from": "2026-12-11", "to": "2026-12-13"},
                "metrics": [
                    {
                        "metric_key": "git.pr_size",
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

    series = r.row("git.pr_size", "timeseries", entity_id=HEIDI)
    series.contains(points={"bucket_start": "2026-12-11", "value": 40})
    series.contains(points={"bucket_start": "2026-12-12", "value": 30})
    r.row("git.pr_size", "period", entity_id=HEIDI).equals(value=35)


def test_a_bitbucket_request_with_no_diffstat_contributes_nothing(spec: SpecRun) -> None:
    """803's size was never collected, so 2026-12-13 carries no size — but a request.

    Bitbucket names the author by account id alone, so 803 reaches the metric through the
    binding the connector states for a workspace participant (#3423), and its created
    count proves it arrived. The absent size is therefore the missing diffstat and
    nothing else.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": {"from": "2026-12-13", "to": "2026-12-13"},
                "metrics": [
                    {"metric_key": "git.pr_size", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_size", "period", entity_id=HEIDI).equals(value=None)
    r.row("git.prs_created", "period", entity_id=HEIDI).equals(value=1)


def test_the_peer_median_over_an_even_cohort_averages_both_middles(spec: SpecRun) -> None:
    """Six members sized [10, 24, 35, 36, 50, 60] disclose 35.5, not the middle 36.

    The peer median answers the same question as the period one, so it reads the same
    textbook definition; p25 and p75 stay order statistics over the cohort.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": COHORT_PERIOD,
                "metrics": [
                    {
                        "metric_key": "git.pr_size",
                        "views": [{"view": "period"}, {"view": "peer"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    # p25/p75 follow ClickHouse quantilesExactIf element selection over the
    # 6-member pool; confirmed against a live run.
    r.row("git.pr_size", "period", entity_id=ALICE).equals(value=36)
    r.row("git.pr_size", "peer", entity_id=ALICE).equals(
        target_value=36, p25=24, median=35.5, p75=50, min=10, max=60, n=6
    )


def test_the_window_holding_every_close_holds_no_creation(spec: SpecRun) -> None:
    """Every one of alice's requests closes or merges inside 12-04…12-07, and the
    window serves nothing: the value sits on the day the request was opened."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-12-04", "to": "2026-12-07"},
                "metrics": [{"metric_key": "git.pr_size", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_size", "period", entity_id=ALICE).equals(value=None)


def test_empty_window(spec: SpecRun) -> None:
    """A window with no requests serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "git.pr_size", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_size", "period", entity_id=ALICE).equals(value=None)
