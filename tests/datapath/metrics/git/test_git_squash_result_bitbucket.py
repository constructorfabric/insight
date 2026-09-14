"""A merged request's result commit, reported under a shortened hash, still folds away.

Bronze: Bitbucket Cloud commits, file changes and pull requests, where a request's
`merge_commit_sha` is the 12-character prefix the vendor API answers with while the
commit rows carry the full 40. Silver resolves that prefix against the commits
collected for the request's own repository; gold then leaves the resolved result out of
the authored commit and line figures. squash-bb's two branch commits count once for 2
commits and 10 + 8 = 18 added lines, all of it `.py` under `src/` and so all of it code,
while the result commit adds neither a third commit nor its recombined 18. Resolution
stays inside one repository, so other-bb's commit sharing the same 12 characters keeps
its 1; a fast-forward's reported result is the work itself, so ff-bb keeps its 1; and a
prefix fitting two commits names neither, so ambig-bb keeps all 3.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import MetricResponse, Row
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_squash_result_bitbucket"

ERIN = "erin@example.com"

SQUASH = "bitbucket-test:acme/squash-bb"
OTHER_REPOSITORY = "bitbucket-test:acme/other-bb"
FAST_FORWARD = "bitbucket-test:acme/ff-bb"
AMBIGUOUS = "bitbucket-test:acme/ambig-bb"


def by_repository(r: MetricResponse, metric: str, repository: str) -> Row:
    return r.row(
        metric, "breakdown", entity_id=ERIN, dimensions={"key": "repository", "value": repository}
    )


def test_a_result_commit_reported_by_prefix_carries_no_commit_of_its_own(spec: SpecRun) -> None:
    """squash-bb reads 2 commits and 18 added lines, all code: the branch pair counts and
    the squash result under the 12-character hash does not. Resolving that prefix stays
    inside the request's repository, so other-bb's look-alike commit still counts 1; it
    does not turn a fast-forward's own commit into a copy, so ff-bb still counts 1; and a
    prefix two commits answer to marks neither, so ambig-bb still counts 3."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.commits",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                    {
                        "metric_key": "git.lines_added",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                    {
                        "metric_key": "git.code_lines",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    by_repository(r, "git.commits", SQUASH).equals(value=2)
    by_repository(r, "git.lines_added", SQUASH).equals(value=18)
    by_repository(r, "git.code_lines", SQUASH).equals(value=18)

    by_repository(r, "git.commits", OTHER_REPOSITORY).equals(value=1)
    by_repository(r, "git.commits", FAST_FORWARD).equals(value=1)
    by_repository(r, "git.commits", AMBIGUOUS).equals(value=3)
