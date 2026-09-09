"""The merge rate is a share of the period's OWN requests, not of the period's merges.

Both of its inputs ride the same pull-request row and are dated by the creation, so the
denominator is every request opened in the period and the numerator is the same rows that
have merged by now. Two consequences follow, and they are what this spec pins: a request
merged after the period closed still counts for the period it was opened in, and a request
opened earlier never counts, however plainly its merge lands inside.

Every window here is sized so that the other reading — the period's merges over its
creations — gives a different number, so no case can pass by coincidence.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_merge_rate_creation_window"

CAROL = "carol@example.com"
DAVE = "dave@example.com"
WINDOW = {"from": "2026-10-01", "to": "2026-10-31"}


def test_the_rate_counts_a_merge_that_happened_after_the_period_closed(spec: SpecRun) -> None:
    """Four of carol's six requests were opened in the period — one on each bound — and
    three of those have merged, two of them only in November. The rate is three quarters.

    The merged COUNT for the same period is two, and the two populations share exactly one
    member: the request opened and merged inside October. The other count member was opened
    in September, and the other two rate members merged in November. Asking the period's
    merges over its creations would read half.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": WINDOW,
                "metrics": [
                    {
                        "metric_key": "git.merge_rate",
                        "views": [{"view": "period"}, {"view": "timeseries", "bucket": "day"}],
                    },
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_merged", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.merge_rate", "period", entity_id=CAROL).equals(value=75)
    # The denominator. September's request is not among these, and neither is the one
    # opened the day after the upper bound — both bounds are inclusive and stop there.
    r.row("git.prs_created", "period", entity_id=CAROL).equals(value=4)
    # Dated by the close, so this counts September's request and neither November one.
    r.row("git.prs_merged", "period", entity_id=CAROL).equals(value=2)

    # Each creation day holds exactly one request, so each bucket reads whole or nothing —
    # and the bucket is the CREATION day even where the merge happened in November. A rule
    # that bucketed by the merge instead would empty every one of these.
    series = r.row("git.merge_rate", "timeseries", entity_id=CAROL)
    for day, rate in (
        ("2026-10-01", 100),  # merged in November
        ("2026-10-06", 0),  # still open
        ("2026-10-08", 100),  # merged the next day
        ("2026-10-31", 100),  # merged in November
    ):
        series.contains(points={"bucket_start": day, "value": rate})


def test_a_request_opened_before_the_period_never_enters_the_rate(spec: SpecRun) -> None:
    """September holds one creation, and that request merged in October. Asked about
    September the rate reads whole, because the question is what became of September's
    requests — not what merged in September, where the answer is nothing.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-09-01", "to": "2026-09-30"},
                "metrics": [
                    {"metric_key": "git.merge_rate", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_merged", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.merge_rate", "period", entity_id=CAROL).equals(value=100)
    r.row("git.prs_created", "period", entity_id=CAROL).equals(value=1)
    r.row("git.prs_merged", "period", entity_id=CAROL).equals(value=None)


def test_a_declined_request_and_an_open_one_both_stay_in_the_denominator(
    spec: SpecRun,
) -> None:
    """dave opened five on one day: two merged — one of them in November — two were closed
    without merging, one is still open. The rate is two fifths: an outcome other than a
    merge lowers it rather than leaving the population.

    The November merge is also what keeps this window from reading the same under either
    question — the period's merges over its creations would give one fifth.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE]},
                "period": WINDOW,
                "metrics": [
                    {"metric_key": "git.merge_rate", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.merge_rate", "period", entity_id=DAVE).equals(value=40)
    r.row("git.prs_created", "period", entity_id=DAVE).equals(value=5)


def test_no_merges_reads_zero_while_no_creations_reads_null(spec: SpecRun) -> None:
    """Two different emptinesses, asserted together because the distinction is the point.

    A single-day window holding only carol's open request has a population and nothing
    merged in it: the rate is zero. August holds no creation at all, so there is no
    population and the rate has no value. Collapsing either into the other — a null where
    a zero belongs, or a zero standing in for "nothing to measure" — is a reporting bug
    that no total would reveal.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-10-06", "to": "2026-10-06"},
                "metrics": [
                    {"metric_key": "git.merge_rate", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.merge_rate", "period", entity_id=CAROL).equals(value=0)
    r.row("git.prs_created", "period", entity_id=CAROL).equals(value=1)

    empty = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL, DAVE]},
                "period": {"from": "2026-08-01", "to": "2026-08-31"},
                "metrics": [{"metric_key": "git.merge_rate", "views": [{"view": "period"}]}],
            },
        }
    )
    assert empty.status == 200

    for person in (CAROL, DAVE):
        empty.row("git.merge_rate", "period", entity_id=person).equals(value=None)
