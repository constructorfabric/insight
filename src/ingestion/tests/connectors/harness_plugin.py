"""Early-loading pytest plugin (addopts `-p harness_plugin`) owning collection.

A bare `pytest` run in the harness root collects the harness's own `meta/`
tests plus every NOCODE connector's mock suite
(`src/ingestion/connectors/<category>/<name>/tests` where the package has a
connector.yaml and no pyproject.toml). CDK (Python) connectors carry their own
pyproject.toml, their own airbyte-cdk pin, and their own coverage component —
their suites are never collected here (their conftest.py would not even import
under this venv). Paths are injected before initial conftest loading, so CDK
conftests are never touched.

Explicit positional args (`pytest ../../connectors/task-tracking/jira/tests`)
disable the injection and run only what was asked.

This module intentionally lives OUTSIDE the measured `connector_tests` package
and imports nothing from it: it is imported during option preparsing, before
pytest-cov starts coverage. Fixtures and runtime hooks live in
connector_tests.plugin, loaded via conftest files instead.
"""

from __future__ import annotations

from pathlib import Path

_HARNESS_ROOT = Path(__file__).resolve().parent
_CONNECTORS_DIR = _HARNESS_ROOT.parents[1] / "connectors"


def nocode_suite_dirs() -> list[Path]:
    suites = []
    for tests_dir in sorted(_CONNECTORS_DIR.glob("*/*/tests")):
        pkg = tests_dir.parent
        if (pkg / "connector.yaml").is_file() and not (pkg / "pyproject.toml").is_file():
            suites.append(tests_dir)
    return suites


def pytest_addoption(parser):
    group = parser.getgroup("connector-tests")
    group.addoption(
        "--meta-only", action="store_true", default=False,
        help="collect only the harness's own unit tests (meta/)",
    )
    group.addoption(
        "--suites-only", action="store_true", default=False,
        help="collect only the nocode connectors' mock suites",
    )


def pytest_load_initial_conftests(early_config, parser, args):
    ns = early_config.known_args_namespace
    if getattr(ns, "file_or_dir", None):
        return  # explicit paths given — collect only those
    injected = []
    if not getattr(ns, "suites_only", False):
        injected.append(str(_HARNESS_ROOT / "meta"))
    if not getattr(ns, "meta_only", False):
        injected.extend(str(p) for p in nocode_suite_dirs())
    args.extend(injected)
