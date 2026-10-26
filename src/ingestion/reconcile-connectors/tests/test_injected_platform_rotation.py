"""A git-proxy source is pushed to Airbyte only when its config drifts.

Every push retires the previous secret version, and a sync still hydrating
that version fails, so a steady tick must leave the source alone and a rotated
proxy token must cost exactly one push.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONNECTOR_DIR = ROOT.parent / "connectors" / "git" / "bitbucket-cloud"
TENANT = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
SOURCE_NAME = f"bitbucket-cloud-main-{TENANT}"
SECRET_HASH = "c" * 64

#: Airbyte answered from files the test writes; every write is recorded, and
#: the tag write lands in the listing so the next tick reads it back.
STUBS = """
kubectl()                { printf '{"stringData": {"bitbucket_token": "t", "bitbucket_workspaces": "acme"}}'; }
valsec_check_secret()    { return 0; }
reconcile_definitions()  { printf 'noop\\tnone\\tdef-1\\n'; }
reconcile_rename_former_tenant() { return 0; }
reconcile_classify_change() { printf 'noop'; }
reconcile_compute_schedule() { printf '0 0 * * *'; }
ab_workspace_id()        { printf 'ws-1'; }
ab_list_sources()        { cat "$SOURCES"; }
ab_list_connections()    { cat "$CONNECTIONS"; }
ab_update_source()       { printf 'UPDATE_SOURCE %s\\n' "$1" >> "$CALLS"; }
argo_apply_cronworkflow() { return 0; }
argo_delete_former_tenant_cronworkflows() { return 0; }
argo_submit_sync_trigger() { printf 'SYNC_TRIGGER %s\\n' "$1" >> "$CALLS"; }
adopt_tag_connection() {
  printf 'TAG %s\\n' "${2:0:12}" >> "$CALLS"
  python3 - "$CONNECTIONS" "$2" <<'PY'
import json, sys
path, cfg_hash = sys.argv[1], sys.argv[2][:12]
connections = json.load(open(path))
connections[0]["tags"] = [{"name": "insight"}, {"name": f"cfg-hash:{cfg_hash}"}]
json.dump(connections, open(path, "w"))
PY
}
"""


def tick(tmp_path: Path, proxy_token: str) -> list[str]:
    calls = tmp_path / "calls"
    calls.unlink(missing_ok=True)
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight INSIGHT_TENANT_ID={TENANT} CONNECTORS_DIR="{ROOT.parent / "connectors"}"
    export AIRBYTE_URL=http://127.0.0.1:1 GIT_PROXY_URL=https://proxy.example.com GIT_PROXY_TOKEN={proxy_token}
    export SOURCES="{tmp_path / "sources.json"}" CONNECTIONS="{tmp_path / "connections.json"}" CALLS="{calls}"
    source "{ROOT}/lib/reconcile.sh"
    {STUBS}
    _reconcile_one_connector bitbucket-cloud "{CONNECTOR_DIR}" 1.0.0 nocode "" "" \\
      bronze_bitbucket_cloud main insight-bitbucket-cloud-main {SECRET_HASH} 0 0 "" ""
    """
    result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, check=False)
    assert result.returncode == 0, result.stdout + result.stderr
    return calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []


def seed(tmp_path: Path) -> None:
    source = {"sourceId": "src-1", "name": SOURCE_NAME, "sourceDefinitionId": "def-1", "connectionConfiguration": {}}
    connection = {
        "connectionId": "conn-1",
        "sourceId": "src-1",
        "name": f"{SOURCE_NAME}-conn",
        "tags": [{"name": "insight"}],
    }
    (tmp_path / "sources.json").write_text(json.dumps([source]), encoding="utf-8")
    (tmp_path / "connections.json").write_text(json.dumps([connection]), encoding="utf-8")


def test_a_steady_tick_pushes_nothing_and_a_rotated_proxy_token_is_pushed_once(tmp_path: Path) -> None:
    seed(tmp_path)
    adopted = tick(tmp_path, "token-1")

    steady = tick(tmp_path, "token-1")
    rotated = tick(tmp_path, "token-2")
    settled = tick(tmp_path, "token-2")

    assert adopted[0].startswith("TAG ") and "UPDATE_SOURCE src-1" in adopted, (
        f"first tick tags and pushes once: {adopted}"
    )
    assert steady == [], f"a tick without drift must not touch the source: {steady}"
    assert [c for c in rotated if not c.startswith("TAG ")] == [
        "UPDATE_SOURCE src-1",
        "SYNC_TRIGGER bitbucket-cloud",
    ], rotated
    assert settled == [], f"the tick after a rotation is steady again: {settled}"
