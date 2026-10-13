"""Every declarative connector stamps the canonical tenant and keys by the tenant key.

`insight_tenant_id` is the canonical tenant UUID every record is stamped with.
`insight_tenant_key` is the tenant segment of `unique_key`: a warm install keeps
its legacy key there so rows it already stored still deduplicate against new
ones. A manifest that keys by `insight_tenant_id` would duplicate every row of a
warm install on its next sync.
"""

from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path
from typing import Any

import pytest
import yaml
from airbyte_cdk.sources.declarative.interpolation.interpolated_string import InterpolatedString
from connector_tests.source import CONNECTORS_DIR

KEYED = "{{ config.get('insight_tenant_key') or config['insight_tenant_id'] }}"
STAMPED = "{{ config['insight_tenant_id'] }}"
MANIFESTS = sorted(CONNECTORS_DIR.glob("*/*/connector.yaml"))


def _fields(node: Any) -> Iterator[tuple[str, str]]:
    """(path, value) of every AddFields field in the manifest."""
    if isinstance(node, dict):
        if node.get("type") == "AddFields":
            for field in node.get("fields", []):
                path = field.get("path")
                if isinstance(path, list) and isinstance(field.get("value"), str):
                    yield ".".join(path), field["value"]
        for value in node.values():
            yield from _fields(value)
    elif isinstance(node, list):
        for item in node:
            yield from _fields(item)


@pytest.mark.parametrize("manifest", MANIFESTS, ids=lambda p: p.parent.name)
def test_unique_keys_lead_with_the_tenant_key(manifest: Path) -> None:
    fields = list(_fields(yaml.safe_load(manifest.read_text())))
    keys = [value for path, value in fields if path == "unique_key" and "insight_tenant" in value]
    assert keys, "the manifest builds no tenant-scoped unique_key"
    assert all(value.startswith(KEYED) for value in keys), [v for v in keys if not v.startswith(KEYED)]


@pytest.mark.parametrize("manifest", MANIFESTS, ids=lambda p: p.parent.name)
def test_records_are_stamped_with_the_canonical_tenant(manifest: Path) -> None:
    stamps = [value for path, value in _fields(yaml.safe_load(manifest.read_text())) if path == "tenant_id"]
    assert stamps and all(value == STAMPED for value in stamps), stamps


@pytest.mark.parametrize("manifest", MANIFESTS, ids=lambda p: p.parent.name)
def test_the_spec_accepts_a_tenant_key_without_requiring_it(manifest: Path) -> None:
    spec = yaml.safe_load(manifest.read_text())["spec"]["connection_specification"]
    assert "insight_tenant_key" in spec["properties"]
    assert "insight_tenant_key" not in spec.get("required", [])


@pytest.mark.parametrize(
    ("config", "expected"),
    [
        ({"insight_tenant_id": "tenant-uuid", "insight_tenant_key": "example-legacy"}, "example-legacy"),
        ({"insight_tenant_id": "tenant-uuid"}, "tenant-uuid"),
        ({"insight_tenant_id": "tenant-uuid", "insight_tenant_key": ""}, "tenant-uuid"),
    ],
)
def test_the_key_expression_falls_back_to_the_tenant(config: dict[str, str], expected: str) -> None:
    assert InterpolatedString.create(KEYED, parameters={}).eval(config) == expected
