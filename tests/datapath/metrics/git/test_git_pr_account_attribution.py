"""Merged pull requests attributed account-first, served per person over a window.

Bronze: pull requests plus the GraphQL diff-stats row carrying the author's numeric
account id and, rarely, a profile email. Silver: one row per pull request with
author_email and author_account_id. Gold resolves the author through the account
binding first; the email map decides only when no account id exists, so a squash-merge
with no collected commits and a ghost author both attribute, a pull request whose
account resolves to nobody attributes to nobody, and a conflicting profile email loses.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_pr_account_attribution"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"


def test_account_first_attribution_decides_pull_request_measures(spec: SpecRun) -> None:
    """Alice takes the squash-merge and the precedence pull request through account 9001.
    Carol gets nothing: she wrote the ghost request's commit and opened nothing, and an
    author the source names no account and no address for stays unresolved rather than
    reaching whoever wrote the commits. Dave gets nothing: his address is on the bot's
    commit, whose account resolves to no person, and an account that resolves to nobody
    ends resolution instead of falling through to the address. Bob gets nothing either:
    his address is claimed by the account an operator gave to alice, so it names no one
    person."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, BOB, CAROL, DAVE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [{"metric_key": "git.prs_merged", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    r.row("git.prs_merged", "period", entity_id=ALICE).equals(value=2)
    r.row("git.prs_merged", "period", entity_id=CAROL).equals(value=None)
    r.row("git.prs_merged", "period", entity_id=DAVE).equals(value=None)
    r.row("git.prs_merged", "period", entity_id=BOB).equals(value=None)


def test_the_bots_pull_request_is_charged_to_nobody_in_the_repository_rollup(
    spec: SpecRun,
) -> None:
    """The rollup an overview draws pools what people own. Four pull requests merged
    into the repository and every person who could hold one is named here, so a total
    of two is the excluded account's work and the ghost author's both resting on no
    one."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, BOB, CAROL]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.prs_merged",
                        "views": [{"view": "rollup", "dimensions": ["repository"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.prs_merged", "rollup").equals(value=2, contributing_entity_count=1)
