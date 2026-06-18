"""Per-user daily Copilot metrics — incremental, two-step signed-URL fetch + NDJSON.

Endpoint: GET /orgs/{org}/copilot/metrics/reports/users-1-day?day=YYYY-MM-DD

Step 1 returns: { "download_links": ["https://..."], "report_day": "YYYY-MM-DD" }
  NOTE: download_links is an array of plain strings (not {"url": "..."} objects).
  base.py handles both formats.
Step 2 (per signed URL): NDJSON, one JSON object per line — each = one user's day.

Day boundaries:
  - Cursor field: `day` (ISO YYYY-MM-DD), step P1D (one API call per day).
  - First-run start: `github_start_date` config (default 90 days ago).
  - End: yesterday UTC (data for day D available ~24h after end-of-day D).
  - Data availability: API has data from 2025-10-10 onwards; earlier dates → HTTP 204.

Source-native field names per `cpt-insightspec-principle-ghcopilot-source-native-schema`:
NDJSON object includes `user_login` (NOT `login`), `loc_added_sum`,
`code_acceptance_activity_count`, `user_initiated_interaction_count`,
`used_chat`, `used_agent`, `used_cli`. We pass these through unchanged.

Cursor advancement (Major #5 fix): the stream uses Airbyte's IncrementalMixin
so the cursor advances even on HTTP 204 days (no records). Without this, every
sync run would re-process all ~80 historical days that pre-date 2025-10-10
(when API data starts) — burning rate limit on guaranteed-empty fetches.
"""

import logging
from datetime import date, timedelta
from typing import Any, Iterable, List, Mapping, MutableMapping, Optional

import requests
from airbyte_cdk.sources.streams import IncrementalMixin

from source_github_copilot.streams.base import CopilotReportsStream, yesterday_utc

logger = logging.getLogger("airbyte")


class CopilotUserMetricsStream(CopilotReportsStream, IncrementalMixin):
    """Incremental per-user daily metrics."""

    name = "copilot_user_metrics"
    cursor_field = "day"

    def __init__(
        self,
        start_date: Optional[str] = None,
        lookback_days: int = 7,
        **kwargs,
    ):
        super().__init__(**kwargs)
        self._start_date = start_date or self._default_start_date()
        self._lookback_days = max(int(lookback_days), 0)
        self._state: Mapping[str, Any] = {}

    @property
    def state(self) -> Mapping[str, Any]:
        return self._state

    @state.setter
    def state(self, value: Mapping[str, Any]):
        self._state = value or {}

    @staticmethod
    def _default_start_date() -> str:
        from datetime import datetime, timezone
        return (datetime.now(timezone.utc) - timedelta(days=90)).strftime("%Y-%m-%d")

    def path(self, **kwargs) -> str:
        return f"orgs/{self._org}/copilot/metrics/reports/users-1-day"

    def request_params(
        self,
        stream_slice: Optional[Mapping[str, Any]] = None,
        **kwargs,
    ) -> MutableMapping[str, Any]:
        day = (stream_slice or {}).get("day")
        if not day:
            raise ValueError("CopilotUserMetricsStream requires `day` in stream_slice")
        return {"day": day}

    def stream_slices(
        self,
        sync_mode=None,
        cursor_field=None,
        stream_state: Optional[Mapping[str, Any]] = None,
        **kwargs,
    ) -> Iterable[Optional[Mapping[str, Any]]]:
        """Yield one slice per day from cursor (or start_date) through yesterday UTC."""
        # Prefer the in-memory state (advanced per-slice in read_records below) over the
        # stream_state argument, so cursor moves forward even when previous slices yielded
        # zero records (HTTP 204 days).
        cursor = (self._state or {}).get(self.cursor_field) or (stream_state or {}).get(self.cursor_field)
        if cursor:
            # Re-fetch a trailing lookback window instead of cursor+1: Copilot daily
            # reports lag (often >24-48h; weekends later) and can be restated. Starting
            # at cursor+1 permanently skips a day that returned 204 because its report
            # wasn't ready yet (the cursor still advances past it). Re-querying the last
            # `lookback_days` keeps the recent tail self-healing; RMT dedup on unique_key
            # makes the re-delivery idempotent. (cf. Zendesk lookback_window / Cursor resync.)
            lb = (date.fromisoformat(cursor) - timedelta(days=self._lookback_days)).isoformat()
            start = max(lb, self._start_date)  # ISO dates compare chronologically as strings
        else:
            start = self._start_date
        end = yesterday_utc()  # inclusive

        try:
            current = date.fromisoformat(start)
            stop = date.fromisoformat(end)
        except ValueError as e:
            logger.error(f"Invalid date in stream_slices (start={start}, end={end}): {e}")
            return

        while current <= stop:
            yield {"day": current.isoformat()}
            current += timedelta(days=1)

    def _record_pk_parts(self, record: dict, day: str) -> List[str]:
        """unique_key composition: {tenant}-{source}-{user_login}-{day}."""
        user_login = record.get("user_login") or ""
        return [user_login, day]

    def _filter_record(self, record: dict) -> bool:
        """Drop records without identity (cannot resolve to a person)."""
        return bool(record.get("user_login"))

    def parse_response(
        self,
        response: requests.Response,
        stream_slice=None,
        **kwargs,
    ) -> Iterable[Mapping[str, Any]]:
        """Override to inject `day` into each record (NDJSON doesn't always carry it)."""
        day = (stream_slice or {}).get("day", "")
        for record in super().parse_response(response, stream_slice=stream_slice, **kwargs):
            # Ensure `day` field is present and matches the requested slice
            if not record.get("day"):
                record = dict(record)
                record["day"] = day
            yield record

    def read_records(
        self,
        sync_mode,
        cursor_field=None,
        stream_slice: Optional[Mapping[str, Any]] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[Mapping[str, Any]]:
        """Wrap parent read_records to advance state per-slice — even when no records emitted.

        This is the Major #5 fix: HTTP 204 days (e.g., dates before 2025-10-10 when API
        data wasn't yet available) emit zero records but should still advance the cursor.
        Without this, every subsequent run re-fetches all empty historical days.
        """
        yielded_at_least_one = False
        for record in super().read_records(
            sync_mode=sync_mode,
            cursor_field=cursor_field,
            stream_slice=stream_slice,
            stream_state=stream_state,
        ):
            yielded_at_least_one = True
            yield record
        # Always advance state to the slice's day, regardless of whether records were yielded.
        if stream_slice and stream_slice.get("day"):
            slice_day = stream_slice["day"]
            current_max = (self._state or {}).get(self.cursor_field, "")
            if slice_day > current_max:
                self._state = {self.cursor_field: slice_day}

    def get_json_schema(self) -> Mapping[str, Any]:
        """JSON Schema for `bronze_github_copilot.copilot_user_metrics`.

        Field set confirmed against live constructor-tech-org API (2026-04-29).
        """
        return {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": True,
            "properties": {
                # framework-injected
                "tenant_id": {"type": "string"},
                "source_id": {"type": "string"},
                "unique_key": {"type": "string"},
                "data_source": {"type": "string"},
                "collected_at": {"type": "string"},
                # grain / identity
                "day": {"type": "string"},
                "user_login": {"type": "string"},
                "user_id": {"type": ["null", "number"]},
                "organization_id": {"type": ["null", "string"]},
                "enterprise_id": {"type": ["null", "string"]},
                # code-generation metrics
                "code_generation_activity_count": {"type": ["null", "number"]},
                "code_acceptance_activity_count": {"type": ["null", "number"]},
                "loc_suggested_to_add_sum": {"type": ["null", "number"]},
                "loc_suggested_to_delete_sum": {"type": ["null", "number"]},
                "loc_added_sum": {"type": ["null", "number"]},
                "loc_deleted_sum": {"type": ["null", "number"]},
                # interaction metrics
                "user_initiated_interaction_count": {"type": ["null", "number"]},
                # feature-usage flags
                "used_chat": {"type": ["null", "boolean"]},
                "used_agent": {"type": ["null", "boolean"]},
                "used_cli": {"type": ["null", "boolean"]},
                "used_copilot_coding_agent": {"type": ["null", "boolean"]},
                "used_copilot_cloud_agent": {"type": ["null", "boolean"]},
                # breakdown arrays (passthrough; schema varies by GitHub API version)
                "totals_by_ide": {"type": ["null", "array"]},
                "totals_by_feature": {"type": ["null", "array"]},
                "totals_by_language_feature": {"type": ["null", "array"]},
                "totals_by_language_model": {"type": ["null", "array"]},
                "totals_by_model_feature": {"type": ["null", "array"]},
            },
        }
