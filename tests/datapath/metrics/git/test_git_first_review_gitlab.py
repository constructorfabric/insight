"""Time to first review (p75) on GitLab, where a review is a system note.

Bronze: merge requests with their `reviewers` array, and the notes GitLab wrote and
people wrote on them. Silver: `class_git_pull_requests_reviewers` holds one `requested`
row per reviewer asked (no instant) and one verdict row per system note whose body is an
approval, its withdrawal or a request for changes. Gold dates a request at its EARLIEST
verdict row that carries an instant.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_first_review_gitlab"

ALICE = "alice@example.com"

SOURCE_GITLAB = {"key": "source", "value": "gitlab"}


def test_only_approval_withdrawal_and_change_request_notes_are_reviews(spec: SpecRun) -> None:
    """A comment and a milestone note precede the approval on the first request; a request
    for changes is the first review on the second. Hours [10, 20] -> p75 index 1 -> 20. A
    comment counted as a review would give 5; the `requested` row counted would give 0."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.first_review_time_p75_h",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.first_review_time_p75_h", "period", entity_id=ALICE).equals(value=20)
    r.row(
        "git.first_review_time_p75_h", "breakdown", entity_id=ALICE, dimensions=SOURCE_GITLAB
    ).equals(value=20)
