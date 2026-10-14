"""A person's commits are their own tenant's, though another tenant shares every identifier.

The neighbour holds the same address, source, repositories and commit sha. Resolution
binds the address to one person per tenant, and the commits a person reads are the
ones their own tenant reported.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_commits_beside_a_neighbour"
NEIGHBOUR = "../git/git_commits_per_active_day"

ERIN = "erin@example.com"
API = {"key": "repository", "value": "git-test:acme/api"}


def test_a_shared_address_reads_only_this_tenants_commits(spec: SpecRun) -> None:
    """One commit on one day here; the neighbour's six commits over two days and two
    repositories under the same address never join it."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-07"},
                "metrics": [
                    {
                        "metric_key": "git.commits_per_active_day",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["repository"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200, r.payload

    r.row("git.commits_per_active_day", "period", entity_id=ERIN).equals(value=1)
    repositories = r.breakdown("git.commits_per_active_day")
    assert len(repositories) == 1, f"repository groups: {repositories}"
    r.row("git.commits_per_active_day", "breakdown", entity_id=ERIN, dimensions=API).equals(value=1)
