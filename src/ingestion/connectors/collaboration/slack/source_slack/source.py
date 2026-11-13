"""Slack member analytics — Airbyte source connector entry point.

Python CDK rather than a declarative manifest: Slack refuses a day with HTTP 200
and `{"ok": false}`, while a served day is a streamed gzip file. A manifest error
handler can only see the body by reading it, which consumes the stream the gzip
decoder reads next, so a declarative connector either stores the refusal as an
empty record or loses every served day. Here the body is read once and decides.

Each check and each sync first asks Slack for the plan and the token's scopes,
and reads only the streams these permit (`source_slack.plan`).
"""

from __future__ import annotations

import json
import logging
import sys
from collections.abc import Iterator, Mapping
from datetime import date
from pathlib import Path
from typing import Any

import requests
from airbyte_cdk.models import (
    AirbyteConnectionStatus,
    AirbyteMessage,
    AirbyteStateMessage,
    ConfiguredAirbyteCatalog,
    ConnectorSpecification,
    FailureType,
    Status,
)
from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream
from airbyte_cdk.utils.traced_exception import AirbyteTracedException

from source_slack.client import SlackApiError, SlackClient, SlackResponseError
from source_slack.plan import BILLING_SCOPE, PLAN_NAMES, Access, access
from source_slack.streams import CONFIG_ERRORS, UsersDetails

logger = logging.getLogger("airbyte")

_PACKAGE = Path(__file__).parent

REQUIRED_CONFIG_FIELDS = ("insight_tenant_id", "insight_source_id", "slack_bot_token", "slack_start_date")


def load_stream_schema(name: str) -> Mapping[str, Any]:
    return json.loads((_PACKAGE / "schemas" / f"{name}.json").read_text(encoding="utf-8"))


def _client(config: Mapping[str, Any]) -> SlackClient:
    return SlackClient(config["slack_bot_token"])


def _streams(config: Mapping[str, Any]) -> list[UsersDetails]:
    return [UsersDetails(config, _client(config))]


def _access(config: Mapping[str, Any]) -> Access:
    """The plan and the streams to read, or the failure that stops the sync."""
    try:
        billing = _client(config).billing()
    except SlackApiError as error:
        message = str(error)
        if error.code == "missing_scope":
            message += f" (the token needs {BILLING_SCOPE})"
        kind = FailureType.config_error if error.code in CONFIG_ERRORS else FailureType.system_error
        raise AirbyteTracedException(message=message, internal_message=message, failure_type=kind) from error
    except SlackResponseError as error:
        raise AirbyteTracedException(
            message=f"Unexpected team.billing.info answer: {error}",
            internal_message=str(error),
            failure_type=FailureType.system_error,
        ) from error
    except requests.RequestException as error:
        raise AirbyteTracedException(
            message=f"team.billing.info kept failing: {type(error).__name__}",
            internal_message=str(error),
            failure_type=FailureType.transient_error,
        ) from error
    found = access(billing)
    for name, scopes in found.lacking_scopes.items():
        logger.warning("Skipping stream %s: the Slack token lacks scope %s", name, ", ".join(scopes))
    if not found.has_main:
        logger.warning("No main stream for the %s plan: member activity is not read", PLAN_NAMES[found.plan])
    return found


class SourceSlack(AbstractSource):
    def spec(self, logger: Any) -> ConnectorSpecification:
        return ConnectorSpecification(**json.loads((_PACKAGE / "spec.json").read_text(encoding="utf-8")))

    def check(self, logger: Any, config: Mapping[str, Any]) -> AirbyteConnectionStatus:
        ok, message = self.check_connection(logger, config)
        return AirbyteConnectionStatus(status=Status.SUCCEEDED if ok else Status.FAILED, message=message)

    def check_connection(self, logger: Any, config: Mapping[str, Any]) -> tuple[bool, Any | None]:
        """Pass with a report of the plan, the streams to read and the missing scopes."""
        for field in REQUIRED_CONFIG_FIELDS:
            if not str(config.get(field) or "").strip():
                return False, f"{field} must be set"
        try:
            date.fromisoformat(config["slack_start_date"])
        except ValueError:
            return False, f"slack_start_date must be YYYY-MM-DD, got {config['slack_start_date']!r}"
        try:
            found = _access(config)
        except AirbyteTracedException as error:
            if error.failure_type != FailureType.config_error:
                raise
            return False, error.message
        for stream in _streams(config):
            if stream.name in found.streams and (problem := stream.probe()):
                return False, problem
        return True, found.report()

    def streams(self, config: Mapping[str, Any]) -> list[Stream]:
        return list(_streams(config))

    def read(
        self,
        logger: logging.Logger,
        config: Mapping[str, Any],
        catalog: ConfiguredAirbyteCatalog,
        state: list[AirbyteStateMessage] | None = None,
    ) -> Iterator[AirbyteMessage]:
        found = _access(config)
        logger.info(found.report())
        selected = [s for s in catalog.streams if s.stream.name in found.streams]
        for configured in catalog.streams:
            name = configured.stream.name
            if name not in found.streams and name not in found.lacking_scopes:
                logger.info("Not reading stream %s on the %s plan", name, PLAN_NAMES[found.plan])
        yield from super().read(logger, config, ConfiguredAirbyteCatalog(streams=selected), state)


def main() -> None:
    from airbyte_cdk.entrypoint import launch

    launch(SourceSlack(), sys.argv[1:])
