"""Two source accounts claiming one address are one person, so nobody's work is attributed
to a duplicate of themselves: the operator sees both accounts under one person, and the
lead's team holds that person once.
"""

from __future__ import annotations

from typing import Any

import pytest
from insight_datapath import clickhouse, records
from insight_datapath.connector_path import ConnectorPath
from insight_datapath.instance import InstanceConfig
from insight_datapath.subjects import Subjects
from insight_stand.api import ApiClient, identity_path
from insight_stand.personas import PersonaSession

pytestmark = pytest.mark.fixture

EMPLOYEES = "bronze_bamboohr.employees"
ORG_MEMBERS = "bronze_github_directory.org_members"


def _holders(cfg: InstanceConfig, *, key: str, member_id: int) -> dict[tuple[str, str], str]:
    """Which person each of the two accounts is assigned to, by (source_type, account_id)."""
    rows = clickhouse.query(
        cfg,
        "SELECT source_type, account_id, toString(person_id) FROM identity.account_assignment"
        f" WHERE (source_type = 'bamboohr' AND account_id = '{key}')"
        f"    OR (source_type = 'github' AND account_id = '{member_id}')",
    )
    return {(str(source), str(account)): str(person) for source, account, person in rows}


def _nodes(subtrees: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Every node beneath the given ones, at any depth."""
    found: list[dict[str, Any]] = []
    for node in subtrees:
        found.append(node)
        found.extend(_nodes(node["subordinates"]))
    return found


def test_two_accounts_sharing_an_address_are_one_person(
    connector_path: ConnectorPath,
    instance_cfg: InstanceConfig,
    subjects: Subjects,
    substitutions: dict[str, str],
    operator: ApiClient,
    lead: ApiClient,
    caller_session: PersonaSession,
    tenant: str,
    run_tag: str,
) -> None:
    """An HR record and a GitHub member sharing one address mint one person, who holds
    both accounts without an operator's hand and stands once in the lead's team."""
    member_id = int(run_tag, 16)
    key = f"emp-{run_tag}"
    email = f"rob.report.{run_tag}@example.com"
    name = "Rob Report"

    connector_path.build(
        {
            EMPLOYEES: [
                records.employee(
                    substitutions=substitutions, key=key, email=email, display_name=name
                )
            ],
            ORG_MEMBERS: [
                records.org_member(
                    source_id=f"github-directory-{run_tag}",
                    login=f"rob-report-{run_tag}",
                    member_id=member_id,
                    email=email,
                    name=name,
                    tenant=tenant,
                )
            ],
        }
    )
    subjects.publish()

    holders = _holders(instance_cfg, key=key, member_id=member_id)
    assert set(holders) == {("bamboohr", key), ("github", str(member_id))}, (
        f"the accounts assigned are {sorted(holders)!r}, not both of {email}'s"
    )
    persons = set(holders.values())
    assert len(persons) == 1, f"{email}'s accounts name {len(persons)} persons: {holders!r}"
    person = persons.pop()

    listing = operator.get(identity_path(f"/v1/resolution/persons/{person}/accounts"))
    assert listing.status_code == 200, f"{listing.status_code} {listing.text[:300]}"
    body = listing.json()
    assert isinstance(body, dict), listing.text[:300]
    bound_by_operator = {
        (str(entry["source"]), str(entry["account_id"])): entry["bound_by_operator"]
        for entry in body["accounts"]
    }
    assert bound_by_operator == {("bamboohr", key): False, ("github", str(member_id)): False}, (
        f"person {person} lists {bound_by_operator!r}, not both accounts as the seed's own decision"
    )

    team = lead.get(identity_path(f"/v1/subchart/{caller_session.person.uuid}"))
    assert team.status_code == 200, f"{team.status_code} {team.text[:300]}"
    tree = team.json()
    assert isinstance(tree, dict), team.text[:300]
    reports = _nodes(tree["root"]["subordinates"])
    mine = [node for node in reports if str(node["person_id"]) == person]
    assert len(mine) == 1, (
        f"the lead's team holds {person} {len(mine)} times among "
        f"{sorted(str(node['email']) for node in reports)!r}"
    )
