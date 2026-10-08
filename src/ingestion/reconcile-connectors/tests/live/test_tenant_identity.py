"""Which tenant reconcile writes into Airbyte and Argo, on a fresh and a warm install.

A fresh install names everything by the canonical tenant UUID. A warm install
already holds sources, connections, sync state and schedules named by a legacy
tenant key; moving it to the canonical UUID must keep every one of them, since
Airbyte finds a source by its exact name and a renamed source is a new source
with no state.
"""

from __future__ import annotations

import pytest
from conftest import CONNECTORS, Snapshot, Stand

CANONICAL_TENANT = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
LEGACY_TENANT_KEY = "example-legacy"
TENANT_LABEL = "insight.cyberfabric.com/tenant"


def _synthetic_state(stand: Stand, connection: dict) -> dict:
    catalog = stand.airbyte._post("/api/v1/connections/get", {"connectionId": connection["connectionId"]})
    streams = [s["stream"] for s in catalog["syncCatalog"]["streams"] if s["config"]["selected"]]
    return {
        "stateType": "stream",
        "streamState": [
            {
                "streamDescriptor": {"name": s["name"], **({"namespace": s["namespace"]} if s.get("namespace") else {})},
                "streamState": {"cursor": "2024-01-01T00:00:00Z"},
            }
            for s in streams
        ],
    }


def _source_tenant(snapshot: Snapshot, name: str) -> str:
    return snapshot.sources[name]["connectionConfiguration"]["insight_tenant_id"]


def test_fresh_install_names_everything_by_the_canonical_tenant(stand: Stand) -> None:
    stand.apply_connector_secrets()
    stand.reconcile(INSIGHT_TENANT_ID=CANONICAL_TENANT)
    first = stand.snapshot()

    for connector in CONNECTORS:
        name = f"{connector}-main-{CANONICAL_TENANT}"
        assert name in first.sources, sorted(first.sources)
        assert _source_tenant(first, name) == CANONICAL_TENANT
    assert len(first.connections) == len(CONNECTORS), sorted(first.connections)
    assert len(first.cronworkflows) == len(CONNECTORS), sorted(first.cronworkflows)
    for cronworkflow in first.cronworkflows.values():
        assert cronworkflow["metadata"]["labels"][TENANT_LABEL] == CANONICAL_TENANT

    stand.reconcile(INSIGHT_TENANT_ID=CANONICAL_TENANT)
    second = stand.snapshot()
    assert {n: s["sourceId"] for n, s in second.sources.items()} == {
        n: s["sourceId"] for n, s in first.sources.items()
    }
    assert {n: c["connectionId"] for n, c in second.connections.items()} == {
        n: c["connectionId"] for n, c in first.connections.items()
    }
    assert set(second.cronworkflows) == set(first.cronworkflows)


@pytest.mark.xfail(strict=True, reason="#3661: the legacy tenant key override is not implemented yet")
def test_warm_install_keeps_its_objects_and_state(stand: Stand) -> None:
    stand.apply_connector_secrets()
    stand.reconcile(INSIGHT_TENANT_ID=LEGACY_TENANT_KEY)
    for connection in stand.snapshot().connections.values():
        stand.airbyte.set_state(connection["connectionId"], _synthetic_state(stand, connection))
    warm = stand.snapshot()
    assert len(warm.sources) == len(CONNECTORS), sorted(warm.sources)

    stand.reconcile(INSIGHT_TENANT_ID=CANONICAL_TENANT, INSIGHT_LEGACY_TENANT_KEY=LEGACY_TENANT_KEY)
    upgraded = stand.snapshot()

    assert {n: s["sourceId"] for n, s in upgraded.sources.items()} == {
        n: s["sourceId"] for n, s in warm.sources.items()
    }
    assert {n: c["connectionId"] for n, c in upgraded.connections.items()} == {
        n: c["connectionId"] for n, c in warm.connections.items()
    }
    assert upgraded.states == warm.states
    assert set(upgraded.cronworkflows) == set(warm.cronworkflows)
    for name in upgraded.sources:
        assert _source_tenant(upgraded, name) == CANONICAL_TENANT
    for cronworkflow in upgraded.cronworkflows.values():
        assert cronworkflow["metadata"]["labels"][TENANT_LABEL] == CANONICAL_TENANT
