from __future__ import annotations

from collections.abc import Callable, Iterator
from dataclasses import dataclass, field
from typing import Literal

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    ApiClient,
    Manifest,
    PersonaSession,
    identity_path,
    v3_path,
)

from .. import scratch
from ..schemas import PersonRole, RoleList
from ..schemas.v3_untyped import (
    DashboardDefinition,
    FolderList,
    FolderSummary,
    Orphan,
    Page,
)

pytestmark = [
    pytest.mark.reliability,
    pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "support_lead", "sales_lead"),
]

TEMPORARY_ADMIN = "support_lead"
PAGE = 200


@dataclass
class TemporaryAdmin:
    client: ApiClient
    person_id: str
    assignment: str
    dashboards: list[str] = field(default_factory=list)
    folders: list[str] = field(default_factory=list)


def _admin_role_id(client: ApiClient) -> str:
    response = client.get(identity_path("/v1/roles"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    roles = response.parse(RoleList).items
    return next(str(role.role_id) for role in roles if role.name == "admin")


@pytest.fixture
def temporary_admin(
    v3_admin: ApiClient,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> Iterator[TemporaryAdmin]:
    person_id = stand_manifest.fixture(TEMPORARY_ADMIN).uuid
    admin_id = stand_manifest.fixture(ADMIN_OPERATOR_FIXTURE).uuid

    response = v3_admin.post(
        identity_path("/v1/person-roles"),
        json_body={
            "person_id": person_id,
            "role_id": _admin_role_id(v3_admin),
            "reason": "stand scenario: orphan handover",
        },
    )
    assert response.status_code == 201, f"status={response.status_code} {response.text[:300]}"
    assignment = scratch.track(
        identity_path("/v1/person-roles"),
        "person_role_id",
        response.parse(PersonRole).person_role_id,
    )

    made = TemporaryAdmin(session_for(TEMPORARY_ADMIN).client, person_id, assignment)
    yield made

    for folder in made.folders:
        made.client.delete(v3_path(f"/v1/folders/{folder}"))
    for dashboard in made.dashboards:
        made.client.delete(v3_path(f"/v1/dashboards/{dashboard}"))

    revoked = v3_admin.delete(identity_path(f"/v1/person-roles/{assignment}"))
    assert revoked.status_code in {204, 404}, f"status={revoked.status_code} {revoked.text[:300]}"

    for segment, ids in (("dashboards", made.dashboards), ("folders", made.folders)):
        for leftover in ids:
            v3_admin.post(v3_path(f"/v1/{segment}/{leftover}/transfer"), json_body={"to": admin_id})
            v3_admin.delete(v3_path(f"/v1/{segment}/{leftover}"))


def _orphaned(client: ApiClient, segment: Literal["dashboards", "folders"]) -> dict[str, Orphan]:
    found: dict[str, Orphan] = {}
    offset = 0
    while True:
        response = client.get(
            v3_path(f"/v1/orphaned-{segment}"), params={"limit": PAGE, "offset": offset}
        )
        assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

        page = response.parse(Page[Orphan])
        found.update((orphan.id, orphan) for orphan in page.items)
        offset += PAGE
        if offset >= page.total:
            return found


def _folder(client: ApiClient, folder: str) -> FolderSummary | None:
    response = client.get(v3_path("/v1/folders"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    listed = response.parse(FolderList).folders
    return next((summary for summary in listed if str(summary.id) == folder), None)


def _opened(client: ApiClient, dashboard: str) -> DashboardDefinition:
    response = client.get(v3_path(f"/v1/dashboards/{dashboard}"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(DashboardDefinition)


def test_work_left_by_an_owner_who_stops_being_an_administrator_is_handed_over_and_stays_shared(
    v3_admin: ApiClient,
    temporary_admin: TemporaryAdmin,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    sales = session_for("sales_lead").client
    sales_id = stand_manifest.fixture("sales_lead").uuid
    admin = stand_manifest.fixture(ADMIN_OPERATOR_FIXTURE)
    owner = temporary_admin.client

    dashboard = scratch.create_v3_dashboard(owner, "handover-dashboard")
    temporary_admin.dashboards.append(dashboard.id)
    folder = str(scratch.create_v3_folder(owner, "handover-folder").id)
    temporary_admin.folders.append(folder)
    assert (str(dashboard.owner_person_id), dashboard.access) == (
        temporary_admin.person_id,
        "owner",
    )

    for segment, shared in (("dashboards", dashboard.id), ("folders", folder)):
        granted = owner.put(
            v3_path(f"/v1/{segment}/{shared}/grants/{sales_id}"), json_body={"level": "viewer"}
        )
        assert granted.status_code == 204, f"status={granted.status_code} {granted.text[:300]}"

    assert dashboard.id not in _orphaned(v3_admin, "dashboards")
    assert folder not in _orphaned(v3_admin, "folders")

    revoked = v3_admin.delete(identity_path(f"/v1/person-roles/{temporary_admin.assignment}"))
    assert revoked.status_code == 204, f"status={revoked.status_code} {revoked.text[:300]}"

    orphaned_dashboard = _orphaned(v3_admin, "dashboards").get(dashboard.id)
    assert orphaned_dashboard is not None
    assert (orphaned_dashboard.name, str(orphaned_dashboard.owner_person_id)) == (
        dashboard.name,
        temporary_admin.person_id,
    )
    orphaned_folder = _orphaned(v3_admin, "folders").get(folder)
    assert orphaned_folder is not None
    assert str(orphaned_folder.owner_person_id) == temporary_admin.person_id

    for segment, orphan in (("dashboards", dashboard.id), ("folders", folder)):
        handed = v3_admin.post(
            v3_path(f"/v1/{segment}/{orphan}/transfer"), json_body={"to": admin.uuid}
        )
        assert handed.status_code == 204, f"status={handed.status_code} {handed.text[:300]}"

    assert dashboard.id not in _orphaned(v3_admin, "dashboards")
    assert folder not in _orphaned(v3_admin, "folders")

    taken_over = _opened(v3_admin, dashboard.id)
    assert (str(taken_over.owner_person_id), taken_over.owner_name, taken_over.access) == (
        admin.uuid,
        admin.display_name,
        "owner",
    )
    adopted = _folder(v3_admin, folder)
    assert adopted is not None
    assert (str(adopted.owner_person_id), adopted.access) == (admin.uuid, "owner")

    still_shared = _opened(sales, dashboard.id)
    assert (str(still_shared.owner_person_id), still_shared.access) == (admin.uuid, "viewer")
    audience_folder = _folder(sales, folder)
    assert audience_folder is not None
    assert (str(audience_folder.owner_person_id), audience_folder.access) == (admin.uuid, "viewer")
