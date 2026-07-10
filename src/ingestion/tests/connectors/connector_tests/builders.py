"""Base config builder for connector mock tests.

Always carries the mandatory `insight_tenant_id` / `insight_source_id` pair so
every suite exercises the tenant/source stamping path with known values.
Connector suites extend it with their source-prefixed fields:

    class JiraConfigBuilder(ConfigBuilder):
        def __init__(self):
            super().__init__()
            self._config.update({...})
"""

from __future__ import annotations

from typing import Any

TEST_TENANT_ID = "test-tenant"
TEST_SOURCE_ID = "test-source"


class ConfigBuilder:
    def __init__(self) -> None:
        self._config: dict[str, Any] = {
            "insight_tenant_id": TEST_TENANT_ID,
            "insight_source_id": TEST_SOURCE_ID,
        }

    def with_tenant_id(self, tenant_id: str) -> "ConfigBuilder":
        self._config["insight_tenant_id"] = tenant_id
        return self

    def with_source_id(self, source_id: str) -> "ConfigBuilder":
        self._config["insight_source_id"] = source_id
        return self

    def with_field(self, key: str, value: Any) -> "ConfigBuilder":
        self._config[key] = value
        return self

    def build(self) -> dict[str, Any]:
        return dict(self._config)
