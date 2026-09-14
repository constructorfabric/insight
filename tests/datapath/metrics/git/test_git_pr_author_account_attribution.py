"""A pull request attributed to its author, never to whoever wrote its commits.

Bronze: the pull requests a git connector reports, the account id it names for each
author, and the commits each request links to. Silver: one class row per request,
carrying the author's address when the source publishes one and the author's account
id when it names one. Gold reaches a person through the author's account first and the
author's own address second — and through nothing else, so a request whose account is
bound to nobody stays unresolved instead of inheriting the address its commits carry.

Four requests, the same pair on each of two connectors: an author account bound to a
person, and one bound to nobody. Neither publishes an author address here — on
Bitbucket that is structural, since it has none to publish. Every request carries
commits written by the same third person, who opened none of them.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_pr_author_account_attribution"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"


def test_a_bound_author_account_takes_its_request_and_an_unbound_one_takes_nobody(
    spec: SpecRun,
) -> None:
    """Alice takes the GitHub request through account 9101 and carol the Bitbucket one
    through bb-carol, each for the creation date and the merge date. On Bitbucket the
    binding is doing all of the work, because the connector publishes no author address
    for anything else to carry. Bob wrote the commits on all four requests and opened
    none: the unbound accounts leave their requests unresolved rather than turning his
    commits into a claim on his behalf."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, BOB, CAROL]},
                "period": {"from": "2026-11-01", "to": "2026-11-30"},
                "metrics": [
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_merged", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.prs_created", "period", entity_id=ALICE).equals(value=1)
    r.row("git.prs_merged", "period", entity_id=ALICE).equals(value=1)
    r.row("git.prs_created", "period", entity_id=CAROL).equals(value=1)
    r.row("git.prs_merged", "period", entity_id=CAROL).equals(value=1)
    r.row("git.prs_created", "period", entity_id=BOB).equals(value=None)
    r.row("git.prs_merged", "period", entity_id=BOB).equals(value=None)
