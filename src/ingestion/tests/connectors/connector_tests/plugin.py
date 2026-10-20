"""Pytest fixtures and hooks shared by the harness run and standalone
per-connector runs. Loaded from conftest.py files via

    from connector_tests.plugin import *

(the harness root conftest and each suite's conftest), so the import happens
after pytest-cov starts and the module is measured like any other harness code.
"""

from __future__ import annotations

import os
import sys
from collections.abc import Iterator
from pathlib import Path
from types import SimpleNamespace

import pytest
from airbyte_cdk.test.mock_http import HttpMocker

__all__ = [
    "cached_source_definitions",
    "connector_inventory",
    "http_mocker",
    "pytest_collectstart",
    "pytest_runtest_makereport",
    "pytest_terminal_summary",
    "retry_delays",
]

_CONNECTORS_DIR = Path(__file__).resolve().parents[3] / "connectors"


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    """Expose the test outcome to fixture teardown (standard pytest pattern)."""
    outcome = yield
    rep = outcome.get_result()
    setattr(item, f"rep_{rep.when}", rep)


@pytest.fixture
def cached_source_definitions(monkeypatch: pytest.MonkeyPatch) -> None:
    from airbyte_cdk.sources.declarative import concurrent_declarative_source
    from airbyte_cdk.sources.declarative.yaml_declarative_source import YamlDeclarativeSource

    from connector_tests import runtime

    monkeypatch.setattr(YamlDeclarativeSource, "_read_and_parse_yaml_file", runtime.read_manifest)
    monkeypatch.setattr(concurrent_declarative_source, "_get_declarative_component_schema", runtime.component_schema)


@pytest.fixture
def retry_delays(monkeypatch: pytest.MonkeyPatch) -> list[float]:
    import backoff._sync as backoff_sync
    from airbyte_cdk.sources.streams.http import rate_limiting

    delays: list[float] = []
    sleeper = SimpleNamespace(sleep=delays.append)
    monkeypatch.setattr(backoff_sync, "time", sleeper)
    monkeypatch.setattr(rate_limiting, "time", sleeper)
    return delays


@pytest.fixture
def http_mocker(
    request: pytest.FixtureRequest, cached_source_definitions: None, retry_delays: list[float]
) -> Iterator[HttpMocker]:
    """Transport-level HTTP mock: every request the connector issues must match
    a registered fixture (no network fallthrough). On a passing test, every
    registered matcher must have been hit at least once — mirroring the
    HttpMocker decorator semantics for plain pytest functions."""
    mocker = HttpMocker()
    mocker.__enter__()
    try:
        yield mocker
    finally:
        mocker.__exit__(None, None, None)
    rep = getattr(request.node, "rep_call", None)
    if rep is not None and rep.passed:
        mocker._validate_all_matchers_called()


def pytest_collectstart(collector):
    """Make each nocode suite's directory importable so suites can import their
    local `config.py` builders under --import-mode=importlib."""
    try:
        p = Path(str(collector.path))
    except Exception:
        return
    suite_dir = p if p.is_dir() else p.parent
    if suite_dir.name == "tests" and (suite_dir.parent / "connector.yaml").is_file():
        s = str(suite_dir)
        if s not in sys.path:
            sys.path.insert(0, s)
        # Every suite names its builder module `config.py`, and Python caches
        # imports by module name: without eviction, the second suite's
        # `from config import ...` would silently get the FIRST suite's cached
        # module. Evict when the cached one belongs to a different suite.
        cached = sys.modules.get("config")
        if cached is not None and getattr(cached, "__file__", None) != str(suite_dir / "config.py"):
            del sys.modules["config"]


def connector_inventory() -> list[dict]:
    """Every connector package (identified by descriptor.yaml) with its type and
    mock-suite status. `covered` = a tests/ dir with at least one test_*.py.
    CDK connectors run their suites under their own coverage components; nocode
    connectors are covered by this harness."""
    rows = []
    for desc in sorted(_CONNECTORS_DIR.glob("*/*/descriptor.yaml")):
        pkg = desc.parent
        ctype = "cdk" if (pkg / "pyproject.toml").is_file() else "nocode"
        tests_dir = pkg / "tests"
        rows.append(
            {
                "connector": f"{pkg.parent.name}/{pkg.name}",
                "type": ctype,
                "covered": tests_dir.is_dir() and any(tests_dir.glob("test_*.py")),
            }
        )
    return rows


_summary_emitted = False


def pytest_terminal_summary(terminalreporter, exitstatus, config):
    """Report which connectors have a test suite and which are still missing one
    — in the pytest terminal summary and, on CI, in the GitHub job summary."""
    global _summary_emitted
    if _summary_emitted:  # the hook registers once per conftest that imports it
        return
    _summary_emitted = True

    rows = connector_inventory()
    nocode = [r for r in rows if r["type"] == "nocode"]
    covered = [r for r in nocode if r["covered"]]
    terminalreporter.section("connector mock-test coverage")
    terminalreporter.line(f"nocode connectors with a mock suite: {len(covered)}/{len(nocode)}")
    for r in rows:
        mark = "covered" if r["covered"] else "MISSING"
        note = " (own component)" if r["type"] == "cdk" else ""
        terminalreporter.line(f"  {mark:8} {r['type']:6} {r['connector']}{note}")

    step_summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if step_summary:
        with Path(step_summary).open("a") as f:
            f.write(
                f"\n## Connector test coverage\n\n"
                f"Nocode connectors with a mock suite: **{len(covered)}/{len(nocode)}**\n\n"
                "| Connector | Type | Suite |\n|---|---|---|\n"
            )
            for r in rows:
                mark = "✅" if r["covered"] else "❌ missing"
                if r["type"] == "cdk":
                    mark += " (own component)" if r["covered"] else ""
                f.write(f"| `{r['connector']}` | {r['type']} | {mark} |\n")
