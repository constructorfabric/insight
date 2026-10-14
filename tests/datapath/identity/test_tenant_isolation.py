"""Another tenant's accounts never become this tenant's people (#3661).

Both tenants' identity inputs share one warehouse. Seeding one tenant reads only
its own inputs, and the email map resolves an address within each tenant on its
own — an address both tenants carry is two separate claims, not a contest.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass

import pytest
from insight_datapath import clickhouse, records
from insight_datapath.connector_path import ConnectorPath
from insight_datapath.instance import InstanceConfig
from insight_datapath.subjects import Subjects

pytestmark = pytest.mark.fixture

ORG_MEMBERS = "bronze_github_directory.org_members"
OTHER_TENANT = "7a1e5c3b-9d2f-4b6a-8e0c-1f2a3b4c5d6e"
SYSTEM_AUTHOR = "00000000-0000-0000-0000-000000000000"
MIRROR_ID_BASE = 1 << 62


@dataclass(frozen=True)
class SharedAddress:
    """One GitHub member per tenant, both carrying the same address."""

    own_id: int
    other_id: int
    email: str
    source_id: str


def _seed_shared_address(connector_path: ConnectorPath, tenant: str, run_tag: str) -> SharedAddress:
    own_id = int(run_tag, 16)
    shared = SharedAddress(
        own_id=own_id,
        other_id=own_id + 1,
        email=f"shared.{run_tag}@example.com",
        source_id=f"github-directory-{run_tag}",
    )
    connector_path.build(
        {
            ORG_MEMBERS: [
                records.org_member(
                    tenant=tenant,
                    source_id=shared.source_id,
                    login=f"own-{run_tag}",
                    member_id=shared.own_id,
                    email=shared.email,
                    name="Own Member",
                ),
                records.org_member(
                    tenant=OTHER_TENANT,
                    source_id=shared.source_id,
                    login=f"other-{run_tag}",
                    member_id=shared.other_id,
                    email=shared.email,
                    name="Other Member",
                ),
            ]
        }
    )
    return shared


def _assignments(cfg: InstanceConfig, member_ids: list[int]) -> dict[str, str]:
    """The tenant each GitHub account is bound under, by account id."""
    ids = ", ".join(f"'{member_id}'" for member_id in member_ids)
    rows = clickhouse.query(
        cfg,
        "SELECT account_id, tenant_id FROM identity.account_assignment"
        f" WHERE source_type = 'github' AND account_id IN ({ids})",
    )
    return {str(account): str(tenant) for account, tenant in rows}


def _map_persons(cfg: InstanceConfig, email: str) -> dict[str, str]:
    """The person the address resolves to, by tenant."""
    rows = clickhouse.query(
        cfg,
        "SELECT tenant_id, toString(person_id) FROM identity.person_map"
        f" WHERE email = {clickhouse.literal(email)}",
    )
    return {str(tenant): str(person) for tenant, person in rows}


def _bind_in_journal_mirror(cfg: InstanceConfig, shared: SharedAddress, person_id: str) -> None:
    """Stands in for the other tenant's own seed run: one binding where the maps read it."""
    clickhouse.execute(
        cfg,
        "INSERT INTO identity.identity_persons"
        " (id, value_type, insight_source_type, insight_source_id, insight_tenant_id,"
        "  value_effective, person_id, author_person_id, created_at, _synced_at)"
        f" SELECT {MIRROR_ID_BASE + shared.other_id}, 'id', 'github',"
        f" toUUID(UUIDNumToString(sipHash128({clickhouse.literal(shared.source_id)}))),"
        f" toUUID({clickhouse.literal(OTHER_TENANT)}), {clickhouse.literal(str(shared.other_id))},"
        f" toUUID({clickhouse.literal(person_id)}), toUUID({clickhouse.literal(SYSTEM_AUTHOR)}),"
        " now64(6), now64(3)",
    )


def test_a_seed_reads_only_its_own_tenants_inputs(
    connector_path: ConnectorPath,
    instance_cfg: InstanceConfig,
    subjects: Subjects,
    tenant: str,
    run_tag: str,
) -> None:
    """The other tenant's member shares the address and is never bound here: an
    unscoped seed would have filed that account under this tenant too."""
    shared = _seed_shared_address(connector_path, tenant, run_tag)
    subjects.publish()

    assert _assignments(instance_cfg, [shared.own_id, shared.other_id]) == {
        str(shared.own_id): tenant
    }
    assert set(_map_persons(instance_cfg, shared.email)) == {tenant}, (
        "the address resolves in this tenant alone; the other tenant was never seeded"
    )


def test_an_address_two_tenants_share_resolves_to_each_tenants_own_person(
    connector_path: ConnectorPath,
    instance_cfg: InstanceConfig,
    subjects: Subjects,
    tenant: str,
    run_tag: str,
) -> None:
    """Each tenant binds its own member: the address is one claim per tenant, so
    neither tenant's claim contests the other's and each resolves to its own person."""
    shared = _seed_shared_address(connector_path, tenant, run_tag)
    subjects.publish()
    other_person = str(uuid.uuid4())
    _bind_in_journal_mirror(instance_cfg, shared, other_person)

    persons = _map_persons(instance_cfg, shared.email)

    assert persons.get(OTHER_TENANT) == other_person, (
        f"the other tenant resolves to its own person: {persons}"
    )
    assert persons.get(tenant) not in (None, other_person), (
        f"this tenant resolves to its own person: {persons}"
    )
