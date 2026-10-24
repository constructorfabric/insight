"""Another tenant sharing every identifier never reaches this tenant's reads.

The neighbour holds the same addresses, Jira accounts and issue keys, so nothing but
the tenant tells the two apart. Resolution binds each tenant's accounts to its own
people, the peer pool gathers one tenant's department, the evidence lists one
tenant's issues, and a person of the neighbour is not readable at all.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import NEIGHBOUR_TENANT, SpecRun
from insight_datapath.subjects import Subjects

pytestmark = pytest.mark.fixture

SPEC = "tasks_closed_beside_a_neighbour"
NEIGHBOUR = "../tasks/tasks_closed"

CAROL = "carol@example.com"
ERIN = "erin@example.com"
PERIOD = {"from": "2026-12-20", "to": "2026-12-31"}


def _closed(person_ids: list[str], views: list[dict[str, str]]) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": person_ids},
            "period": PERIOD,
            "metrics": [{"metric_key": "tasks.closed", "views": views}],
        },
    }


def test_a_shared_account_counts_only_this_tenants_closures(spec: SpecRun) -> None:
    """Carol closes one issue here and three in the neighbour under the same Jira
    account: her count is 1, and her department of three discloses nothing."""
    r = spec.call(_closed([CAROL], [{"view": "period"}, {"view": "peer"}]))
    assert r.status == 200, r.payload

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=1)
    r.row("tasks.closed", "peer", entity_id=CAROL).equals(
        target_value=1, p25=None, median=None, p75=None, min=None, max=None, n=3
    )


def test_the_evidence_lists_only_this_tenants_issues(spec: SpecRun) -> None:
    """Both tenants hold TCW1 under one key; carol's evidence names it once, and the
    neighbour's TCW2 and TCW3 not at all."""
    r = spec.call(
        {
            "url": "/v1/metric-drilldown",
            "method": "POST",
            "body": {
                "metric_key": "tasks.closed",
                "entity": {"type": "person", "id": CAROL},
                "period": PERIOD,
            },
        }
    )
    assert r.status == 200, r.payload

    refs = sorted(str(row["values"].get("ref")) for row in r.payload["rows"])
    assert refs == ["TCW1"], f"evidence refs: {refs}"


@pytest.mark.parametrize("email", [CAROL, ERIN], ids=["shared address", "neighbour only"])
def test_a_person_of_the_neighbour_is_not_readable(
    spec: SpecRun, subjects: Subjects, email: str
) -> None:
    """The neighbour's own people answer to no reader of this tenant, whether or not an
    address of theirs also names someone here."""
    neighbour_ids = subjects.for_tenant(NEIGHBOUR_TENANT).person_ids([email])
    assert email in neighbour_ids, f"the neighbour minted no person for {email}"

    status, payload = spec.caller.call_request(
        _closed([neighbour_ids[email]], [{"view": "period"}])
    )

    assert status == 403, f"another tenant's person must be refused, got {status}: {payload!r}"
