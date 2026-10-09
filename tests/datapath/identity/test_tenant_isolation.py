"""Another tenant's accounts never become this tenant's people (#3661).

Both tenants' identity inputs share one warehouse. Seeding one tenant reads only
its own inputs, and the email map resolves an address within each tenant on its
own — an address both tenants carry is two separate claims, not a contest.
"""

from __future__ import annotations

import pytest
from insight_datapath import clickhouse, records
from insight_datapath.connector_path import ConnectorPath
from insight_datapath.instance import InstanceConfig
from insight_datapath.subjects import Subjects

pytestmark = pytest.mark.fixture

ORG_MEMBERS = "bronze_github_directory.org_members"
OTHER_TENANT = "7a1e5c3b-9d2f-4b6a-8e0c-1f2a3b4c5d6e"


def _assignments(cfg: InstanceConfig, member_ids: list[int]) -> dict[str, str]:
    """The tenant each GitHub account is bound under, by account id."""
    ids = ", ".join(f"'{member_id}'" for member_id in member_ids)
    rows = clickhouse.query(
        cfg,
        "SELECT account_id, tenant_id FROM identity.account_assignment"
        f" WHERE source_type = 'github' AND account_id IN ({ids})",
    )
    return {str(account): str(tenant) for account, tenant in rows}


def _map_tenants(cfg: InstanceConfig, email: str) -> set[str]:
    rows = clickhouse.query(
        cfg, f"SELECT tenant_id FROM identity.person_map WHERE email = {clickhouse.literal(email)}"
    )
    return {str(tenant) for (tenant,) in rows}


def test_a_seed_reads_only_its_own_tenants_inputs(
    connector_path: ConnectorPath,
    instance_cfg: InstanceConfig,
    subjects: Subjects,
    tenant: str,
    run_tag: str,
) -> None:
    """The other tenant's member shares the address and is never bound here: an
    unscoped seed would have filed that account under this tenant too."""
    own_id = int(run_tag, 16)
    other_id = own_id + 1
    email = f"shared.{run_tag}@example.com"
    source_id = f"github-directory-{run_tag}"

    connector_path.build(
        {
            ORG_MEMBERS: [
                records.org_member(
                    tenant=tenant, source_id=source_id, login=f"own-{run_tag}",
                    member_id=own_id, email=email, name="Own Member",
                ),
                records.org_member(
                    tenant=OTHER_TENANT, source_id=source_id, login=f"other-{run_tag}",
                    member_id=other_id, email=email, name="Other Member",
                ),
            ]
        }
    )
    subjects.publish()

    assert _assignments(instance_cfg, [own_id, other_id]) == {str(own_id): tenant}
    assert _map_tenants(instance_cfg, email) == {tenant}, (
        "the address resolves in this tenant alone; the other tenant was never seeded"
    )
