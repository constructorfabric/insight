"""Unit tests of the harness itself (no connector API mocking needed)."""

from __future__ import annotations

import pytest

from connector_tests import ConfigBuilder, connector_dir, get_source, stream_schema
from connector_tests.builders import TEST_SOURCE_ID, TEST_TENANT_ID
from connector_tests.schema_assert import assert_records_conform


def test_config_builder_always_carries_tenant_and_source() -> None:
    config = ConfigBuilder().build()
    assert config["insight_tenant_id"] == TEST_TENANT_ID
    assert config["insight_source_id"] == TEST_SOURCE_ID

    custom = (
        ConfigBuilder().with_tenant_id("t2").with_source_id("s2").with_field("x", 1).build()
    )
    assert custom["insight_tenant_id"] == "t2"
    assert custom["insight_source_id"] == "s2"
    assert custom["x"] == 1


def test_connector_dir_rejects_unknown_package() -> None:
    with pytest.raises(FileNotFoundError, match="no connector.yaml"):
        connector_dir("no-such-category/no-such-connector")


def test_get_source_rejects_config_missing_required_field() -> None:
    # jira spec requires jira_instance_url etc. — a bare base config must fail
    # at construction time, naming the field.
    with pytest.raises(ValueError, match="jira_instance_url"):
        get_source("task-tracking/jira", ConfigBuilder().build())


def test_stream_schema_resolves_inline_schema() -> None:
    schema = stream_schema("task-tracking/jira", "jira_projects")
    assert schema["type"] == "object"
    assert "unique_key" in schema["properties"]


def test_stream_schema_unknown_stream() -> None:
    with pytest.raises(ValueError, match="not found"):
        stream_schema("task-tracking/jira", "no_such_stream")


def test_load_fixture_overrides_and_errors(tmp_path) -> None:
    from connector_tests import load_fixture

    jira_tests = str(
        connector_dir("task-tracking/jira") / "tests" / "test_jira_projects.py"
    )
    base = load_fixture(jira_tests, "project.json")
    assert base["key"] == "PROJ1"
    # overrides may use any record field name, including `name`
    over = load_fixture(jira_tests, "project.json", key="PROJ2", name="Project PROJ2")
    assert (over["key"], over["name"]) == ("PROJ2", "Project PROJ2")
    assert base["key"] == "PROJ1", "overrides must not mutate the cached/base data"
    with pytest.raises(FileNotFoundError):
        load_fixture(jira_tests, "no_such.json")


def test_connector_inventory_tracks_covered_and_missing() -> None:
    from connector_tests.plugin import connector_inventory

    rows = {r["connector"]: r for r in connector_inventory()}
    jira = rows["task-tracking/jira"]
    assert jira["type"] == "nocode"
    assert jira["covered"], "the jira reference suite must be visible as covered"
    gitlab = rows["git/gitlab"]
    assert gitlab["type"] == "cdk"
    assert any(
        r["type"] == "nocode" and not r["covered"] for r in rows.values()
    ), "connectors without a suite must be reported as missing"


def test_terminal_summary_reports_coverage_table(monkeypatch, tmp_path) -> None:
    """The end-of-run report (terminal + GitHub job summary) lists covered and
    missing connectors. Called directly because pytest stops measuring coverage
    before real terminal-summary hooks run."""
    from connector_tests import plugin

    class _Reporter:
        def __init__(self) -> None:
            self.lines: list[str] = []

        def section(self, title: str) -> None:
            self.lines.append(title)

        def line(self, text: str) -> None:
            self.lines.append(text)

    step_summary = tmp_path / "summary.md"
    monkeypatch.setenv("GITHUB_STEP_SUMMARY", str(step_summary))
    monkeypatch.setattr(plugin, "_summary_emitted", False)
    reporter = _Reporter()
    try:
        plugin.pytest_terminal_summary(reporter, 0, None)
        # idempotence: a second registration (another conftest) emits nothing
        before = len(reporter.lines)
        plugin.pytest_terminal_summary(reporter, 0, None)
        assert len(reporter.lines) == before
    finally:
        # let the real end-of-run hook still emit its report
        monkeypatch.setattr(plugin, "_summary_emitted", False)

    text = "\n".join(reporter.lines)
    assert "task-tracking/jira" in text and "covered" in text
    assert "MISSING" in text
    md = step_summary.read_text()
    assert "| `task-tracking/jira` | nocode | ✅ |" in md
    assert "❌ missing" in md


def test_assert_records_conform_flags_type_violation_and_undeclared_field() -> None:
    good = {"unique_key": "t-s-1", "tenant_id": "t", "source_id": "s", "key": "P1"}
    with pytest.raises(AssertionError, match="not declared"):
        assert_records_conform(
            [dict(good, bogus_field=1)], "task-tracking/jira", "jira_projects"
        )
    # non-strict tolerates undeclared fields but still type-checks
    assert_records_conform(
        [dict(good, bogus_field=1)], "task-tracking/jira", "jira_projects", strict=False
    )
    with pytest.raises(AssertionError, match="unique_key"):
        assert_records_conform(
            [dict(good, unique_key=123)], "task-tracking/jira", "jira_projects"
        )
