"""Slack member analytics — Airbyte source connector entry point.

Python CDK rather than a declarative manifest: Slack refuses a day with HTTP 200
and `{"ok": false}`, while a served day is a streamed gzip file. A manifest error
handler can only see the body by reading it, which consumes the stream the gzip
decoder reads next, so a declarative connector either stores the refusal as an
empty record or loses every served day. Here the body is read once and decides.
"""

from __future__ import annotations

import json
import logging
import sys
from collections.abc import Mapping
from datetime import date
from pathlib import Path
from typing import Any

from airbyte_cdk.models import ConnectorSpecification
from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream

from source_slack.client import SlackAnalyticsClient, SlackApiError, SlackResponseError
from source_slack.streams import NOT_YET_AVAILABLE, PUBLICATION_DELAY, UsersDetails, today

logger = logging.getLogger("airbyte")

_PACKAGE = Path(__file__).parent

REQUIRED_CONFIG_FIELDS = ("insight_tenant_id", "insight_source_id", "slack_bot_token", "slack_start_date")


def load_stream_schema(name: str) -> Mapping[str, Any]:
    return json.loads((_PACKAGE / "schemas" / f"{name}.json").read_text(encoding="utf-8"))


def _client(config: Mapping[str, Any]) -> SlackAnalyticsClient:
    return SlackAnalyticsClient(config["slack_bot_token"])


class SourceSlack(AbstractSource):
    def spec(self, logger: Any) -> ConnectorSpecification:
        return ConnectorSpecification(**json.loads((_PACKAGE / "spec.json").read_text(encoding="utf-8")))

    def check_connection(self, logger: Any, config: Mapping[str, Any]) -> tuple[bool, Any | None]:
        for field in REQUIRED_CONFIG_FIELDS:
            if not str(config.get(field) or "").strip():
                return False, f"{field} must be set"
        try:
            date.fromisoformat(config["slack_start_date"])
        except ValueError:
            return False, f"slack_start_date must be YYYY-MM-DD, got {config['slack_start_date']!r}"
        day = (today() - PUBLICATION_DELAY).isoformat()
        try:
            rows = _client(config).member_day(day)
            next(rows, None)
            rows.close()
        except SlackApiError as error:
            # Slack only says a day is not ready after accepting the token and its scope.
            if error.code == NOT_YET_AVAILABLE:
                return True, None
            return False, str(error)
        except (SlackResponseError, OSError) as error:
            return False, f"admin.analytics.getFile failed: {error}"
        return True, None

    def streams(self, config: Mapping[str, Any]) -> list[Stream]:
        return [UsersDetails(config, _client(config))]


def main() -> None:
    from airbyte_cdk.entrypoint import launch

    launch(SourceSlack(), sys.argv[1:])
