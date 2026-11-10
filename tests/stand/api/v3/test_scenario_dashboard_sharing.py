from __future__ import annotations

from collections.abc import Callable

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    ApiClient,
    ApiResponse,
    Manifest,
    PersonaSession,
    scratch_name,
    v3_path,
)

from .. import scratch
from ..schemas import ProblemDocument
from ..schemas.v3_untyped import (
    CreatedId,
    DashboardDefinition,
    DashboardEntry,
    Dataset,
    DrawnWidget,
    Grant,
    Items,
    Page,
    RunResult,
    WidgetDefinition,
)

pytestmark = [
    pytest.mark.security,
    pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "dev_lead", "sales_lead"),
]

EVERYONE = "everyone"


def _listed(client: ApiClient, dashboard: DashboardDefinition) -> list[str]:
    response = client.get(v3_path("/v1/dashboards"), params={"q": dashboard.name})
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return [entry.id for entry in response.parse(Page[DashboardEntry]).items]


def _opened(client: ApiClient, dashboard: DashboardDefinition) -> DashboardDefinition:
    response = client.get(v3_path(f"/v1/dashboards/{dashboard.id}"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(DashboardDefinition)


def _assert_hidden_from(client: ApiClient, dashboard: DashboardDefinition) -> None:
    assert dashboard.id not in _listed(client, dashboard)

    response = client.get(v3_path(f"/v1/dashboards/{dashboard.id}"))
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 404


def _grant(client: ApiClient, dashboard: DashboardDefinition, grantee: str) -> None:
    response = client.put(
        v3_path(f"/v1/dashboards/{dashboard.id}/grants/{grantee}"),
        json_body={"level": "viewer"},
    )
    assert response.status_code == 204, f"status={response.status_code} {response.text[:300]}"


def _withdraw(client: ApiClient, dashboard: DashboardDefinition, grantee: str) -> None:
    response = client.delete(v3_path(f"/v1/dashboards/{dashboard.id}/grants/{grantee}"))
    assert response.status_code == 204, f"status={response.status_code} {response.text[:300]}"


def _grants(client: ApiClient, dashboard: DashboardDefinition) -> list[Grant]:
    response = client.get(v3_path(f"/v1/dashboards/{dashboard.id}/grants"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(Items[Grant]).items


def _run(client: ApiClient, dashboard: DashboardDefinition, widget: str) -> RunResult:
    response = client.post(v3_path(f"/v1/dashboards/{dashboard.id}/widgets/{widget}/run"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(RunResult)


def _assert_refused(response: ApiResponse) -> None:
    assert response.status_code == 403, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 403


def test_a_viewer_grant_lets_a_lead_read_and_run_a_dashboard_but_not_change_it_and_withdrawal_takes_it_back(
    v3_admin: ApiClient,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
    scratch_v3_dataset: Dataset,
    scratch_v3_widget: WidgetDefinition,
    scratch_v3_dashboard: DashboardDefinition,
) -> None:
    dev = session_for("dev_lead").client
    sales = session_for("sales_lead").client
    dev_id = stand_manifest.fixture("dev_lead").uuid
    admin_id = stand_manifest.fixture(ADMIN_OPERATOR_FIXTURE).uuid
    dashboard = scratch_v3_dashboard

    _assert_hidden_from(dev, dashboard)
    _assert_hidden_from(sales, dashboard)
    assert _opened(v3_admin, dashboard).visibility == "private"

    _grant(v3_admin, dashboard, dev_id)

    granted = _grants(v3_admin, dashboard)
    assert [(str(grant.grantee), grant.level, str(grant.granted_by)) for grant in granted] == [
        (dev_id, "viewer", admin_id)
    ]
    assert _opened(v3_admin, dashboard).visibility == "people"

    assert dashboard.id in _listed(dev, dashboard)
    seen = _opened(dev, dashboard)
    assert seen.access == "viewer"
    assert str(seen.owner_person_id) == admin_id
    assert seen.body == dashboard.body
    _assert_hidden_from(sales, dashboard)

    widgets = dev.get(v3_path(f"/v1/dashboards/{dashboard.id}/widgets"))
    assert widgets.status_code == 200, f"status={widgets.status_code} {widgets.text[:300]}"
    drawn = widgets.parse(Items[DrawnWidget]).items
    assert [widget.id for widget in drawn] == [scratch_v3_widget.id]
    assert drawn[0].body == scratch_v3_widget.body

    owners_rows = _run(v3_admin, dashboard, scratch_v3_widget.id)
    viewers_rows = _run(dev, dashboard, scratch_v3_widget.id)
    assert viewers_rows.columns == [scratch.SCRATCH_METRIC_COLUMN]
    assert viewers_rows == owners_rows

    renamed = dev.put(
        v3_path(f"/v1/dashboards/{dashboard.id}"),
        json_body={"name": scratch_name("renamed"), "body": dashboard.body},
    )
    _assert_refused(renamed)
    assert _opened(v3_admin, dashboard).name == dashboard.name

    library = dev.get(v3_path("/v1/metrics"))
    _assert_refused(library)

    authored = dev.post(
        v3_path("/v1/metrics"),
        json_body={
            "name": scratch_name("metric"),
            "body": {
                "dataset": scratch_v3_dataset.name,
                "fields": [
                    {"agg": "count", "type": "int", "as_name": scratch.SCRATCH_METRIC_COLUMN}
                ],
            },
        },
    )
    if authored.status_code == 201:
        v3_admin.delete(v3_path(f"/v1/metrics/{authored.parse(CreatedId).id}"))
    _assert_refused(authored)

    _grant(v3_admin, dashboard, EVERYONE)

    assert {str(grant.grantee) for grant in _grants(v3_admin, dashboard)} == {dev_id, EVERYONE}
    assert _opened(v3_admin, dashboard).visibility == "everyone"
    assert dashboard.id in _listed(sales, dashboard)
    assert _opened(sales, dashboard).access == "viewer"

    _withdraw(v3_admin, dashboard, dev_id)
    _withdraw(v3_admin, dashboard, EVERYONE)

    assert _grants(v3_admin, dashboard) == []
    assert _opened(v3_admin, dashboard).visibility == "private"
    _assert_hidden_from(dev, dashboard)
    _assert_hidden_from(sales, dashboard)
