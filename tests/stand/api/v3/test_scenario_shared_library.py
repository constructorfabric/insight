from __future__ import annotations

import uuid
from collections.abc import Callable, Iterator

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    SCRATCH_PREFIX,
    ApiClient,
    Manifest,
    PersonaSession,
    QueryParams,
    scratch_name,
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
    Page,
    Pins,
    TagList,
)

pytestmark = [
    pytest.mark.reliability,
    pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "dev_lead"),
]


@pytest.fixture
def dev_lead(session_for: Callable[[str], PersonaSession]) -> ApiClient:
    return session_for("dev_lead").client


@pytest.fixture
def leads_folder(dev_lead: ApiClient) -> Iterator[Folder]:
    folder = scratch.create_v3_folder(dev_lead, "lead-folder")
    yield folder
    dev_lead.delete(v3_path(f"/v1/folders/{folder.id}"))


def _scratch_tag() -> str:
    return f"{SCRATCH_PREFIX}-{uuid.uuid4().hex[:12]}"


def _folder(client: ApiClient, folder: Folder) -> FolderSummary | None:
    response = client.get(v3_path("/v1/folders"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    listed = response.parse(FolderList).folders
    return next((summary for summary in listed if summary.id == folder.id), None)


def _listed(client: ApiClient, params: QueryParams) -> list[str]:
    response = client.get(v3_path("/v1/dashboards"), params=params)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return [entry.id for entry in response.parse(Page[DashboardEntry]).items]


def _pinned(client: ApiClient) -> list[str]:
    response = client.get(v3_path("/v1/pins"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return [pin.id for pin in response.parse(Pins).pins]


def _tag_counts(client: ApiClient) -> dict[str, int]:
    response = client.get(v3_path("/v1/tags"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return {tag.name: tag.dashboards for tag in response.parse(TagList).tags}


def _set_grant(client: ApiClient, dashboard: DashboardDefinition, grantee: str) -> None:
    response = client.put(
        v3_path(f"/v1/dashboards/{dashboard.id}/grants/{grantee}"),
        json_body={"level": "viewer"},
    )
    assert response.status_code == 204, f"status={response.status_code} {response.text[:300]}"


def _withdraw(client: ApiClient, dashboard: DashboardDefinition, grantee: str) -> None:
    response = client.delete(v3_path(f"/v1/dashboards/{dashboard.id}/grants/{grantee}"))
    assert response.status_code == 204, f"status={response.status_code} {response.text[:300]}"


def test_a_lead_files_pins_and_filters_a_shared_dashboard_and_loses_them_only_while_the_grant_is_gone(
    v3_admin: ApiClient,
    dev_lead: ApiClient,
    stand_manifest: Manifest,
    scratch_v3_dashboard: DashboardDefinition,
    leads_folder: Folder,
) -> None:
    dev_id = stand_manifest.fixture("dev_lead").uuid
    dashboard = scratch_v3_dashboard
    in_folder: QueryParams = {"folder": str(leads_folder.id)}
    tag = _scratch_tag()

    _set_grant(v3_admin, dashboard, dev_id)

    filed = dev_lead.put(v3_path(f"/v1/folders/{leads_folder.id}/dashboards/{dashboard.id}"))
    assert filed.status_code == 200, f"status={filed.status_code} {filed.text[:300]}"
    assert filed.parse(FolderEntry).carries is False

    summary = _folder(dev_lead, leads_folder)
    assert summary is not None
    assert (summary.access, summary.dashboards) == ("owner", 1)
    assert _listed(dev_lead, in_folder) == [dashboard.id]
    assert _folder(v3_admin, leads_folder) is None

    new_name = scratch_name("lead-folder-renamed")
    renamed = dev_lead.request(
        "PATCH", v3_path(f"/v1/folders/{leads_folder.id}"), json_body={"name": new_name}
    )
    assert renamed.status_code == 200, f"status={renamed.status_code} {renamed.text[:300]}"
    assert renamed.parse(Folder).name == new_name
    summary = _folder(dev_lead, leads_folder)
    assert summary is not None
    assert summary.name == new_name

    pinned = dev_lead.put(v3_path(f"/v1/pins/{dashboard.id}"))
    assert pinned.status_code == 204, f"status={pinned.status_code} {pinned.text[:300]}"
    assert dashboard.id in _pinned(dev_lead)

    tagged = v3_admin.put(v3_path(f"/v1/dashboards/{dashboard.id}/tags"), json_body={"tags": [tag]})
    assert tagged.status_code == 204, f"status={tagged.status_code} {tagged.text[:300]}"
    assert _tag_counts(dev_lead).get(tag) == 1
    assert _listed(dev_lead, {"tag": tag}) == [dashboard.id]

    _withdraw(v3_admin, dashboard, dev_id)

    assert dashboard.id not in _pinned(dev_lead)
    summary = _folder(dev_lead, leads_folder)
    assert summary is not None
    assert summary.dashboards == 0
    assert _listed(dev_lead, in_folder) == []
    assert tag not in _tag_counts(dev_lead)

    _set_grant(v3_admin, dashboard, dev_id)

    assert dashboard.id in _pinned(dev_lead)
    summary = _folder(dev_lead, leads_folder)
    assert summary is not None
    assert summary.dashboards == 1
    assert _listed(dev_lead, in_folder) == [dashboard.id]

    unpinned = dev_lead.delete(v3_path(f"/v1/pins/{dashboard.id}"))
    assert unpinned.status_code == 204, f"status={unpinned.status_code} {unpinned.text[:300]}"
    assert dashboard.id not in _pinned(dev_lead)

    unfiled = dev_lead.delete(v3_path(f"/v1/folders/{leads_folder.id}/dashboards/{dashboard.id}"))
    assert unfiled.status_code == 204, f"status={unfiled.status_code} {unfiled.text[:300]}"
    assert _listed(dev_lead, in_folder) == []

    removed = dev_lead.delete(v3_path(f"/v1/folders/{leads_folder.id}"))
    assert removed.status_code == 204, f"status={removed.status_code} {removed.text[:300]}"
    assert _folder(dev_lead, leads_folder) is None

    response = v3_admin.get(v3_path(f"/v1/dashboards/{dashboard.id}"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    intact = response.parse(DashboardDefinition)
    assert (intact.name, intact.body, intact.tags) == (dashboard.name, dashboard.body, [tag])
    assert dashboard.id in _listed(dev_lead, {"q": dashboard.name})
