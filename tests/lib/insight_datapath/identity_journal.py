"""What the identity service's own journal holds, read where the service keeps it.

A correction is DB-shaped: what `INSERT IGNORE` accepted, which row is newest, who
authored it. The API reports the binding in force; the journal is the only place
the rule about what a decision appended can be stated, so the lane reads it directly.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass

from insight_datapath import mariadb
from insight_datapath.instance import InstanceConfig

IDENTITY_DATABASE = "identity"

#: The author persons-seed stamps; any other author is an operator's decision.
SYSTEM_AUTHOR = str(uuid.UUID(int=0))


@dataclass(frozen=True)
class Decision:
    """One `id` row of the journal: where the account pointed, and who said so."""

    person_id: str
    author_person_id: str
    reason: str


def _canonical(hex_uuid: str) -> str:
    return str(uuid.UUID(hex=hex_uuid))


def account_decisions(
    cfg: InstanceConfig, *, tenant: str, source_type: str, source_id: str, account_id: str
) -> list[Decision]:
    """Every decision the journal holds for one account, newest first."""
    rows = mariadb.query(
        cfg,
        """
        SELECT LOWER(HEX(person_id)), LOWER(HEX(author_person_id)), reason
        FROM persons
        WHERE value_type = 'id'
          AND insight_tenant_id = UNHEX(REPLACE(%s, '-', ''))
          AND insight_source_type = %s
          AND insight_source_id = UNHEX(REPLACE(%s, '-', ''))
          AND value_id = %s
        ORDER BY created_at DESC, id DESC
        """,
        (tenant, source_type, source_id, account_id),
        database=IDENTITY_DATABASE,
    )
    return [
        Decision(_canonical(str(person)), _canonical(str(author)), str(reason or ""))
        for person, author, reason in rows
    ]


def open_parent(cfg: InstanceConfig, *, tenant: str, child: str) -> str | None:
    """The manager the org chart currently names for `child`, or None at a root."""
    rows = mariadb.query(
        cfg,
        """
        SELECT LOWER(HEX(parent_person_id))
        FROM org_chart
        WHERE insight_tenant_id = UNHEX(REPLACE(%s, '-', ''))
          AND child_person_id = UNHEX(REPLACE(%s, '-', ''))
          AND valid_to IS NULL
        """,
        (tenant, child),
        database=IDENTITY_DATABASE,
    )
    if len(rows) > 1:
        raise AssertionError(f"{child} has {len(rows)} open parents; the chart is a tree")
    if not rows or rows[0][0] is None:
        return None
    return _canonical(str(rows[0][0]))
