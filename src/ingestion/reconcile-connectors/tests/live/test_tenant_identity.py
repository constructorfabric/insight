"""Which tenant reconcile writes into Airbyte and Argo, on a fresh and a warm install.

A fresh install names everything by the tenant UUID. A warm install already holds
sources, connections, sync state and schedules named by the tenant it ran under
before; moving to the UUID renames them in place, so every id and every byte of
sync state is kept and no schedule is left behind under the old name.
"""

from __future__ import annotations

from pathlib import Path

from conftest import CONNECTORS, Snapshot, Stand

TENANT = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
FORMER_TENANT = "example-legacy"
TENANT_LABEL = "insight.cyberfabric.com/tenant"


def _synthetic_state(stand: Stand, connection: dict) -> dict:
    catalog = stand.airbyte._post("/api/v1/connections/get", {"connectionId": connection["connectionId"]})
    streams = [s["stream"] for s in catalog["syncCatalog"]["streams"] if s["config"]["selected"]]
    return {
        "stateType": "stream",
        "streamState": [
            {
                "streamDescriptor": {
                    "name": s["name"],
                    **({"namespace": s["namespace"]} if s.get("namespace") else {}),
                },
                "streamState": {"cursor": "2024-01-01T00:00:00Z"},
            }
            for s in streams
        ],
    }


def _ids(snapshot: Snapshot) -> tuple[set[str], set[str]]:
    return (
        {s["sourceId"] for s in snapshot.sources.values()},
        {c["connectionId"] for c in snapshot.connections.values()},
    )


def _states_by_connection(snapshot: Snapshot) -> dict[str, dict]:
    return {snapshot.connections[name]["connectionId"]: state for name, state in snapshot.states.items()}


def test_a_fresh_install_names_everything_by_the_tenant(stand: Stand) -> None:
    stand.apply_connector_secrets()
    stand.reconcile(INSIGHT_TENANT_ID=TENANT)
    first = stand.snapshot()

    assert set(first.sources) == {f"{c}-main-{TENANT}" for c in CONNECTORS}, sorted(first.sources)
    assert set(first.connections) == {f"{c}-main-{TENANT}-conn" for c in CONNECTORS}, sorted(first.connections)
    assert len(first.cronworkflows) == len(CONNECTORS), sorted(first.cronworkflows)
    assert {cw["metadata"]["labels"][TENANT_LABEL] for cw in first.cronworkflows.values()} == {TENANT}

    stand.reconcile(INSIGHT_TENANT_ID=TENANT)
    second = stand.snapshot()

    assert _ids(second) == _ids(first), "a second tick must change nothing"
    assert set(second.cronworkflows) == set(first.cronworkflows)


def test_a_warm_install_is_renamed_keeping_its_ids_and_sync_state(stand: Stand, baseline_main: Path) -> None:
    stand.apply_connector_secrets()
    stand.reconcile(baseline_main, INSIGHT_TENANT_ID=FORMER_TENANT)
    for connection in stand.snapshot().connections.values():
        stand.airbyte.set_state(connection["connectionId"], _synthetic_state(stand, connection))
    warm = stand.snapshot()
    assert set(warm.sources) == {f"{c}-main-{FORMER_TENANT}" for c in CONNECTORS}, sorted(warm.sources)

    stand.reconcile(INSIGHT_TENANT_ID=TENANT)
    upgraded = stand.snapshot()

    assert set(upgraded.sources) == {f"{c}-main-{TENANT}" for c in CONNECTORS}, sorted(upgraded.sources)
    assert set(upgraded.connections) == {f"{c}-main-{TENANT}-conn" for c in CONNECTORS}, sorted(upgraded.connections)
    assert _ids(upgraded) == _ids(warm), "renamed in place, never recreated"
    assert _states_by_connection(upgraded) == _states_by_connection(warm)
    for source in upgraded.sources.values():
        assert source["connectionConfiguration"]["insight_tenant_id"] == TENANT
    assert len(upgraded.cronworkflows) == len(CONNECTORS), sorted(upgraded.cronworkflows)
    assert {cw["metadata"]["labels"][TENANT_LABEL] for cw in upgraded.cronworkflows.values()} == {TENANT}
