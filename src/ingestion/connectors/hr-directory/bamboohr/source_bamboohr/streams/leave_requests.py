from __future__ import annotations

import logging
from collections.abc import Iterable, Mapping
from datetime import datetime, timezone
from typing import Any

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams import Stream

from source_bamboohr.client import BambooClient

logger = logging.getLogger("airbyte")

DEFAULT_START_DATE = "2020-01-01"

NULLABLE_STR: Mapping[str, Any] = {"type": ["string", "null"]}
NULLABLE_BOOL: Mapping[str, Any] = {"type": ["boolean", "null"]}

SCHEMA: Mapping[str, Any] = {
    "$schema": "http://json-schema.org/schema#",
    "type": "object",
    "additionalProperties": True,
    "required": ["unique_key"],
    "properties": {
        "type": {
            "type": ["object", "null"],
            "properties": {"id": NULLABLE_STR, "icon": NULLABLE_STR, "name": NULLABLE_STR},
        },
        "id": NULLABLE_STR,
        "end": NULLABLE_STR,
        "name": NULLABLE_STR,
        "dates": {"type": ["object", "null"], "additionalProperties": True},
        "notes": {
            "type": ["object", "null"],
            "properties": {"manager": NULLABLE_STR, "employee": NULLABLE_STR},
        },
        "start": NULLABLE_STR,
        "amount": {
            "type": ["object", "null"],
            "properties": {"unit": NULLABLE_STR, "amount": NULLABLE_STR},
        },
        "status": {
            "type": ["object", "null"],
            "properties": {
                "status": NULLABLE_STR,
                "lastChanged": NULLABLE_STR,
                "lastChangedByUserId": NULLABLE_STR,
            },
        },
        "actions": {
            "type": ["object", "null"],
            "properties": {
                "deny": NULLABLE_BOOL,
                "edit": NULLABLE_BOOL,
                "view": NULLABLE_BOOL,
                "bypass": NULLABLE_BOOL,
                "cancel": NULLABLE_BOOL,
                "approve": NULLABLE_BOOL,
            },
        },
        "created": NULLABLE_STR,
        "source_id": NULLABLE_STR,
        "tenant_id": NULLABLE_STR,
        "employeeId": NULLABLE_STR,
        "unique_key": {"type": "string"},
    },
}


class LeaveRequestsStream(Stream):
    name = "leave_requests"
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
        end = datetime.now(timezone.utc).strftime("%Y-%m-%d")
        rows = self._client.get("time_off/requests", params={"start": self._start_date, "end": end})
        if not isinstance(rows, list):
            raise RuntimeError(f"BambooHR time-off response is not a list: {type(rows).__name__}")

        count = 0
        for row in rows:
            if not isinstance(row, Mapping):
                logger.warning("Skipping BambooHR leave request that is not an object")
                continue

            request_id = row.get("id")
            if request_id is None or str(request_id).strip() == "":
                logger.warning("Skipping BambooHR leave request without an id")
                continue

            count += 1
            yield {
                **row,
                "tenant_id": self._tenant_id,
                "source_id": self._source_id,
                "unique_key": f"{self._tenant_id}-{self._source_id}-{request_id}",
            }

        logger.info("BambooHR leave_requests stream emitted %d records", count)

    def get_json_schema(self) -> Mapping[str, Any]:
        return SCHEMA
