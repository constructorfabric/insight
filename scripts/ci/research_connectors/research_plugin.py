from __future__ import annotations

import hashlib
import json
import os
import random
import time
from collections import Counter
from copy import deepcopy
from functools import lru_cache
from pathlib import Path
from types import SimpleNamespace

import pytest
import requests
import yaml
from airbyte_cdk.sources.declarative import concurrent_declarative_source
from airbyte_cdk.sources.declarative.yaml_declarative_source import YamlDeclarativeSource
from requests_mock import Adapter

PATCH = pytest.MonkeyPatch()
VARIANT = os.environ["CONNECTOR_RESEARCH_VARIANT"]
REPORT = Path(os.environ["CONNECTOR_RESEARCH_REPORT"])
STATS = {"manifest_calls": 0, "schema_calls": 0, "manifest_cpu_seconds": 0.0, "schema_cpu_seconds": 0.0}
DELAYS: list[float] = []
OUTCOMES: list[tuple[str, str, str]] = []
REQUESTS: list[tuple[str, str, int]] = []
CASE = ""
ORIGINAL_MANIFEST = YamlDeclarativeSource._read_and_parse_yaml_file
ORIGINAL_SCHEMA = concurrent_declarative_source._get_declarative_component_schema
ORIGINAL_SEND = Adapter.send
ORIGINAL_SLEEP = time.sleep


@lru_cache(maxsize=32)
def parse(contents: str) -> dict[str, object]:
    return yaml.safe_load(contents)


@lru_cache(maxsize=1)
def schema() -> dict[str, object]:
    return ORIGINAL_SCHEMA()


def manifest(source: YamlDeclarativeSource, filename: str) -> dict[str, object]:
    started = time.process_time()
    try:
        if VARIANT in ("parse-cache", "combined"):
            return deepcopy(parse(Path(filename).read_text()))
        return ORIGINAL_MANIFEST(source, filename)
    finally:
        STATS["manifest_calls"] += 1
        STATS["manifest_cpu_seconds"] += time.process_time() - started


def component_schema() -> dict[str, object]:
    started = time.process_time()
    try:
        return deepcopy(schema()) if VARIANT in ("parse-cache", "combined") else ORIGINAL_SCHEMA()
    finally:
        STATS["schema_calls"] += 1
        STATS["schema_cpu_seconds"] += time.process_time() - started


def sleep(seconds: float) -> None:
    DELAYS.append(float(seconds))
    if VARIANT not in ("retry-waits", "combined"):
        ORIGINAL_SLEEP(seconds)


def send(adapter: Adapter, request: requests.PreparedRequest, **kwargs: object) -> requests.Response:
    response = ORIGINAL_SEND(adapter, request, **kwargs)
    payload = json.dumps([request.method, request.url, str(request.body)], separators=(",", ":"))
    REQUESTS.append((CASE, hashlib.sha256(payload.encode()).hexdigest(), response.status_code))
    return response


def pytest_sessionstart(session: pytest.Session) -> None:
    import backoff._sync as backoff_sync
    from airbyte_cdk.sources.streams.http import rate_limiting

    if VARIANT not in ("baseline", "parse-cache", "retry-waits", "combined"):
        raise ValueError("unknown research variant")
    random.seed(0)
    PATCH.setattr(YamlDeclarativeSource, "_read_and_parse_yaml_file", manifest)
    PATCH.setattr(concurrent_declarative_source, "_get_declarative_component_schema", component_schema)
    PATCH.setattr(Adapter, "send", send)
    for module in (backoff_sync, rate_limiting):
        PATCH.setattr(module, "time", SimpleNamespace(**{**vars(module.time), "sleep": sleep}))


def pytest_runtest_setup(item: pytest.Item) -> None:
    global CASE
    CASE = item.nodeid


def pytest_runtest_logreport(report: pytest.TestReport) -> None:
    OUTCOMES.append((report.nodeid, report.when, report.outcome))


def pytest_sessionfinish(session: pytest.Session, exitstatus: int) -> None:
    counts = Counter(REQUESTS)
    report = {
        "variant": VARIANT,
        "exit_code": int(exitstatus),
        "outcomes": sorted(OUTCOMES),
        "requests": sorted([*key, count] for key, count in counts.items()),
        "requested_sleep_seconds": sum(DELAYS),
        "sleep_calls": len(DELAYS),
        "parse_stats": STATS,
        "manifest_cache": parse.cache_info()._asdict(),
        "schema_cache": schema.cache_info()._asdict(),
    }
    REPORT.write_text(json.dumps(report, indent=2) + "\n")
    PATCH.undo()
