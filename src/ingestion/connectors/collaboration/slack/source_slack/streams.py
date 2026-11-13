"""`users_details`: one row per member per day from Slack's member analytics file.

Incremental on `date`, one request per day from the cursor day to today - 2 days
(Slack's publication delay). The cursor day is read again, as the declarative
connector did: its state `{"date": "YYYY-MM-DD"}` could name a day it had not
finished, and unique_key makes the second copy replace the first.
"""

from __future__ import annotations

import logging
from collections.abc import Iterable, Mapping, MutableMapping
from datetime import UTC, date, datetime, timedelta
from typing import Any

import requests
from airbyte_cdk.models import FailureType
from airbyte_cdk.sources.streams import CheckpointMixin, Stream
from airbyte_cdk.utils.traced_exception import AirbyteTracedException

from source_slack.client import SlackAnalyticsClient, SlackApiError, SlackResponseError

logger = logging.getLogger("airbyte")

CURSOR = "date"
PUBLICATION_DELAY = timedelta(days=2)
# A day Slack still calls not yet available this long after it ended is not late,
# it is missing: the sync fails instead of waiting for it forever.
NOT_YET_GRACE = timedelta(days=5)

# Refusals an operator fixes in the token, its scopes or the workspace plan.
CONFIG_ERRORS = frozenset(
    {
        "access_denied",
        "account_inactive",
        "ekm_access_denied",
        "enterprise_is_restricted",
        "feature_not_available",
        "invalid_auth",
        "member_analytics_disabled",
        "missing_scope",
        "no_permission",
        "not_allowed_token_type",
        "not_an_admin",
        "not_authed",
        "org_level_email_display_disabled",
        "token_expired",
        "team_access_not_granted",
        "token_revoked",
        "two_factor_setup_required",
    }
)
NOT_YET_AVAILABLE = "file_not_yet_available"


def today() -> date:
    return datetime.now(UTC).date()


def failure(error: SlackApiError) -> AirbyteTracedException:
    kind = FailureType.config_error if error.code in CONFIG_ERRORS else FailureType.system_error
    return AirbyteTracedException(message=str(error), internal_message=str(error), failure_type=kind)


def _cursor_from_state(value: Mapping[str, Any]) -> date | None:
    try:
        return date.fromisoformat(str(value.get(CURSOR))[:10])
    except ValueError:
        return None


class UsersDetails(Stream, CheckpointMixin):
    primary_key = "unique_key"
    cursor_field = CURSOR
    source_defined_cursor = True
    state_checkpoint_interval = None

    def __init__(self, config: Mapping[str, Any], client: SlackAnalyticsClient) -> None:
        self._client = client
        self._tenant_id = config["insight_tenant_id"]
        self._source_id = config["insight_source_id"]
        self._start = date.fromisoformat(config["slack_start_date"])
        self._cursor: date | None = None
        self._stopped = False

    @property
    def name(self) -> str:
        return "users_details"

    def get_json_schema(self) -> Mapping[str, Any]:
        from source_slack.source import load_stream_schema

        return load_stream_schema(self.name)

    @property
    def state(self) -> MutableMapping[str, Any]:
        return {CURSOR: self._cursor.isoformat()} if self._cursor else {}

    @state.setter
    def state(self, value: Mapping[str, Any]) -> None:
        self._cursor = _cursor_from_state(value) if isinstance(value, Mapping) else None

    def stream_slices(
        self, sync_mode: Any = None, cursor_field: Any = None, stream_state: Mapping[str, Any] | None = None
    ) -> Iterable[Mapping[str, Any] | None]:
        if stream_state:
            self.state = stream_state
        first = max(self._start, self._cursor) if self._cursor else self._start
        last = today() - PUBLICATION_DELAY
        day = first
        while day <= last:
            yield {CURSOR: day.isoformat()}
            day += timedelta(days=1)

    def read_records(
        self, sync_mode: Any, cursor_field: Any = None, stream_slice: Mapping[str, Any] | None = None, **kwargs: Any
    ) -> Iterable[Mapping[str, Any]]:
        if not stream_slice or self._stopped:
            return
        day = stream_slice[CURSOR]
        try:
            for row in self._client.member_day(day):
                yield self._stamp(row, day)
        except SlackApiError as error:
            if error.code == NOT_YET_AVAILABLE and date.fromisoformat(day) >= today() - NOT_YET_GRACE:
                # Later days cannot be ready either; the cursor stays on the last day read.
                logger.warning("%s; stopping at the last published day", error)
                self._stopped = True
                return
            raise failure(error) from error
        except SlackResponseError as error:
            raise AirbyteTracedException(
                message=f"Unexpected admin.analytics.getFile answer: {error}",
                internal_message=str(error),
                failure_type=FailureType.system_error,
            ) from error
        except requests.RequestException as error:
            raise AirbyteTracedException(
                message=f"admin.analytics.getFile for {day} kept failing: {type(error).__name__}",
                internal_message=str(error),
                failure_type=FailureType.transient_error,
            ) from error
        self._cursor = date.fromisoformat(day)

    def _stamp(self, row: Mapping[str, Any], day: str) -> dict[str, Any]:
        user_id = row.get("user_id")
        if not user_id:
            raise AirbyteTracedException(
                message=f"Slack member file for {day} has a row without user_id",
                internal_message=f"row keys: {sorted(row)}",
                failure_type=FailureType.system_error,
            )
        row_day = row.get(CURSOR) or day
        return {
            **row,
            CURSOR: row_day,
            "tenant_id": self._tenant_id,
            "source_id": self._source_id,
            "unique_key": f"{self._tenant_id}-{self._source_id}-{user_id}-{row_day}",
        }
