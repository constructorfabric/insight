"""Org-level daily Copilot metrics — incremental, two-step signed-URL fetch + NDJSON.

Endpoint: GET /orgs/{org}/copilot/metrics/reports/organization-1-day?day=YYYY-MM-DD

Same pattern as user_metrics but the NDJSON contains org-wide aggregates instead
of per-user rows. Envelope returns 1 download_link (plain string URL), and each
NDJSON file contains a single line with the org's daily totals.

Field set confirmed against live constructor-tech-org API (2026-04-29). JSON Schema
uses additionalProperties=true so new API fields pass through without migration.

Cursor advancement (Major #5 fix): same IncrementalMixin pattern as
user_metrics — state advances per-slice so HTTP 204 days don't get re-fetched
on every sync.
"""

import logging
from datetime import date, timedelta
from typing import Any, Iterable, List, Mapping, MutableMapping, Optional

import requests
from airbyte_cdk.sources.streams import IncrementalMixin

from source_github_copilot.streams.base import CopilotReportsStream, yesterday_utc

logger = logging.getLogger("airbyte")


class CopilotOrgMetricsStream(CopilotReportsStream, IncrementalMixin):
    """Incremental org-level daily metrics."""

    name = "copilot_org_metrics"
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
        return f"orgs/{self._org}/copilot/metrics/reports/organization-1-day"

    def request_params(
        self,
        stream_slice: Optional[Mapping[str, Any]] = None,
        **kwargs,
    ) -> MutableMapping[str, Any]:
        day = (stream_slice or {}).get("day")
        if not day:
            raise ValueError("CopilotOrgMetricsStream requires `day` in stream_slice")
        return {"day": day}

    def stream_slices(
        self,
        sync_mode=None,
        cursor_field=None,
        stream_state: Optional[Mapping[str, Any]] = None,
        **kwargs,
    ) -> Iterable[Optional[Mapping[str, Any]]]:
        # Prefer in-memory state (advanced per-slice in read_records below) over
        # stream_state argument — see Major #5 fix in user_metrics.py.
        cursor = (self._state or {}).get(self.cursor_field) or (stream_state or {}).get(self.cursor_field)
        if cursor:
            # Re-fetch a trailing lookback window instead of starting at cursor+1.
            # Copilot daily reports lag (often >24-48h; weekends later) and can be
            # restated. Starting at cursor+1 would PERMANENTLY skip a day that
            # returned 204 because its report wasn't ready yet (the cursor still
            # advances past it). Re-querying the last `lookback_days` keeps the
            # recent tail self-healing; RMT dedup on unique_key makes the
            # re-delivery idempotent. (cf. Zendesk lookback_window / Cursor resync.)
            lb = (date.fromisoformat(cursor) - timedelta(days=self._lookback_days)).isoformat()
            start = max(lb, self._start_date)  # ISO dates compare chronologically as strings
        else:
            start = self._start_date
        end = yesterday_utc()

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
        """unique_key composition: {tenant}-{source}-{day}.

        Org metrics have no user dimension; tenant + source + day uniquely identify
        the row. The tenant-source prefix makes multi-org tenants collision-safe
        by construction (different `source_id` values per Copilot connection).
        """
        return [day]

    def parse_response(
        self,
        response: requests.Response,
        stream_slice=None,
        **kwargs,
    ) -> Iterable[Mapping[str, Any]]:
        day = (stream_slice or {}).get("day", "")
        for record in super().parse_response(response, stream_slice=stream_slice, **kwargs):
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
        """Advance state per-slice — see user_metrics.py for Major #5 rationale."""
        for record in super().read_records(
            sync_mode=sync_mode,
            cursor_field=cursor_field,
            stream_slice=stream_slice,
            stream_state=stream_state,
        ):
            yield record
        if stream_slice and stream_slice.get("day"):
            slice_day = stream_slice["day"]
            current_max = (self._state or {}).get(self.cursor_field, "")
            if slice_day > current_max:
                self._state = {self.cursor_field: slice_day}

    def get_json_schema(self) -> Mapping[str, Any]:
        """JSON Schema for `bronze_github_copilot.copilot_org_metrics`.

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
                "organization_id": {"type": ["null", "string"]},
                "enterprise_id": {"type": ["null", "string"]},
                # active-user aggregates
                "daily_active_users": {"type": ["null", "number"]},
                "weekly_active_users": {"type": ["null", "number"]},
                "monthly_active_users": {"type": ["null", "number"]},
                "monthly_active_chat_users": {"type": ["null", "number"]},
                "monthly_active_agent_users": {"type": ["null", "number"]},
                "daily_active_copilot_cloud_agent_users": {"type": ["null", "number"]},
                "weekly_active_copilot_cloud_agent_users": {"type": ["null", "number"]},
                "monthly_active_copilot_cloud_agent_users": {"type": ["null", "number"]},
                "daily_active_copilot_code_review_users": {"type": ["null", "number"]},
                "weekly_active_copilot_code_review_users": {"type": ["null", "number"]},
                "monthly_active_copilot_code_review_users": {"type": ["null", "number"]},
                "daily_passive_copilot_code_review_users": {"type": ["null", "number"]},
                "weekly_passive_copilot_code_review_users": {"type": ["null", "number"]},
                "monthly_passive_copilot_code_review_users": {"type": ["null", "number"]},
                # interaction / code metrics
                "user_initiated_interaction_count": {"type": ["null", "number"]},
                "code_generation_activity_count": {"type": ["null", "number"]},
                "code_acceptance_activity_count": {"type": ["null", "number"]},
                "loc_suggested_to_add_sum": {"type": ["null", "number"]},
                "loc_suggested_to_delete_sum": {"type": ["null", "number"]},
                "loc_added_sum": {"type": ["null", "number"]},
                "loc_deleted_sum": {"type": ["null", "number"]},
                # pull-request metrics object
                "pull_requests": {"type": ["null", "object"]},
                # breakdown arrays (passthrough; schema varies by GitHub API version)
                "totals_by_ide": {"type": ["null", "array"]},
                "totals_by_feature": {"type": ["null", "array"]},
                "totals_by_language_feature": {"type": ["null", "array"]},
                "totals_by_language_model": {"type": ["null", "array"]},
                "totals_by_model_feature": {"type": ["null", "array"]},
            },
        }
