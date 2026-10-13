"""The canonical tenant and the legacy key reach different places.

Records are stamped with the canonical tenant UUID; Airbyte/Argo names and the
tenant segment of Bronze keys carry the key a warm install created them with.
Mixing the two up either orphans a warm install's sources or files its rows
under a tenant nobody reads.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
TENANT = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
LEGACY = "example-legacy"


def test_a_source_config_carries_both_the_tenant_and_the_key() -> None:
    result = subprocess.run(
        [
            sys.executable, str(ROOT / "python" / "compose_source_config.py"),
            "--tenant-id", TENANT, "--tenant-key", LEGACY, "--source-id", "main",
        ],
        input=json.dumps({"api_key": "synthetic", "insight_tenant_id": "from-the-secret"}),
        capture_output=True, text=True, check=True,
    )
    assert json.loads(result.stdout) == {
        "api_key": "synthetic",
        "insight_tenant_id": TENANT,
        "insight_tenant_key": LEGACY,
        "insight_source_id": "main",
    }


def _sync_trigger() -> dict:
    result = subprocess.run(
        [
            sys.executable, str(ROOT / "python" / "render_sync_trigger.py"),
            "--connector", "cursor", "--connection-name", f"cursor-main-{LEGACY}-conn",
            "--tenant", TENANT, "--tenant-key", LEGACY,
            "--insight-source-id", "main", "--tpl", str(ROOT / "templates" / "sync-trigger.yaml.tpl"),
        ],
        env={**os.environ, "INSIGHT_NAMESPACE": "insight", "ARGO_SERVICE_ACCOUNT": "insight-reconcile"},
        capture_output=True, text=True, check=True,
    )
    return yaml.safe_load(result.stdout)


def test_a_sync_trigger_is_named_by_the_key_and_labelled_with_the_tenant() -> None:
    metadata = _sync_trigger()["metadata"]
    assert metadata["generateName"] == f"cursor-{LEGACY}-sync-now-"
    assert metadata["labels"]["insight.cyberfabric.com/tenant"] == TENANT


def _resolve(tenant: str, legacy_key: str = "") -> subprocess.CompletedProcess[str]:
    script = f"""
    set -uo pipefail
    eval "$(sed -n '/^resolve_tenant_id()/,/^}}/p' "{ROOT}/main.sh")"
    INSIGHT_TENANT_ID='{tenant}' INSIGHT_TENANT_KEY='{legacy_key}' resolve_tenant_id
    """
    return subprocess.run(["bash", "-c", script], capture_output=True, text=True, check=False)


def test_the_canonical_tenant_is_accepted() -> None:
    result = _resolve(TENANT, LEGACY)
    assert result.returncode == 0, result.stderr
    assert result.stdout == TENANT


@pytest.mark.parametrize("value", ["", LEGACY, TENANT.upper(), f"{TENANT} "])
def test_anything_but_a_lowercase_uuid_is_refused(value: str) -> None:
    result = _resolve(value)
    assert result.returncode != 0
    assert "INSIGHT_TENANT_KEY" in result.stderr


def test_a_legacy_key_with_whitespace_is_refused() -> None:
    assert _resolve(TENANT, "example legacy").returncode != 0
