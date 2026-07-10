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
