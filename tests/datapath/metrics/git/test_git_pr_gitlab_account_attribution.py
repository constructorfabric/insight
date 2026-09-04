"""GitLab merge requests attributed through the author's account, not only an address.

Bronze: merge requests naming their author's numeric account id, user records that
carry no address for it, and one merge-request commit somebody else wrote. Silver: one
class row per merge request, carrying both the author's address and the author's
account id. Gold resolves the author through the account binding first, and the address
map decides only when no account id exists. Account 700 is bound to bob, so both of his
requests reach him — the squash-merge whose commits were never collected and the one
carrying dave's commit alike — and every measure emitted from that gold row reads 2.
Account 701 is bound to nobody, so carol's request reaches nobody; dave wrote a commit
and opened nothing, so no request is his.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_pr_gitlab_account_attribution"

BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"

SOURCE_GITLAB = {"key": "source", "value": "gitlab"}


def test_a_merge_request_with_no_author_address_counts_through_the_account_binding(
    spec: SpecRun,
) -> None:
    """Bob merges 2. Account 700 is bound to him and that binding is the only thing
    naming him on request 201, whose user record carries no address and whose commits
    were never collected; on 202 it has to win against dave's commit address rather
    than merely fill a gap. Dave wrote that commit and opened nothing, so the merge is
    not his, and account 701 is bound to nobody, so carol's request reaches nobody —
    which is what makes these assertions about the binding."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB, CAROL, DAVE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.prs_merged",
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

    r.row("git.prs_merged", "period", entity_id=BOB).equals(value=2)
    r.row("git.prs_merged", "breakdown", entity_id=BOB, dimensions=SOURCE_GITLAB).equals(value=2)
    r.row("git.prs_merged", "period", entity_id=DAVE).equals(value=None)
    r.row("git.prs_merged", "period", entity_id=CAROL).equals(value=None)


def test_the_created_count_follows_the_same_binding_in_the_window_they_were_opened_in(
    spec: SpecRun,
) -> None:
    """Every pull-request measure rides the same gold row, so a request that reaches
    nobody is missing from the created count exactly as it is from the merged one. Both
    of bob's requests were opened on the 20th; dave opened nothing, and writing a commit
    a request carries does not make him its author any more than it does for the merge;
    carol's unbound account still reaches nobody."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB, CAROL, DAVE]},
                "period": {"from": "2026-09-20", "to": "2026-09-21"},
                "metrics": [
                    {
                        "metric_key": "git.prs_created",
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

    r.row("git.prs_created", "period", entity_id=BOB).equals(value=2)
    r.row("git.prs_created", "breakdown", entity_id=BOB, dimensions=SOURCE_GITLAB).equals(value=2)
    r.row("git.prs_created", "period", entity_id=DAVE).equals(value=None)
    r.row("git.prs_created", "period", entity_id=CAROL).equals(value=None)
