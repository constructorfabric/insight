from __future__ import annotations

from collections.abc import Callable, Iterator

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    ApiClient,
    Manifest,
    PersonaSession,
    QueryParams,
    v3_path,
)

from .. import scratch
from ..schemas.v3_untyped import (
    DashboardDefinition,
    DashboardEntry,
    Folder,
    FolderEntry,
    FolderList,
    FolderSummary,
    Grant,
    Items,
    Page,
    WidgetDefinition,
)

pytestmark = [
    pytest.mark.security,
    pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "sales_lead", "hr_lead"),
]


@pytest.fixture
def second_dashboard(
    v3_admin: ApiClient, scratch_v3_widget: WidgetDefinition
) -> Iterator[DashboardDefinition]:
    dashboard = scratch.create_v3_dashboard(v3_admin, "second-dashboard", [scratch_v3_widget.id])
    yield dashboard
    v3_admin.delete(v3_path(f"/v1/dashboards/{dashboard.id}"))


def _folder(client: ApiClient, folder: Folder) -> FolderSummary | None:
    response = client.get(v3_path("/v1/folders"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    listed = response.parse(FolderList).folders
    return next((summary for summary in listed if summary.id == folder.id), None)


def _visible(client: ApiClient, dashboards: list[DashboardDefinition]) -> set[str]:
    seen: set[str] = set()
    for dashboard in dashboards:
        response = client.get(v3_path("/v1/dashboards"), params={"q": dashboard.name})
        assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
        seen.update(entry.id for entry in response.parse(Page[DashboardEntry]).items)

    return seen & {dashboard.id for dashboard in dashboards}


def _in_folder(client: ApiClient, folder: Folder) -> set[str]:
    params: QueryParams = {"folder": str(folder.id)}
    response = client.get(v3_path("/v1/dashboards"), params=params)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return {entry.id for entry in response.parse(Page[DashboardEntry]).items}


def _folder_grants(client: ApiClient, folder: Folder) -> list[Grant]:
    response = client.get(v3_path(f"/v1/folders/{folder.id}/grants"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(Items[Grant]).items


def _dashboard_grants(client: ApiClient, dashboard: DashboardDefinition) -> list[Grant]:
    response = client.get(v3_path(f"/v1/dashboards/{dashboard.id}/grants"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(Items[Grant]).items


def test_sharing_a_folder_shares_every_dashboard_in_it_and_withdrawing_it_hides_them_again(
    v3_admin: ApiClient,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
    scratch_v3_folder: Folder,
    scratch_v3_dashboard: DashboardDefinition,
    second_dashboard: DashboardDefinition,
) -> None:
    sales = session_for("sales_lead").client
    hr = session_for("hr_lead").client
    sales_id = stand_manifest.fixture("sales_lead").uuid
    folder = scratch_v3_folder
    dashboards = [scratch_v3_dashboard, second_dashboard]
    both = {dashboard.id for dashboard in dashboards}

    for dashboard in dashboards:
        filed = v3_admin.put(v3_path(f"/v1/folders/{folder.id}/dashboards/{dashboard.id}"))
        assert filed.status_code == 200, f"status={filed.status_code} {filed.text[:300]}"
        assert filed.parse(FolderEntry).carries is True

    owned = _folder(v3_admin, folder)
    assert owned is not None
    assert owned.dashboards == 2
    assert _folder(sales, folder) is None
    assert _visible(sales, dashboards) == set()

    granted = v3_admin.put(
        v3_path(f"/v1/folders/{folder.id}/grants/{sales_id}"), json_body={"level": "viewer"}
    )
    assert granted.status_code == 204, f"status={granted.status_code} {granted.text[:300]}"

    assert [(str(grant.grantee), grant.level) for grant in _folder_grants(v3_admin, folder)] == [
        (sales_id, "viewer")
    ]
    shared = _folder(sales, folder)
    assert shared is not None
    assert (shared.access, shared.dashboards) == ("viewer", 2)
    assert _visible(sales, dashboards) == both
    assert _in_folder(sales, folder) == both
    assert all(_dashboard_grants(v3_admin, dashboard) == [] for dashboard in dashboards)

    assert _folder(hr, folder) is None
    assert _visible(hr, dashboards) == set()

    withdrawn = v3_admin.delete(v3_path(f"/v1/folders/{folder.id}/grants/{sales_id}"))
    assert withdrawn.status_code == 204, f"status={withdrawn.status_code} {withdrawn.text[:300]}"

    assert _folder_grants(v3_admin, folder) == []
    assert _folder(sales, folder) is None
    assert _visible(sales, dashboards) == set()
    owned = _folder(v3_admin, folder)
    assert owned is not None
    assert owned.dashboards == 2
