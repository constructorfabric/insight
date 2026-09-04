"""Builders for the three bronze inputs the Jira field-history chain reads.

A test declares only what it is about: the field's catalogue row, the issue's
current JSON, and the changelog items. Everything Airbyte adds — raw ids,
extraction stamps, the columns no Jira model reads — is filled in here, so a
scenario stays readable as a table.

Column sets mirror `scripts/connectors-ddl/jira.sql`, the snapshot CI keeps in
lock-step with the real connectors.
"""

from __future__ import annotations

import json
import uuid
from typing import Any

SOURCE_ID = "jira-transform-test"
TENANT_ID = "11111111-1111-1111-1111-111111111111"

# Every fixture issue is created at this instant, so a synthetic_initial row's
# expected `event_at` is one constant rather than a per-test literal.
CREATED_AT = "2026-01-05T09:00:00"
# When the issue row was read. Later than every fixture event, so the round trip
# compares rather than skipping the pair, and it is the stamp a `retired_field`
# row carries.
OBSERVED_AT = "2026-03-01T12:00:00"


# A second sync of the same entity: Airbyte appends a new row rather than
# editing, and the later extraction mark is what makes it the current one.
LATER_SYNC = "2026-04-01T12:00:00"


def _airbyte(extracted_at: str) -> dict[str, Any]:
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": extracted_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
    }


def field(
    field_id: str,
    *,
    name: str | None = None,
    schema_type: str = "",
    schema_items: str = "",
    schema_custom: str = "",
    extracted_at: str = OBSERVED_AT,
) -> dict[str, Any]:
    """One row of the field catalogue — the classifier's only input."""
    return {
        **_airbyte(extracted_at),
        "id": field_id,
        "key": field_id,
        "name": name or field_id,
        "custom": schema_custom != "",
        "tenant_id": TENANT_ID,
        "source_id": SOURCE_ID,
        "unique_key": f"{SOURCE_ID}-{field_id}",
        "field_id": field_id,
        "schema_type": schema_type,
        "schema_items": schema_items,
        "schema_custom": schema_custom,
        "collected_at": extracted_at,
    }


def issue(
    key: str,
    *,
    fields: dict[str, Any] | None = None,
    created: str = CREATED_AT,
    reporter_id: str = "reporter-acct",
    extracted_at: str = OBSERVED_AT,
) -> dict[str, Any]:
    """The issue's current state. `fields` becomes `custom_fields_json`.

    A key ABSENT from `fields` and a key present with `None` are different
    states and must stay so — the first says the field is not in this issue's
    context, the second that it applies and is unset.
    """
    return {
        **_airbyte(extracted_at),
        "id": key,
        "key": key,
        "tenant_id": TENANT_ID,
        "source_id": SOURCE_ID,
        "unique_key": f"{SOURCE_ID}-{key}",
        "jira_id": key,
        "id_readable": key,
        "project_key": key.split("-")[0],
        "reporter_id": reporter_id,
        "created": created,
        "updated": created,
        "custom_fields_json": json.dumps(fields if fields is not None else {}, sort_keys=True),
        "collected_at": extracted_at,
    }


def item(
    field_id: str,
    *,
    frm: str | None = None,
    frm_str: str | None = None,
    to: str | None = None,
    to_str: str | None = None,
) -> dict[str, Any]:
    """One changelog item: the four sides Jira gives, named as Jira names them.

    `frm`/`to` are the id side, `frm_str`/`to_str` the rendered side. Which of
    them a field populates is a property of the field, and reproducing that
    faithfully is the whole point of these fixtures.
    """
    return {"field": field_id, "fieldId": field_id, "from": frm, "fromString": frm_str, "to": to, "toString": to_str}


def event(
    key: str,
    changelog_id: int,
    at: str,
    items: list[dict[str, Any]],
    *,
    author: str = "author-acct",
    extracted_at: str = OBSERVED_AT,
) -> dict[str, Any]:
    """One changelog entry for an issue, carrying one or more items."""
    return {
        **_airbyte(extracted_at),
        "id": str(changelog_id),
        "tenant_id": TENANT_ID,
        "source_id": SOURCE_ID,
        "unique_key": f"{SOURCE_ID}-{changelog_id}",
        "id_readable": key,
        "author_account_id": author,
        "changelog_id": changelog_id,
        "created_at": at,
        "items": json.dumps(items),
        "collected_at": extracted_at,
    }
