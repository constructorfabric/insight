"""Branch scope of a commit whose hash sits in two connected repositories (#3354).

A fork and its upstream hold the same commit, each copy with its own default-branch
flag. The scope served belongs to the commit and it lands if any copy did: by the flag
on whichever copy sits on its repository's default branch, or by a merged request into
a default branch that lists the hash, even when the only collected copy is the fork's.
The copy whose own repository shows the landing survives the collapse and names the
repository. Before, the fork's owner sorted first and its copy answered for the hash,
and a request filed under the upstream was matched by repository coordinates and missed.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import MetricResponse, Row
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_branch_scope_shared_hash"

DAVE = "dave@example.com"
BOB = "bob@example.com"
ERIN = "erin@example.com"

FORK = "git-test:alpha-fork/payments"
UPSTREAM = "git-test:constructor/payments"

BY_REPOSITORY = {"view": "breakdown", "dimensions": ["repository"]}


def _scope_request(person: str) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [person]},
            "period": {"from": "2026-10-01", "to": "2026-10-02"},
            "metrics": [
                {
                    "metric_key": "git.default_branch_commits",
                    "views": [{"view": "period"}, BY_REPOSITORY],
                },
                {
                    "metric_key": "git.non_default_branch_commits",
                    "views": [{"view": "period"}, BY_REPOSITORY],
                },
                {
                    "metric_key": "git.default_branch_code_lines",
                    "views": [{"view": "period"}, BY_REPOSITORY],
                },
                {"metric_key": "git.non_default_branch_code_lines", "views": [{"view": "period"}]},
                {"metric_key": "git.default_branch_lines_removed", "views": [{"view": "period"}]},
                {"metric_key": "git.commits", "views": [{"view": "period"}]},
            ],
        },
    }


def _by_repository(r: MetricResponse, metric_key: str, person: str, repository: str) -> Row:
    return r.row(
        metric_key,
        "breakdown",
        entity_id=person,
        dimensions={"key": "repository", "value": repository},
    )


def test_a_commit_lands_when_any_of_its_repository_copies_is_on_a_default_branch(
    spec: SpecRun,
) -> None:
    """Dave's shared hash is flagged only in the upstream, which sorts after the fork: it
    lands with its lines under the upstream, and still counts once."""
    r = spec.call(_scope_request(DAVE))
    assert r.status == 200

    r.row("git.default_branch_commits", "period", entity_id=DAVE).equals(value=1)
    r.row("git.non_default_branch_commits", "period", entity_id=DAVE).equals(value=1)
    r.row("git.commits", "period", entity_id=DAVE).equals(value=2)

    r.row("git.default_branch_code_lines", "period", entity_id=DAVE).equals(value=12)
    r.row("git.default_branch_lines_removed", "period", entity_id=DAVE).equals(value=3)
    r.row("git.non_default_branch_code_lines", "period", entity_id=DAVE).equals(value=5)

    _by_repository(r, "git.default_branch_commits", DAVE, UPSTREAM).equals(value=1)
    _by_repository(r, "git.default_branch_code_lines", DAVE, UPSTREAM).equals(value=12)
    _by_repository(r, "git.non_default_branch_commits", DAVE, FORK).equals(value=1)


def test_a_merged_request_upstream_lands_a_hash_no_copy_flags_and_names_its_repository(
    spec: SpecRun,
) -> None:
    """Neither copy of bob's hash is flagged; the upstream's merged request lists it, so it
    lands, and the upstream copy survives because that is where the request lives."""
    r = spec.call(_scope_request(BOB))
    assert r.status == 200

    r.row("git.default_branch_commits", "period", entity_id=BOB).equals(value=1)
    r.row("git.non_default_branch_commits", "period", entity_id=BOB).equals(value=None)
    r.row("git.commits", "period", entity_id=BOB).equals(value=1)

    r.row("git.default_branch_code_lines", "period", entity_id=BOB).equals(value=9)
    r.row("git.default_branch_lines_removed", "period", entity_id=BOB).equals(value=2)
    r.row("git.non_default_branch_code_lines", "period", entity_id=BOB).equals(value=None)

    _by_repository(r, "git.default_branch_commits", BOB, UPSTREAM).equals(value=1)
    _by_repository(r, "git.default_branch_code_lines", BOB, UPSTREAM).equals(value=9)


def test_a_request_merged_upstream_lands_a_commit_collected_only_in_the_fork_once(
    spec: SpecRun,
) -> None:
    """Erin's commit was collected in the fork alone; the upstream's merged request into
    main lists its hash, so it lands, filed under the fork where it was collected. The
    squash that request produced is derived, so the work counts once; her unlisted
    commit stays in flight."""
    r = spec.call(_scope_request(ERIN))
    assert r.status == 200

    r.row("git.default_branch_commits", "period", entity_id=ERIN).equals(value=1)
    r.row("git.non_default_branch_commits", "period", entity_id=ERIN).equals(value=1)
    r.row("git.commits", "period", entity_id=ERIN).equals(value=2)

    r.row("git.default_branch_code_lines", "period", entity_id=ERIN).equals(value=7)
    r.row("git.non_default_branch_code_lines", "period", entity_id=ERIN).equals(value=3)
    r.row("git.default_branch_lines_removed", "period", entity_id=ERIN).equals(value=0)

    _by_repository(r, "git.default_branch_commits", ERIN, FORK).equals(value=1)
    _by_repository(r, "git.default_branch_code_lines", ERIN, FORK).equals(value=7)
    _by_repository(r, "git.non_default_branch_commits", ERIN, FORK).equals(value=1)
