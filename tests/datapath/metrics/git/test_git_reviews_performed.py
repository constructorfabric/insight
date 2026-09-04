"""Reviews performed per reviewer, dated by submission.

Bronze: GitHub pull-request reviews carry only the reviewer's numeric account id;
commit-author rows supply the (account, e-mail) pairs that id resolves through.
Silver: `class_git_pr_review_events`, one review event per verdict stamped with the
resolved actor e-mail. Gold attributes one review to the reviewer per event, dated by
submission; a verdict from an account no e-mail pair names emits nothing.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_reviews_performed"

ALICE = "alice@example.com"
CAROL = "carol@example.com"


def test_reviews_performed_count_per_reviewer_dated_by_submission(spec: SpecRun) -> None:
    """carol's two verdicts land on different days, so the timeseries proves dating by
    submission; the account-999 verdict maps to no e-mail and joins no pool: n stays 5."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.reviews_performed",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.reviews_performed", "period", entity_id=CAROL).equals(value=2)
    r.row("git.reviews_performed", "peer", entity_id=CAROL).equals(
        target_value=2, p25=1, median=1, p75=1, min=1, max=2, n=5
    )
    r.row("git.reviews_performed", "timeseries", entity_id=CAROL).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    r.row("git.reviews_performed", "timeseries", entity_id=CAROL).contains(
        points={"bucket_start": "2026-10-02", "value": 1}
    )
    r.row(
        "git.reviews_performed",
        "breakdown",
        entity_id=CAROL,
        dimensions={"key": "source", "value": "github"},
    ).equals(value=2)


def test_authoring_pull_requests_is_not_reviewing_them(spec: SpecRun) -> None:
    """alice authors both pull requests and reviews nothing: an honest null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [{"metric_key": "git.reviews_performed", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.reviews_performed", "period", entity_id=ALICE).equals(value=None)


def test_empty_window(spec: SpecRun) -> None:
    """A window before any verdict serves null, not zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "git.reviews_performed", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.reviews_performed", "period", entity_id=CAROL).equals(value=None)
