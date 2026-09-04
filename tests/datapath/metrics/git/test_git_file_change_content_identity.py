"""Line metrics count a change once per content, not once per commit that carries it.

The same content reaches a repository twice when a branch keeps its own copy of a
tree that also landed on the default branch, or a commit is cherry-picked: two
commits, one (pre_image_oid, post_image_oid) pair, one authored change. The earliest
commit keeps the lines, the later repeat reports none and a commit size of zero, and
rows with no oid have unknown identity so both survive. The fixture isolates itself
in its own repository so other git fixtures cannot disturb the breakdown assertions.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_file_change_content_identity"

ERIN = "erin@example.com"
REPOSITORY = {"key": "repository", "value": "git-test:constructor/content-identity"}


def test_a_change_counts_once_per_content_however_many_commits_carry_it(spec: SpecRun) -> None:
    """October adds 100 (the earliest add) + 7 + 8 (one modification, not two) + 5 + 5 lines
    and removes 3 + 1 + 1; commit counting is unaffected, so six commits land."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.lines_added",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                    {
                        "metric_key": "git.lines_removed",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                    {
                        "metric_key": "git.commits",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.lines_added", "breakdown", entity_id=ERIN, dimensions=REPOSITORY).equals(value=125)
    r.row("git.lines_removed", "breakdown", entity_id=ERIN, dimensions=REPOSITORY).equals(value=5)
    r.row("git.commits", "breakdown", entity_id=ERIN, dimensions=REPOSITORY).equals(value=6)


def test_the_repeat_of_an_earlier_content_reports_no_lines_of_its_own(spec: SpecRun) -> None:
    """November holds one commit whose only file change repeats content October already
    counted: the commit counts, the repository gets no lines row, and the commit's size is 0."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-11-30"},
                "metrics": [
                    {
                        "metric_key": "git.lines_added",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                    {
                        "metric_key": "git.commits",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                    {"metric_key": "git.commit_size", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "breakdown", entity_id=ERIN, dimensions=REPOSITORY).equals(value=1)
    assert some(r.breakdown("git.lines_added"), dimensions=REPOSITORY) == []
    r.row("git.commit_size", "period", entity_id=ERIN).equals(value=0)
