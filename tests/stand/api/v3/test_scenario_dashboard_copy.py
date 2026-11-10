from __future__ import annotations

from collections.abc import Callable, Iterator
from typing import Final

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    RUN_TAG,
    ApiClient,
    ApiResponse,
    PersonaSession,
    identity_path,
    scratch_name,
    v3_path,
)

from .. import scratch
from ..schemas import PersonRole, RoleList
from ..schemas.v3_untyped import (
    CreatedId,
    DashboardDefinition,
    DashboardEntry,
    Dependents,
    Folder,
    FolderEntry,
    Page,
    WidgetDefinition,
)

_SECOND_ADMIN: Final[str] = "hr_ic"
_TAG: Final[str] = f"copied-{RUN_TAG}"


def _expect(response: ApiResponse, status: int) -> ApiResponse:
    assert response.status_code == status, (
        f"{response.url}: expected {status}, got {response.status_code} {response.text[:300]}"
    )
    return response


def _listed(client: ApiClient, owned: str, needle: str) -> list[str]:
    response = client.get(v3_path("/v1/dashboards"), params={"owned": owned, "q": needle})
    return [entry.id for entry in _expect(response, 200).parse(Page[DashboardEntry]).items]


@pytest.fixture
def second_admin(
    admin_operator_session: PersonaSession, session_for: Callable[[str], PersonaSession]
) -> Iterator[PersonaSession]:
    operator = admin_operator_session.client
    colleague = session_for(_SECOND_ADMIN)

    roles = _expect(operator.get(identity_path("/v1/roles")), 200).parse(RoleList)
    admin_role = next(str(role.role_id) for role in roles.items if role.name == "admin")

    granted = _expect(
        operator.post(
            identity_path("/v1/person-roles"),
            json_body={
                "person_id": colleague.person.uuid,
                "role_id": admin_role,
                "reason": "stand scenario: a second administrator owns a dashboard",
            },
        ),
        201,
    ).parse(PersonRole)
    assignment = scratch.track(
        identity_path("/v1/person-roles"), "person_role_id", granted.person_role_id
    )

    yield colleague

    revoked = operator.delete(identity_path(f"/v1/person-roles/{assignment}"))
    assert revoked.status_code in {204, 404}, (
        f"the temporary admin grant to {_SECOND_ADMIN} was not revoked: {revoked.status_code}"
    )


@pytest.fixture
def colleague_dashboard(
    second_admin: PersonaSession,
    admin_operator_session: PersonaSession,
    scratch_v3_widget: WidgetDefinition,
) -> Iterator[DashboardDefinition]:
    colleague = second_admin.client
    name = scratch_name("colleague")
    body = {"title": name, "items": [{"widget": scratch_v3_widget.id}]}

    created = _expect(
        colleague.post(v3_path("/v1/dashboards"), json_body={"name": name, "body": body}), 201
    ).parse(CreatedId)
    scratch.track(v3_path("/v1/dashboards"), "id", created.id, params={"q": name, "limit": 200})
    path = v3_path(f"/v1/dashboards/{created.id}")

    try:
        _expect(colleague.put(f"{path}/tags", json_body={"tags": [_TAG]}), 204)
        _expect(
            colleague.put(
                f"{path}/grants/{admin_operator_session.person.uuid}",
                json_body={"level": "editor"},
            ),
            204,
        )
        yield _expect(colleague.get(path), 200).parse(DashboardDefinition)
    finally:
        colleague.delete(path)


@pytest.fixture
def copies(
    v3_admin: ApiClient,
    colleague_dashboard: DashboardDefinition,
    scratch_v3_folder: Folder,
) -> Iterator[list[str]]:
    ids: list[str] = []
    yield ids

    for copy_id in ids:
        v3_admin.delete(v3_path(f"/v1/dashboards/{copy_id}"))


@pytest.mark.reliability
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, _SECOND_ADMIN)
def test_an_administrator_copies_a_colleagues_dashboard_as_their_own(
    v3_admin: ApiClient,
    admin_operator_session: PersonaSession,
    second_admin: PersonaSession,
    colleague_dashboard: DashboardDefinition,
    scratch_v3_widget: WidgetDefinition,
    scratch_v3_folder: Folder,
    copies: list[str],
) -> None:
    original_path = v3_path(f"/v1/dashboards/{colleague_dashboard.id}")
    operator = admin_operator_session.person.uuid

    original = _expect(v3_admin.get(original_path), 200).parse(DashboardDefinition)
    assert (str(original.owner_person_id), original.access) == (
        second_admin.person.uuid,
        "editor",
    )

    filed = _expect(
        v3_admin.put(v3_path(f"/v1/folders/{scratch_v3_folder.id}/dashboards/{original.id}")),
        200,
    ).parse(FolderEntry)
    assert filed.carries

    copy_name = scratch_name("copy")
    copied = _expect(
        v3_admin.post(f"{original_path}/duplicate", json_body={"name": copy_name}), 201
    ).parse(CreatedId)
    copies.append(copied.id)
    scratch.track(v3_path("/v1/dashboards"), "id", copied.id, params={"q": copy_name, "limit": 200})
    copy_path = v3_path(f"/v1/dashboards/{copied.id}")

    copy = _expect(v3_admin.get(copy_path), 200).parse(DashboardDefinition)
    assert copy.id != original.id
    assert (copy.name, str(copy.owner_person_id), copy.access) == (copy_name, operator, "owner")
    assert [folder.id for folder in copy.folders] == [scratch_v3_folder.id]
    assert copy.tags == original.tags == [_TAG]
    assert copy.body == original.body

    dependents = _expect(
        v3_admin.get(v3_path(f"/v1/widgets/{scratch_v3_widget.id}/dependents")), 200
    ).parse(Dependents)
    assert {(holder.kind, holder.id) for holder in dependents.holders} == {
        ("dashboards", original.id),
        ("dashboards", copied.id),
    }

    assert _listed(v3_admin, "me", original.name) == [copied.id]
    assert _listed(v3_admin, "shared", original.name) == [original.id]

    edited = {**copy.body, "title": f"{copy_name} edited"}
    _expect(v3_admin.put(copy_path, json_body={"name": copy_name, "body": edited}), 204)
    assert _expect(v3_admin.get(copy_path), 200).parse(DashboardDefinition).body == edited

    untouched = _expect(second_admin.client.get(original_path), 200).parse(DashboardDefinition)
    assert (untouched.name, untouched.body, untouched.tags) == (
        original.name,
        original.body,
        original.tags,
    )
