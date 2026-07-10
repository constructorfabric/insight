"""Pytest fixtures and hooks shared by the harness run and standalone
per-connector runs. Loaded from conftest.py files via

    from connector_tests.plugin import *

(the harness root conftest and each suite's conftest), so the import happens
after pytest-cov starts and the module is measured like any other harness code.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from airbyte_cdk.test.mock_http import HttpMocker

__all__ = ["http_mocker", "pytest_collectstart", "pytest_runtest_makereport"]


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    """Expose the test outcome to fixture teardown (standard pytest pattern)."""
    outcome = yield
    rep = outcome.get_result()
    setattr(item, f"rep_{rep.when}", rep)


@pytest.fixture
def http_mocker(request):
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
