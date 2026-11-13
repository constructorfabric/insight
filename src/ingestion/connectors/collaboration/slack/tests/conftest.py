"""Shared helpers: Slack's two answer shapes and a protocol read through the CDK entrypoint."""

from __future__ import annotations

import gzip
import json
from datetime import date
from pathlib import Path
from typing import Any

import pytest
from airbyte_cdk.models import (
    AirbyteStateBlob,
    AirbyteStateMessage,
    AirbyteStateType,
    AirbyteStreamState,
    ConfiguredAirbyteCatalog,
    StreamDescriptor,
    SyncMode,
)
from airbyte_cdk.test.catalog_builder import CatalogBuilder
from airbyte_cdk.test.entrypoint_wrapper import EntrypointOutput, read
from source_slack import client as client_module
from source_slack import streams as streams_module
from source_slack.client import API_URL
from source_slack.source import SourceSlack

TODAY = date(2026, 10, 4)  # today - 2 days = 2026-10-02

CONFIG = {
    "insight_tenant_id": "test-tenant",
    "insight_source_id": "slack-test",
    "slack_bot_token": "xoxp-test-token",
    "slack_start_date": "2026-10-01",
}

_FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture(autouse=True)
def sleeps(monkeypatch: pytest.MonkeyPatch) -> list[float]:
    """Freeze the clock and record every retry wait instead of sleeping."""
    waits: list[float] = []
    monkeypatch.setattr(streams_module, "today", lambda: TODAY)
    monkeypatch.setattr("source_slack.source.today", lambda: TODAY)
    monkeypatch.setattr(client_module.time, "sleep", waits.append)
    return waits


def member_rows(day: str) -> list[dict[str, Any]]:
    rows = json.loads((_FIXTURES / "member_rows.json").read_text())
    return [{**row, "date": day} for row in rows]


def member_file(day: str) -> dict[str, Any]:
    body = gzip.compress("\n".join(json.dumps(r) for r in member_rows(day)).encode())
    return {"content": body, "status_code": 200, "headers": {"Content-Type": "application/gzip"}}


def refusal(code: str) -> dict[str, Any]:
    return {"json": {"ok": False, "error": code}, "status_code": 200}


def day_url(day: str) -> str:
    return f"{API_URL}?type=member&date={day}"


def catalog(sync_mode: SyncMode = SyncMode.incremental) -> ConfiguredAirbyteCatalog:
    return CatalogBuilder().with_stream("users_details", sync_mode).build()


def stream_state(value: dict[str, Any]) -> list[AirbyteStateMessage]:
    return [
        AirbyteStateMessage(
            type=AirbyteStateType.STREAM,
            stream=AirbyteStreamState(
                stream_descriptor=StreamDescriptor(name="users_details"), stream_state=AirbyteStateBlob(**value)
            ),
        )
    ]


def run(
    state: list[AirbyteStateMessage] | None = None,
    expecting_exception: bool = False,
    config: dict[str, Any] | None = None,
) -> EntrypointOutput:
    return read(SourceSlack(), config or CONFIG, catalog(), state=state, expecting_exception=expecting_exception)


def last_state(output: EntrypointOutput) -> dict[str, Any]:
    return output.state_messages[-1].state.stream.stream_state.__dict__
