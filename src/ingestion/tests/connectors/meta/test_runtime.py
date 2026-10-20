from __future__ import annotations

import time
from pathlib import Path

import backoff
import pytest
import requests
from airbyte_cdk.sources.declarative.yaml_declarative_source import YamlDeclarativeSource
from airbyte_cdk.sources.streams.http.exceptions import UserDefinedBackoffException
from airbyte_cdk.sources.streams.http.rate_limiting import user_defined_backoff_handler
from connector_tests import runtime
from connector_tests.source import CONNECTORS_DIR


@pytest.mark.parametrize("path", sorted(CONNECTORS_DIR.glob("*/*/connector.yaml")))
def test_cached_manifest_matches_the_cdk_parser(path: Path) -> None:
    expected = YamlDeclarativeSource._read_and_parse_yaml_file(object(), str(path))
    assert runtime.read_manifest(object(), str(path)) == expected


def test_source_mutations_cannot_change_another_manifest(tmp_path: Path) -> None:
    path = tmp_path / "connector.yaml"
    path.write_text("streams:\n  - name: events\n")
    first = runtime.read_manifest(object(), str(path))
    first["streams"][0]["name"] = "changed"
    second = runtime.read_manifest(object(), str(path))
    assert second["streams"][0]["name"] == "events"


def test_changed_manifest_contents_are_parsed_again(tmp_path: Path) -> None:
    path = tmp_path / "connector.yaml"
    path.write_text("name: before\n")
    assert runtime.read_manifest(object(), str(path)) == {"name": "before"}
    path.write_text("name: after\n")
    assert runtime.read_manifest(object(), str(path)) == {"name": "after"}


def test_source_mutations_cannot_change_another_component_schema() -> None:
    first = runtime.component_schema()
    expected = runtime.component_schema()
    first.clear()
    assert runtime.component_schema() == expected
    assert expected


def test_retry_sleeper_does_not_replace_process_sleep(retry_delays: list[float]) -> None:
    import backoff._sync as backoff_sync

    assert backoff_sync.time is not time
    assert backoff_sync.time.sleep is not time.sleep
    backoff_sync.time.sleep(17)
    assert retry_delays == [17]


def test_exponential_retry_delays_and_attempt_limit_are_preserved(retry_delays: list[float]) -> None:
    attempts = 0

    @backoff.on_exception(backoff.expo, ValueError, max_tries=3, jitter=None)
    def fail() -> None:
        nonlocal attempts
        attempts += 1
        raise ValueError("synthetic retry failure")

    with pytest.raises(ValueError, match="synthetic retry failure"):
        fail()

    assert attempts == 3
    assert retry_delays == [1, 2]


@pytest.mark.parametrize("delay", [0, 7, 0.5])
def test_cdk_user_delay_keeps_its_safety_margin(delay: float, retry_delays: list[float]) -> None:
    attempts = 0
    request = requests.Request("GET", "https://example.com/events").prepare()

    @user_defined_backoff_handler(max_tries=2)
    def recover() -> str:
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            raise UserDefinedBackoffException(delay, request, None)
        return "recovered"

    assert recover() == "recovered"
    assert attempts == 2
    assert retry_delays == [delay + 1, 0]
