from __future__ import annotations

import logging
from collections.abc import Iterable, Mapping
from datetime import UTC, datetime
from typing import Any

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams import Stream

from source_bamboohr.client import BambooClient, BambooHrApiError

logger = logging.getLogger("airbyte")

NULLABLE_STR = {"type": ["string", "null"]}
NULLABLE_ID = {"type": ["integer", "string", "null"]}
SCHEMA: Mapping[str, Any] = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "additionalProperties": True,
    "required": ["unique_key"],
    "properties": {
        "id": NULLABLE_ID,
        "type": NULLABLE_STR,
        "employeeId": NULLABLE_ID,
        "name": NULLABLE_STR,
        "start": NULLABLE_STR,
        "end": NULLABLE_STR,
        "tenant_id": NULLABLE_STR,
        "source_id": NULLABLE_STR,
        "unique_key": {"type": "string"},
    },
}


class WhosOutStream(Stream):
    name = "whos_out"
    primary_key = "unique_key"

    def __init__(self, client: BambooClient, tenant_id: str, source_id: str, start_date: str) -> None:
        self._client = client
        self._tenant_id = tenant_id
        self._source_id = source_id
        self._start_date = start_date

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any]]:
        end = datetime.now(UTC).date().isoformat()
        try:
            rows = self._client.get(
                "time_off/whos_out", params={"start": self._start_date, "end": end, "filter": "off"}
            )
        except BambooHrApiError as exc:
            if exc.status_code != 403:
                raise
            logger.warning(
                "Skipping BambooHR whos_out: HTTP 403 (feature disabled or access denied). "
                "Availability data is unavailable; other streams continue."
            )
            return

        if not isinstance(rows, list):
            raise TypeError(f"BambooHR whos_out response is not a list: {type(rows).__name__}")

        count = 0
        for row in rows:
            if not isinstance(row, Mapping):
                logger.warning("Skipping BambooHR whos_out entry that is not an object")
                continue

            key_parts = [row.get(field) for field in ("type", "id", "start")]
            if any(part is None or not str(part).strip() for part in key_parts):
                logger.warning("Skipping BambooHR whos_out entry without type, id, or start")
                continue

            occurrence_key = "-".join(str(part) for part in key_parts)
            count += 1
            yield {
                **row,
                "tenant_id": self._tenant_id,
                "source_id": self._source_id,
                "unique_key": f"{self._tenant_id}-{self._source_id}-{occurrence_key}",
            }

        logger.info("BambooHR whos_out stream emitted %d records", count)

    def get_json_schema(self) -> Mapping[str, Any]:
        return SCHEMA
