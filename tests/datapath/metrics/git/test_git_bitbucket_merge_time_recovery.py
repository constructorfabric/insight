"""A Bitbucket merge recorded without an activity entry still has a close time.

Bitbucket reports no close time on a pull request; the terminal entry in its activity is
what supplies one. A merge reached by pushing a commit that carries the request's head
changes the state with no merge action, so no such entry exists — and a merged request
with no close time reaches no period at all, because every measure that dates by the
close has nowhere to file it.

The merge commit the request names is the corroboration that the merge happened. Which
timestamp then dates it depends on what the activity can account for: an update nothing
explains is the silent state change itself, while an update a comment explains leaves the
commit as the closer record.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_bitbucket_merge_time_recovery"

HEIDI = "heidi@example.com"
DAVE = "dave@example.com"
WINDOW = {"from": "2026-10-01", "to": "2026-10-31"}

# One day per request, and each comes from a different piece of evidence: the terminal
# entry, the unexplained update, the merge commit, the update after the commit was ruled
# out, and a commit reporting only an author date.
LANDED = (
    "2026-10-05",
    "2026-10-13",
    "2026-10-17",
    "2026-10-21",
    "2026-10-25",
    "2026-10-30",
)

# Every day some losing candidate would have produced. Each is a real timestamp in the
# fixture, so a rule reading the wrong evidence fills one of these rather than merely
# shifting a total.
LOSING = (
    "2026-10-09",  # 401's merge commit, which loses to the terminal entry
    "2026-10-12",  # 402's merge commit, which loses to the unexplained update
    "2026-10-15",  # 403's commit's AUTHOR date, which loses to its committer date
    "2026-10-16",  # 410's merge commit, which loses to that request's update
    "2026-10-18",  # 408's own day: its prefix exists only in another repository
    "2026-10-19",  # 403's update, explained by a comment and so not the close
    "2026-10-20",  # 404's merge commit, older than the request that carried it
    "2026-10-26",  # 405's own days: its merge commit was never collected
    "2026-10-27",
    "2026-10-28",  # 406's own days, and the two commits its prefix names
    "2026-10-29",
    "2026-10-22",  # 409's merge commit, older than the request it belongs to
    "2026-10-23",  # 409's update, also older than its creation
    "2026-10-24",  # 409's own creation day: nothing may be invented from it
    "2026-10-31",  # 407's update, explained by a comment
)


def test_each_merge_lands_on_the_day_its_own_evidence_names(spec: SpecRun) -> None:
    """Five of heidi's eight requests reach a period, on five different days. Three do not:
    one names a merge commit that was never collected, one a prefix that names two commits
    in its repository, and one a prefix that exists only in another repository.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": WINDOW,
                "metrics": [
                    {
                        "metric_key": "git.prs_merged",
                        "views": [{"view": "period"}, {"view": "timeseries", "bucket": "day"}],
                    },
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.prs_created", "period", entity_id=HEIDI).equals(value=9)
    r.row("git.prs_merged", "period", entity_id=HEIDI).equals(value=6)

    series = r.row("git.prs_merged", "timeseries", entity_id=HEIDI)
    for day in LANDED:
        series.contains(points={"bucket_start": day, "value": 1})

    filled = [
        point["bucket_start"]
        for point in series.fields["points"]
        if point["value"] and point["bucket_start"] in LOSING
    ]
    assert not filled, f"a merge was filed under evidence that should have lost: {filled!r}"


def test_a_merge_with_no_usable_evidence_is_dropped_not_dated_at_the_epoch(
    spec: SpecRun,
) -> None:
    """The two requests that resolve nothing must reach NO period, not the beginning of
    time. This is the failure the October window cannot see: a close time that resolves to
    a type default instead of to nothing keeps October's total right and files the merge in
    1970. A period may not exceed 400 days, so the epoch needs a window of its own.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": {"from": "1970-01-01", "to": "1970-12-31"},
                "metrics": [{"metric_key": "git.prs_merged", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    r.row("git.prs_merged", "period", entity_id=HEIDI).equals(value=None)


def test_a_recovered_close_time_is_usable_and_not_merely_countable(spec: SpecRun) -> None:
    """A recovered close time has to survive the guards every duration measure applies —
    a close before the opening yields no value at all — so the cycle time is what proves
    the recovery produced a coherent interval and not just a countable row.

    Of the five requests that landed, the four the recovery dated span 77, 26, 77 and 2
    hours from opening to close; the one the terminal entry dated spans 97. The median of
    those five is 77 — a value only reachable if the recovered closes are both present and
    ordered after their openings.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": WINDOW,
                "metrics": [{"metric_key": "git.pr_cycle_time_h", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    r.row("git.pr_cycle_time_h", "period", entity_id=HEIDI).equals(value=77)


def test_a_declined_request_carrying_a_merge_hash_gains_no_close_time(spec: SpecRun) -> None:
    """dave's two requests were both declined, but only one closed with an entry. The other
    carries a merge hash whose commit WAS collected, and the recovery must not reach it: a
    close time there would make a request that merely went stale look abandoned on a date.

    The abandonment rate is what discriminates — both count as created, one as abandoned,
    so the rate is half. Were the merge hash to supply a close time it would read whole.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE]},
                "period": WINDOW,
                "metrics": [
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                    {"metric_key": "git.pr_abandonment_rate", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.prs_created", "period", entity_id=DAVE).equals(value=2)
    r.row("git.pr_abandonment_rate", "period", entity_id=DAVE).equals(value=50.0)
