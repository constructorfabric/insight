from __future__ import annotations

import logging
from collections.abc import Iterable, Mapping
from typing import Any

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams import Stream

from source_bamboohr.client import BambooClient

logger = logging.getLogger("airbyte")

NULLABLE_STR: Mapping[str, Any] = {"type": ["string", "null"]}
NULLABLE_ID: Mapping[str, Any] = {"type": ["number", "string", "null"]}

SCHEMA: Mapping[str, Any] = {
    "$schema": "http://json-schema.org/schema#",
    "type": "object",
    "additionalProperties": True,
    "required": ["unique_key"],
    "properties": {
        "type": NULLABLE_STR,
        "id": NULLABLE_ID,
        "name": NULLABLE_STR,
        "alias": NULLABLE_STR,
        "unique": NULLABLE_ID,
        "source_id": NULLABLE_STR,
        "tenant_id": NULLABLE_STR,
        "deprecated": {"type": ["boolean", "null"]},
        "unique_key": {"type": "string"},
    },
}


def field_key(field: Mapping[str, Any]) -> str:
    """Natural key of a field-metadata row: a deprecated field may carry the id of
    an active one, so the prefix is what keeps the two keys apart."""
    field_id = field["id"]
    return f"d{field_id}" if field.get("deprecated") else str(field_id)


class MetaFieldsStream(Stream):
    name = "meta_fields"
    primary_key = "unique_key"

    def __init__(self, client: BambooClient, tenant_id: str, source_id: str) -> None:
        self._client = client
        self._tenant_id = tenant_id
        self._source_id = source_id

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any]]:
        rows = self._client.get("meta/fields")
        if not isinstance(rows, list):
            raise RuntimeError(f"BambooHR field metadata is not a list: {type(rows).__name__}")

        count = 0
        for row in rows:
            if not isinstance(row, Mapping):
                logger.warning("Skipping BambooHR field metadata entry that is not an object")
                continue

            if row.get("id") is None or str(row["id"]).strip() == "":
                logger.warning("Skipping BambooHR field metadata entry without an id")
                continue

            unique = field_key(row)
            count += 1
            yield {
                **row,
                "unique": unique,
                "tenant_id": self._tenant_id,
                "source_id": self._source_id,
                "unique_key": f"{self._tenant_id}-{self._source_id}-{unique}",
            }

        logger.info("BambooHR meta_fields stream emitted %d records", count)

    def get_json_schema(self) -> Mapping[str, Any]:
        return SCHEMA
