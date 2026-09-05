"""The org chart a connector's manager signal becomes, end to end.

A bamboohr `supervisorEmail` reaches `identity.identity_inputs` as one `parent_email`
observation through the connector's own dbt models, persons-seed projects it into one
open org-chart edge, and the lead reads the chain back through `/v1/subchart` at the
depth it was synced with. A cyclic chain is stored as observed and bounded on the read;
the ms-entra connector emits no manager signal at all.
"""

from __future__ import annotations

import uuid
from collections.abc import Sequence
from typing import Any

import pytest
from insight_datapath import clickhouse as ch
from insight_datapath import identity_journal
from insight_datapath.connector_path import ConnectorPath
from insight_datapath.instance import InstanceConfig
from insight_datapath.records import employee
from insight_datapath.subjects import SubjectError, Subjects
from insight_stand.api import ApiClient, JsonValue, identity_path
from insight_stand.personas import PersonaSession
from insight_stand.scratch_identity import scratch_name

pytestmark = pytest.mark.fixture

EMPLOYEES = "bronze_bamboohr.employees"
ENTRA_USERS = "bronze_ms_entra.users"

PROFILE_TYPES = ("person_display_name", "person_first_name", "person_last_name")
MANAGER_SIGNALS = frozenset({"parent_email", "parent_id"})

#: INVARIANT: equals `max_depth` in deploy/compose/identity-resolution-fullauth.yaml.
SUBCHART_MAX_DEPTH = 16
CHAIN_LENGTH = 5


def _email(role: str, run_tag: str) -> str:
    return f"{role}.{run_tag}@example.com"


def _upserts(
    cfg: InstanceConfig, *, source_type: str, account: str, value_types: Sequence[str]
) -> list[tuple[str, str]]:
    """The UPSERT observations of `account` for the given value types, by value type."""
    wanted = ", ".join(f"'{value_type}'" for value_type in value_types)
    rows = ch.query(
        cfg,
        "SELECT value_type, value FROM identity.identity_inputs"
        f" WHERE insight_source_type = '{source_type}' AND source_account_id = '{account}'"
        f"   AND value_type IN ({wanted}) AND operation_type = 'UPSERT'"
        " ORDER BY value_type",
    )
    return [(str(value_type), str(value)) for value_type, value in rows]


def _value_types(cfg: InstanceConfig, *, source_type: str, account: str) -> set[str]:
    rows = ch.query(
        cfg,
        "SELECT DISTINCT value_type FROM identity.identity_inputs"
        f" WHERE insight_source_type = '{source_type}' AND source_account_id = '{account}'",
    )
    return {str(value_type) for (value_type,) in rows}


def _entra_user(*, tenant: str, run_tag: str, account: str, email: str) -> dict[str, str | int]:
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": "2026-01-05T00:00:00",
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": tenant,
        "source_id": f"ms-entra-test-{run_tag}",
        "unique_key": f"ms-entra-test-{account}",
        "id": account,
        "mail": email,
    }


def _persons_of(subjects: Subjects, rows: Sequence[dict[str, Any]]) -> dict[str, str]:
    """The person minted for every record, by email; a record left unminted fails."""
    ids = subjects.person_ids_of_records(rows)

    expected = {str(row["workEmail"]) for row in rows}
    assert set(ids) == expected, (
        f"persons-seed minted {sorted(ids)}; the records name {sorted(expected)}"
    )
    return ids


def _open_parent(cfg: InstanceConfig, tenant: str, child: str) -> str | None:
    return identity_journal.open_parent(cfg, tenant=tenant, child=child)


def _subchart(lead: ApiClient, root: str, *, depth: int | None = None) -> dict[str, JsonValue]:
    params = None if depth is None else {"depth": depth}
    response = lead.get(identity_path(f"/v1/subchart/{root}"), params=params)
    assert response.status_code == 200, (
        f"the lead reading the subchart of {root}: {response.status_code} {response.text[:300]}"
    )

    body = response.json()
    assert isinstance(body, dict), f"subchart body of {root}: {response.text[:300]}"
    node = body.get("root")
    assert isinstance(node, dict), f"subchart root of {root}: {response.text[:300]}"
    return node


def _walk(node: JsonValue, depth: int = 0) -> list[tuple[str, int]]:
    """Every node of the subtree as (person_id, depth), the root at depth 0."""
    assert isinstance(node, dict), f"a subchart node is an object: {node!r}"
    subordinates = node.get("subordinates", [])
    assert isinstance(subordinates, list), f"subordinates of {node.get('person_id')}: {node!r}"

    found = [(str(node["person_id"]), depth)]
    for child in subordinates:
        found += _walk(child, depth + 1)
    return found


def _grant_sight(operator: ApiClient, *, viewer: str, viewed: str) -> str:
    created = operator.post(
        identity_path("/v1/visibility"),
        json_body={
            "viewer_person_id": viewer,
            "viewed_person_id": viewed,
            "reason": scratch_name("org-chart-cycle"),
        },
    )
    assert created.status_code == 201, (
        f"granting sight of {viewed}: {created.status_code} {created.text[:300]}"
    )

    body = created.json()
    assert isinstance(body, dict) and body.get("visibility_id"), (
        f"grant body for {viewed}: {created.text[:300]}"
    )
    return str(body["visibility_id"])


def _revoke_sight(operator: ApiClient, grant_id: str) -> None:
    revoked = operator.delete(identity_path(f"/v1/visibility/{grant_id}"))
    assert revoked.status_code == 204, (
        f"revoking grant {grant_id}: {revoked.status_code} {revoked.text[:300]}"
    )


def test_the_bamboohr_pipeline_projects_a_supervisor_into_one_open_edge(
    connector_path: ConnectorPath,
    substitutions: dict[str, str],
    subjects: Subjects,
    instance_cfg: InstanceConfig,
    tenant: str,
    lead: ApiClient,
    caller_session: PersonaSession,
    run_tag: str,
) -> None:
    """A report's `supervisorEmail` becomes exactly one `parent_email` observation beside
    the profile rows, persons-seed turns it into the report's one open edge, and the lead
    sees the manager with that one report beneath them."""
    manager_key, report_key = f"mgr-{run_tag}", f"rep-{run_tag}"
    manager_email, report_email = _email("manager", run_tag), _email("report", run_tag)
    rows = [
        employee(
            substitutions=substitutions,
            key=manager_key,
            email=manager_email,
            display_name="Meg Manager",
        ),
        employee(
            substitutions=substitutions,
            key=report_key,
            email=report_email,
            display_name="Rob Report",
            supervisor_email=manager_email,
        ),
    ]
    connector_path.build({EMPLOYEES: rows})

    parent_rows = _upserts(
        instance_cfg, source_type="bamboohr", account=report_key, value_types=("parent_email",)
    )
    assert parent_rows == [("parent_email", manager_email)], (
        f"the report's manager signal in identity_inputs: {parent_rows}"
    )
    profile_rows = _upserts(
        instance_cfg, source_type="bamboohr", account=report_key, value_types=PROFILE_TYPES
    )
    assert profile_rows == [
        ("person_display_name", "Rob Report"),
        ("person_first_name", "Rob"),
        ("person_last_name", "Report"),
    ], f"the report's profile rows in identity_inputs: {profile_rows}"

    subjects.publish()
    ids = _persons_of(subjects, rows)
    manager, report = ids[manager_email], ids[report_email]

    assert _open_parent(instance_cfg, tenant, report) == manager, (
        f"the report's open parent is not the manager {manager}"
    )
    assert _open_parent(instance_cfg, tenant, manager) == caller_session.person.uuid, (
        f"the manager's open parent is not the lead {caller_session.person.uuid}"
    )

    tree = _walk(_subchart(lead, manager))
    assert tree == [(manager, 0), (report, 1)], f"the lead's view of the manager's team: {tree}"


def test_a_circular_manager_chain_terminates_and_stays_bounded(
    connector_path: ConnectorPath,
    substitutions: dict[str, str],
    subjects: Subjects,
    instance_cfg: InstanceConfig,
    tenant: str,
    operator: ApiClient,
    lead: ApiClient,
    caller_session: PersonaSession,
    run_tag: str,
) -> None:
    """Two people naming each other as manager are stored as observed, both edges open,
    and the subchart descends the cycle no further than the server's depth cap."""
    a_key, b_key = f"cycle-a-{run_tag}", f"cycle-b-{run_tag}"
    a_email, b_email = _email("cycle.a", run_tag), _email("cycle.b", run_tag)
    rows = [
        employee(
            substitutions=substitutions,
            key=a_key,
            email=a_email,
            display_name="Cy Alpha",
            supervisor_email=b_email,
        ),
        employee(
            substitutions=substitutions,
            key=b_key,
            email=b_email,
            display_name="Cy Beta",
            supervisor_email=a_email,
        ),
    ]
    connector_path.build({EMPLOYEES: rows})

    try:
        subjects.publish()
    except SubjectError as exc:
        pytest.fail(f"persons-seed must terminate on a cyclic manager chain: {exc}")
    ids = _persons_of(subjects, rows)
    person_a, person_b = ids[a_email], ids[b_email]

    assert _open_parent(instance_cfg, tenant, person_a) == person_b, (
        f"{a_email}'s open parent is not {b_email}"
    )
    assert _open_parent(instance_cfg, tenant, person_b) == person_a, (
        f"{b_email}'s open parent is not {a_email}"
    )

    grant_id = _grant_sight(operator, viewer=caller_session.person.uuid, viewed=person_a)
    try:
        depths = [depth for _, depth in _walk(_subchart(lead, person_a))]
        assert 2 <= max(depths) <= SUBCHART_MAX_DEPTH, (
            f"the descent of a cyclic chart reached depth {max(depths)}; "
            f"expected between 2 and {SUBCHART_MAX_DEPTH}"
        )
    finally:
        _revoke_sight(operator, grant_id)


def test_the_ms_entra_connector_emits_no_manager_signal(
    connector_path: ConnectorPath,
    instance_cfg: InstanceConfig,
    tenant: str,
    run_tag: str,
) -> None:
    """An ms-entra user reaches identity with profile signals and no `parent_email` or
    `parent_id`, so nothing from that connector can shape the org chart."""
    account = f"entra-{run_tag}"
    user = _entra_user(
        tenant=tenant, run_tag=run_tag, account=account, email=_email("entra", run_tag)
    )
    connector_path.build({ENTRA_USERS: [user]})

    value_types = _value_types(instance_cfg, source_type="ms-entra", account=account)
    assert value_types, f"the ms-entra user {account} reached no identity input at all"
    assert not value_types & MANAGER_SIGNALS, (
        f"ms-entra emits a manager signal for {account}: {sorted(value_types & MANAGER_SIGNALS)}"
    )


def test_a_five_level_chain_projects_every_level_at_its_depth(
    connector_path: ConnectorPath,
    substitutions: dict[str, str],
    subjects: Subjects,
    instance_cfg: InstanceConfig,
    tenant: str,
    lead: ApiClient,
    caller_session: PersonaSession,
    run_tag: str,
) -> None:
    """A synced chain of five supervisors projects one open edge per level, and the
    subchart read at that depth places every level exactly where the chain put it."""
    emails = [_email(f"chain.{level}", run_tag) for level in range(CHAIN_LENGTH)]
    rows = []
    for level, email in enumerate(emails):
        supervisor = emails[level - 1] if level else None
        rows.append(
            employee(
                substitutions=substitutions,
                key=f"chain-{level}-{run_tag}",
                email=email,
                display_name=f"Chain Level{level}",
                supervisor_email=supervisor,
            )
        )
    connector_path.build({EMPLOYEES: rows})

    subjects.publish()
    ids = _persons_of(subjects, rows)
    chain = [ids[email] for email in emails]

    assert _open_parent(instance_cfg, tenant, chain[0]) == caller_session.person.uuid, (
        f"level 0's open parent is not the lead {caller_session.person.uuid}"
    )
    for level in range(1, CHAIN_LENGTH):
        assert _open_parent(instance_cfg, tenant, chain[level]) == chain[level - 1], (
            f"level {level}'s open parent is not level {level - 1}"
        )

    depth_by_person = dict(_walk(_subchart(lead, chain[0], depth=CHAIN_LENGTH)))
    for level, person in enumerate(chain):
        assert depth_by_person.get(person) == level, (
            f"level {level} ({person}) surfaced at depth {depth_by_person.get(person)!r}"
        )
