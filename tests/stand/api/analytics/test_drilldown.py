"""`/v1/metric-drilldown` and its export — the rows behind a metric value.

    POST /v1/metric-drilldown         400 empty-entity
    POST /v1/metric-drilldown/export  400 empty-entity

The 415 half is in `test_request_contracts.py`, swept over every body route.

Error contracts only, deliberately. A 200 here needs a metric_key that resolves
against seeded observations AND an entity the caller may see AND a period those
rows fall in — three seeded facts that would make the test an assertion about
the seed rather than about the route. The rig covers the same two codes for the
same reason.

What the stand adds over the rig is that both requests cross a real gateway
carrying a real session, so a 400 here is the handler's own validation rather
than the edge's: an anonymous call to either url is 401 long before any body is
read (`test_gateway.py`), which is what makes these two codes attributable.

`/export` is the newer of the pair (#2074) and the only operation in this suite
that does not answer JSON — it serves CSV or XLSX. Its error contract is still
the shared one, which is exactly why the boilerplate 403 it declares is
subtracted in `coverage.py`: the handler has no gate to produce one.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, analytics_path
from insight_stand.api import JsonValue

DRILLDOWN = analytics_path("/v1/metric-drilldown")
DRILLDOWN_EXPORT = analytics_path("/v1/metric-drilldown/export")

#: Well-formed apart from the one field under test. An empty entity id is the
#: cheapest rejection that is unambiguously the HANDLER's — it needs no seeded
#: metric to reach, and no lookup can turn it into a 404 on the way.
_EMPTY_ENTITY_ID = ""


def _request() -> dict[str, JsonValue]:
    return {
        "metric_key": "tasks.closed",
        "entity": {"type": "person", "id": _EMPTY_ENTITY_ID},
        "period": {"from": "2026-01-01", "to": "2026-01-31"},
        "filters": [],
        "display_dimensions": [],
        "limit": 100,
    }


def test_drilldown_400_empty_entity_id(api: ApiClient) -> None:
    response = api.post(DRILLDOWN, json_body=_request())
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"


def test_drilldown_export_400_empty_entity_id(api: ApiClient) -> None:
    """Same rejection, and it must happen before any format negotiation.

    `format` is what makes this operation different; validating the entity
    first is what keeps it the same route. A 200 with an empty CSV would be the
    regression worth catching here.
    """
    request = _request()
    request["format"] = "csv"
    del request["limit"]

    response = api.post(DRILLDOWN_EXPORT, json_body=request)
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.parametrize(
    ("label", "entity_id"),
    [
        ("pre-cutover email", "somebody@example.com"),
        ("nil uuid", "00000000-0000-0000-0000-000000000000"),
    ],
)
@pytest.mark.parametrize("path", [DRILLDOWN, DRILLDOWN_EXPORT], ids=["drilldown", "export"])
def test_drilldown_400_for_a_key_that_is_not_a_person_id(
    api: ApiClient, path: str, label: str, entity_id: str
) -> None:
    """`entity.id` is a canonical person UUID since the identity cutover (#2098).

    Both spellings that used to work, or nearly, are now loud 400s — on the
    export route as well as the plain one, because a caller who has not
    migrated will hit whichever they were already using and a CSV of nothing is
    the least debuggable possible answer.
    """
    request = dict(_request())
    request["entity"] = {"type": "person", "id": entity_id}
    if path == DRILLDOWN_EXPORT:
        request["format"] = "csv"
        del request["limit"]

    response = api.post(path, json_body=request)
    assert response.status_code == 400, (
        f"{path} answered {response.status_code} to a {label}: {response.text[:300]}"
    )
