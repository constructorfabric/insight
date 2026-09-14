"""PR comments per person, with the comment_target split into own and others.

Bronze: GitHub conversation comments and inline review comments, with the
commenter accounts resolved through `commit_authors`. Silver:
`class_git_pr_review_events`, one 'comment' event per row stamped with the actor's
e-mail. Gold attributes each comment to the actor and compares them with the
request's author: 'own' when they match, 'others' otherwise or when the author is
unknown, so own + others = total always holds.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_pr_comments"

ALICE = "alice@example.com"
DAVE = "dave@example.com"


def test_pr_comments_split_into_own_and_others_and_the_halves_add_up(spec: SpecRun) -> None:
    """alice comments twice on her own #1 and once on carol's #2: total 3 = own 2 + others 1;
    the five one-comment peers give median 1, range [1, 3], n 5."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.pr_comments",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["comment_target"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.pr_comments", "period", entity_id=ALICE).equals(value=3)
    r.row("git.pr_comments", "peer", entity_id=ALICE).equals(
        target_value=3, p25=1, median=1, p75=1, min=1, max=3, n=5
    )
    r.row("git.pr_comments", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 3}
    )
    r.row(
        "git.pr_comments",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "comment_target", "value": "own"},
    ).equals(value=2)
    r.row(
        "git.pr_comments",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "comment_target", "value": "others"},
    ).equals(value=1)


def test_every_comment_came_from_the_one_connected_forge(spec: SpecRun) -> None:
    """The source split is its own case: a request carries at most one view of a kind."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.pr_comments",
                        "views": [{"view": "breakdown", "dimensions": ["source"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200
    r.row(
        "git.pr_comments",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "source", "value": "github"},
    ).equals(value=3)


def test_comment_on_a_request_whose_author_is_unknown_counts_under_others(spec: SpecRun) -> None:
    """dave's comment on #99, whose author has no e-mail anywhere, reads others;
    own + others = total even here."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.pr_comments",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["comment_target"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_comments", "period", entity_id=DAVE).equals(value=1)
    r.row(
        "git.pr_comments",
        "breakdown",
        entity_id=DAVE,
        dimensions={"key": "comment_target", "value": "others"},
    ).equals(value=1)


def test_empty_window(spec: SpecRun) -> None:
    """A window with no comments serves null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "git.pr_comments", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_comments", "period", entity_id=ALICE).equals(value=None)
