from __future__ import annotations

import logging
from collections.abc import Iterable, Mapping
from typing import Annotated, Any

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams import Stream
from pydantic import AfterValidator, BaseModel, BeforeValidator, ConfigDict, ValidationError

from source_bamboohr.client import BambooClient
from source_bamboohr.parsing import ROWS, Row, blank_to_none, malformed, valid_rows

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


# No attribute of a metadata entry can fail it: a dropped entry would take its id
# out of the sensitive-key set and let a sensitive column through unredacted.
AnyId = Annotated[str | None, BeforeValidator(lambda v: None if v is None else str(v)), AfterValidator(blank_to_none)]
AnyText = Annotated[str, BeforeValidator(lambda v: str(v or ""))]
Truthy = Annotated[bool, BeforeValidator(bool)]


class MetaField(BaseModel):
    model_config = ConfigDict(frozen=True)

    id: AnyId = None
    alias: AnyText = ""
    deprecated: Truthy = False


def read_meta_fields(client: BambooClient) -> list[tuple[MetaField, Row]]:
    try:
        rows = ROWS.validate_json(client.get("meta/fields"))
    except ValidationError as exc:
        raise malformed("field metadata", exc) from None

    return list(valid_rows(MetaField, rows, "field metadata entry"))


def field_key(field_id: str, deprecated: bool) -> str:
    """Natural key of a field-metadata row: a deprecated field may carry the id of
    an active one, so the prefix is what keeps the two keys apart."""
    return f"d{field_id}" if deprecated else field_id


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
        count = 0
        for field, row in read_meta_fields(self._client):
            if field.id is None:
                logger.warning("Skipping BambooHR field metadata entry without an id")
                continue

            unique = field_key(field.id, field.deprecated)
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
