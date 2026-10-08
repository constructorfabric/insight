"""Reconcile against a real Airbyte and Argo on the local stand (tests/live/stand.sh).

These tests drive `main.sh reconcile` end to end and read the result back from
Airbyte and Kubernetes. They skip unless the stand's environment is exported:

    eval "$(src/ingestion/reconcile-connectors/tests/live/stand.sh env)"
    pytest src/ingestion/reconcile-connectors/tests/live

Every call goes through the stand's KUBECONFIG. A KUBECONFIG outside the stand's
state directory is refused: reconcile runs kubectl without --context, and these
tests create and delete Secrets, sources and schedules.
"""

from __future__ import annotations

import base64
import json
import os
import shutil
import subprocess
import sys
import urllib.request
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
MAIN = ROOT / "main.sh"
STAND_DIR_MARKER = "insight-reconcile-stand"
DEFAULT_ORGANIZATION = "00000000-0000-0000-0000-000000000000"

#: Connectors with static schemas, so reconcile can discover them without credentials
#: that reach a real API, and syncs are never triggered.
CONNECTORS = ("cursor", "zulip-proxy")

SECRET_FIELDS = {
    "cursor": {"cursor_api_key": "synthetic-key"},
    "zulip-proxy": {
        "zulip_proxy_base_url": "http://zulip-proxy.invalid/api",
        "zulip_proxy_api_key": "synthetic-key",
        "zulip_proxy_start_date": "2024-01-01",
    },
}


def _modern_bash() -> str:
    """Reconcile needs bash 4+ (associative arrays); macOS ships 3.2 as `bash`."""
    candidates = [os.environ.get("RECONCILE_BASH"), shutil.which("bash"),
                  "/opt/homebrew/bin/bash", "/usr/local/bin/bash"]
    for candidate in filter(None, candidates):
        probe = subprocess.run([candidate, "-c", "echo ${BASH_VERSINFO[0]}"],
                               capture_output=True, text=True, check=False)
        if probe.returncode == 0 and int(probe.stdout.strip() or 0) >= 4:
            return candidate
    raise RuntimeError("no bash 4+ found; install one or set RECONCILE_BASH")


def _stand_env() -> dict[str, str] | None:
    kubeconfig = os.environ.get("KUBECONFIG", "")
    if STAND_DIR_MARKER not in kubeconfig or not os.environ.get("AIRBYTE_URL"):
        return None
    env = dict(os.environ)
    # reconcile's helpers run as `python3` and need this interpreter's packages (pyyaml).
    path = [str(Path(sys.executable).parent), str(Path(_modern_bash()).parent), env["PATH"]]
    env["PATH"] = os.pathsep.join(path)
    return env


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    if _stand_env() is not None:
        return
    skip = pytest.mark.skip(reason="local reconcile stand not exported (see tests/live/stand.sh)")
    for item in items:
        if Path(str(item.fspath)).parent == Path(__file__).parent:
            item.add_marker(skip)


def kubectl(*args: str, stdin: str | None = None) -> str:
    result = subprocess.run(
        ["kubectl", *args], input=stdin, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise RuntimeError(f"kubectl {' '.join(args)}: {result.stderr.strip()}")
    return result.stdout


class Airbyte:
    def __init__(self, url: str, namespace: str) -> None:
        self.url = url.rstrip("/")
        secret = json.loads(kubectl("-n", namespace, "get", "secret", "airbyte-auth-secrets", "-o", "json"))
        data = secret["data"]
        creds = {
            "client_id": base64.b64decode(data["instance-admin-client-id"]).decode(),
            "client_secret": base64.b64decode(data["instance-admin-client-secret"]).decode(),
            "grant_type": "client_credentials",
        }
        self.token = self._post("/api/v1/applications/token", creds, auth=False)["access_token"]
        (workspace,) = self._post(
            "/api/v1/workspaces/list_by_organization_id", {"organizationId": DEFAULT_ORGANIZATION}
        )["workspaces"]
        self.workspace_id = workspace["workspaceId"]

    def _post(self, path: str, body: dict, auth: bool = True) -> dict:
        headers = {"Content-Type": "application/json"}
        if auth:
            headers["Authorization"] = f"Bearer {self.token}"
        request = urllib.request.Request(
            self.url + path, data=json.dumps(body).encode(), headers=headers, method="POST"
        )
        with urllib.request.urlopen(request, timeout=60) as response:
            payload = response.read()
        return json.loads(payload) if payload else {}

    def sources(self) -> list[dict]:
        return self._post("/api/v1/sources/list", {"workspaceId": self.workspace_id})["sources"]

    def connections(self) -> list[dict]:
        return self._post("/api/v1/connections/list", {"workspaceId": self.workspace_id})["connections"]

    def state(self, connection_id: str) -> dict:
        return self._post("/api/v1/state/get", {"connectionId": connection_id})

    def set_state(self, connection_id: str, state: dict) -> None:
        self._post(
            "/api/v1/state/create_or_update",
            {"connectionId": connection_id, "connectionState": {**state, "connectionId": connection_id}},
        )

    def delete_everything(self) -> None:
        for connection in self.connections():
            self._post("/api/v1/connections/delete", {"connectionId": connection["connectionId"]})
        for source in self.sources():
            self._post("/api/v1/sources/delete", {"sourceId": source["sourceId"]})


@dataclass
class Snapshot:
    """What reconcile left behind, keyed by the names an operator would see."""

    sources: dict[str, dict] = field(default_factory=dict)
    connections: dict[str, dict] = field(default_factory=dict)
    states: dict[str, dict] = field(default_factory=dict)
    cronworkflows: dict[str, dict] = field(default_factory=dict)


@dataclass
class Stand:
    env: dict[str, str]
    airbyte: Airbyte

    @property
    def namespace(self) -> str:
        return self.env["INSIGHT_NAMESPACE"]

    def apply_connector_secrets(self) -> None:
        for connector in CONNECTORS:
            manifest = {
                "apiVersion": "v1",
                "kind": "Secret",
                "metadata": {
                    "name": f"insight-{connector}-main",
                    "namespace": self.namespace,
                    "labels": {"app.kubernetes.io/part-of": "insight"},
                    "annotations": {
                        "insight.cyberfabric.com/connector": connector,
                        "insight.cyberfabric.com/source-id": "main",
                    },
                },
                "stringData": SECRET_FIELDS[connector],
            }
            kubectl("apply", "-f", "-", stdin=json.dumps(manifest))

    def reconcile(self, **tenant_env: str) -> subprocess.CompletedProcess[str]:
        env = {**self.env, **tenant_env}
        result = subprocess.run(
            [shutil.which("bash", path=env["PATH"]), str(MAIN), "reconcile", "--no-sync-trigger"],
            env=env, capture_output=True, text=True, check=False, timeout=1800,
        )
        assert result.returncode == 0, f"reconcile failed:\n{result.stdout[-4000:]}\n{result.stderr[-4000:]}"
        return result

    def snapshot(self) -> Snapshot:
        snap = Snapshot()
        for source in self.airbyte.sources():
            snap.sources[source["name"]] = source
        for connection in self.airbyte.connections():
            snap.connections[connection["name"]] = connection
            snap.states[connection["name"]] = self.airbyte.state(connection["connectionId"])
        listing = json.loads(kubectl("-n", self.namespace, "get", "cronworkflows", "-o", "json"))
        for item in listing["items"]:
            snap.cronworkflows[item["metadata"]["name"]] = item
        return snap

    def wipe(self) -> None:
        self.airbyte.delete_everything()
        kubectl("-n", self.namespace, "delete", "cronworkflows", "--all", "--ignore-not-found")
        kubectl("-n", self.namespace, "delete", "secret", "-l", "app.kubernetes.io/part-of=insight",
                "--ignore-not-found")


@pytest.fixture(scope="session")
def stand_env() -> dict[str, str]:
    env = _stand_env()
    assert env is not None
    return env


@pytest.fixture
def stand(stand_env: dict[str, str]) -> Iterator[Stand]:
    """A stand with no sources, connections, schedules or connector Secrets."""
    airbyte = Airbyte(stand_env["AIRBYTE_URL"], stand_env["AIRBYTE_AUTH_SECRET_NAMESPACE"])
    stand = Stand(stand_env, airbyte)
    stand.wipe()
    yield stand
    stand.wipe()
