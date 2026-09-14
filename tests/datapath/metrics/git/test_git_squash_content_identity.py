"""A squash re-applies its branch's whole span as one step, so its copies fold away.

Content identity is the object id a change produced: the branch makes A to B then B to C,
the squash makes A to C, and the squash's copies of work already collected suppress instead
of counting twice. Each case owns a month, so every period assertion is isolated without a
repository dimension.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_squash_content_identity"

ERIN = "erin@example.com"


def test_a_partly_collected_branchs_squash_counts_only_its_uncollected_paths(
    spec: SpecRun,
) -> None:
    """The squash stays a commit — the two commits it alone records are real work — but only
    its own new path counts: 10 plus 5 plus 7, not the 52 its copies would read."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-31"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["branch_scope"]},
                        ],
                    },
                    {"metric_key": "git.lines_removed", "views": [{"view": "period"}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "period", entity_id=ERIN).equals(value=3)
    r.row("git.lines_added", "period", entity_id=ERIN).equals(value=22)
    r.row("git.lines_removed", "period", entity_id=ERIN).equals(value=2)
    r.row("git.commit_size", "period", entity_id=ERIN).equals(value=7)

    r.row(
        "git.lines_added",
        "breakdown",
        entity_id=ERIN,
        dimensions={"key": "branch_scope", "value": "default"},
    ).equals(value=22)
    assert not some(
        r.breakdown("git.lines_added"),
        dimensions={"key": "branch_scope", "value": "non_default"},
    ), "the request merged into the default branch, so all of its work landed"


def test_a_squash_with_no_request_folds_into_the_branch_commit_it_ends_on(
    spec: SpecRun,
) -> None:
    """Nothing links this squash to its originals, so it still counts as a commit, but its
    span ends on content already produced: 8 plus 4, where a pair-keyed identity would read 20."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-11-30"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["branch_scope"]},
                        ],
                    },
                    {
                        "metric_key": "git.lines_removed",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["branch_scope"]},
                        ],
                    },
                    {"metric_key": "git.commit_size", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "period", entity_id=ERIN).equals(value=3)
    r.row("git.lines_added", "period", entity_id=ERIN).equals(value=12)
    r.row("git.lines_removed", "period", entity_id=ERIN).equals(value=3)
    r.row("git.commit_size", "period", entity_id=ERIN).equals(value=6)

    for scope, added, removed in (("default", 4, 2), ("non_default", 8, 1)):
        selector = {"key": "branch_scope", "value": scope}
        r.row("git.lines_added", "breakdown", entity_id=ERIN, dimensions=selector).equals(
            value=added
        )
        r.row("git.lines_removed", "breakdown", entity_id=ERIN, dimensions=selector).equals(
            value=removed
        )


def test_two_removals_of_two_contents_at_one_path_both_count(spec: SpecRun) -> None:
    """5 plus 6: a deletion keyed on its post-image alone would carry no identity, collapse
    both removals into one and read 5."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "period"}]},
                    {"metric_key": "git.lines_removed", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "period", entity_id=ERIN).equals(value=3)
    r.row("git.lines_added", "period", entity_id=ERIN).equals(value=6)
    r.row("git.lines_removed", "period", entity_id=ERIN).equals(value=11)
