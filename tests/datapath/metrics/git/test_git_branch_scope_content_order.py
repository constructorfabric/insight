"""A commit lands on the default branch only if the trunk did not already hold its content.

The content heal answers "did this work land" where reachability cannot, because a squash
leaves its originals unreachable for good. It compares a change's object id at a path
against the default branch's own changes — and it has to ask whether the trunk already
held that content, because then the match says nothing about this commit. One change is
enough and branch scope lives on the commit, so a restored file used to carry the whole
commit across, incidental files and all. #3340
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_branch_scope_content_order"

HEIDI = "heidi@example.com"
PERIOD = {"from": "2026-10-01", "to": "2026-10-02"}

LANDED_BLOCKS = (
    ("00", 1),
    ("02", 2),
    ("04", 1),
    ("06", 1),
    ("08", 1),
    ("12", 1),
    ("18", 1),
    ("20", 2),
    ("22", 2),
)
UNLANDED_BLOCKS = (("10", 1), ("14", 1), ("16", 1))


def hour_block(value: str) -> dict[str, str]:
    return {"key": "hour_block", "value": value}


def test_a_commit_restoring_content_the_trunk_already_held_has_not_landed(
    spec: SpecRun,
) -> None:
    """Twelve of the fifteen commits land. The three that do not each fail the ordering
    a different way: one restores content the trunk took earlier, one repeats a state the
    trunk holds both before and after, and one matches content whose earliest carrier the
    source never dated. A sixteenth commit is that undated carrier, and it reaches no
    commit measure at all — a commit nobody dated is not an authored commit."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": PERIOD,
                "metrics": [
                    {"metric_key": "git.default_branch_commits", "views": [{"view": "period"}]},
                    {"metric_key": "git.non_default_branch_commits", "views": [{"view": "period"}]},
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_commits", "period", entity_id=HEIDI).equals(value=12)
    r.row("git.non_default_branch_commits", "period", entity_id=HEIDI).equals(value=3)
    r.row("git.commits", "period", entity_id=HEIDI).equals(value=15)


def test_only_the_commits_the_trunk_preceded_report_not_landing(spec: SpecRun) -> None:
    """The hour block names the commit, so a rule that moved the wrong one is not merely
    a different total: 10 matches content with an undated carrier, 14 repeats content the
    trunk already had, 16 restores it, and nothing else may appear."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": PERIOD,
                "metrics": [
                    {
                        "metric_key": "git.non_default_branch_commits",
                        "views": [{"view": "breakdown", "dimensions": ["hour_block"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    for block, commits in UNLANDED_BLOCKS:
        r.row(
            "git.non_default_branch_commits",
            "breakdown",
            entity_id=HEIDI,
            dimensions=hour_block(block),
        ).equals(value=commits)

    assert len(r.breakdown("git.non_default_branch_commits")) == len(UNLANDED_BLOCKS), (
        "those two commits are the only ones that did not land"
    )


def test_each_landing_rule_carries_its_own_commit(spec: SpecRun) -> None:
    """One block per commit that landed. 02 is the inclusive boundary, beside the trunk
    commit it ties with; 08 is the commit whose carrier was AUTHORED before it and
    WRITTEN after, which only the committer date answers for; 12 is the ordinary squash;
    18 is the commit one of whose two changes matches nothing. 22 holds two: the trunk
    commit that takes back a content it already had, and the dated carrier that would
    promote block 10 if the undated carrier beside it were skipped rather than obeyed."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": PERIOD,
                "metrics": [
                    {
                        "metric_key": "git.default_branch_commits",
                        "views": [{"view": "breakdown", "dimensions": ["hour_block"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    for block, commits in LANDED_BLOCKS:
        r.row(
            "git.default_branch_commits",
            "breakdown",
            entity_id=HEIDI,
            dimensions=hour_block(block),
        ).equals(value=commits)

    for block, _ in UNLANDED_BLOCKS:
        assert not some(r.breakdown("git.default_branch_commits"), dimensions=hour_block(block)), (
            f"the commit in block {block} did not land"
        )


def test_the_lines_of_a_commit_follow_the_side_the_commit_itself_is_on(
    spec: SpecRun,
) -> None:
    """The file that triggers a heal need not be the file whose lines move. src/extra.rs
    matches nothing on the trunk and its eleven lines still cross, because the commit
    beside it does; src/only.rs is the same shape on the losing side and its nine lines
    stay, as do the twenty-one of src/blind.rs. The restore's own copy of src/shared.rs
    loses the content dedup to the trunk commit that held it first, so it contributes
    nothing either way."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": PERIOD,
                "metrics": [
                    {"metric_key": "git.default_branch_code_lines", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.non_default_branch_code_lines",
                        "views": [{"view": "period"}],
                    },
                    {"metric_key": "git.code_lines", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_code_lines", "period", entity_id=HEIDI).equals(value=47)
    r.row("git.non_default_branch_code_lines", "period", entity_id=HEIDI).equals(value=30)
    r.row("git.code_lines", "period", entity_id=HEIDI).equals(value=77)
