from __future__ import annotations

import os
import shlex
import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "deploy/compose/insight-init.sh"
LOCAL = ROOT / "deploy/gitops/environments/local"


def run_fragment(
    directory: Path, airbyte: bool, argo: bool, fail_values: bool = False
) -> subprocess.CompletedProcess[str]:
    source = SCRIPT.read_text()
    fragment = source[source.index("  local core_token signing_key") : source.index("  trap - EXIT")]
    inputs = {
        "inventory_tmpl": LOCAL / "inventory.yaml.template",
        "values_tmpl": LOCAL / "values.yaml.template",
        "inventory_out": directory / "inventory.yaml",
        "values_out": directory / "values.yaml",
        "env_local_out": directory / ".env.local",
        "secrets_store_out": directory / "secrets-store.yaml",
        "kube_ctx": "synthetic",
        "TENANT_DEFAULT_ID": "00000000-0000-4000-8000-000000000001",
        "DEV_USER_EMAIL": "user@example.com",
        "airbyte_email": "user@example.com",
        "airbyte_org": "Example Corp",
        "MARIADB_ROOT_PASSWORD": "synthetic",
        "MARIADB_PASSWORD": "synthetic",
        "CLICKHOUSE_PASSWORD": "synthetic",
        "sys_airbyte": str(airbyte).lower(),
        "sys_argo": str(argo).lower(),
    }
    for key in ("l0_envoy_gateway", "l0_cert_manager", "l0_sealed_secrets", "sys_redpanda_console", "sys_obs"):
        inputs[key] = "false"
    assignments = "\n".join(f"{key}={shlex.quote(str(value))}" for key, value in inputs.items())
    env = os.environ.copy()
    if fail_values:
        binary = directory / "bin"
        binary.mkdir()
        wrapper = binary / "yq"
        wrapper.write_text(
            f'#!/bin/bash\ncase "$*" in *public_keys*) exit 1;; esac\nexec {shlex.quote(shutil.which("yq"))} "$@"\n'
        )
        wrapper.chmod(0o700)
        env["PATH"] = f"{binary}:{env['PATH']}"
    return subprocess.run(
        ["/bin/bash", "-c", f"set -euo pipefail\n{assignments}\nrun() {{\n{fragment}\n}}\nrun"],
        env=env,
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )


@pytest.mark.parametrize(("airbyte", "argo"), [(True, True), (False, True), (True, False), (False, False)])
def test_generated_reader_key_matches_the_registry_and_ingestion_switch(
    tmp_path: Path, airbyte: bool, argo: bool
) -> None:
    result = run_fragment(tmp_path, airbyte, argo)
    assert result.returncode == 0, result.stderr
    store_path = tmp_path / "secrets-store.yaml"
    assert store_path.stat().st_mode & 0o777 == 0o600
    store = yaml.safe_load(store_path.read_text())
    secret = yaml.safe_load(store["insight-local-insight-reconcile-signing-key"])
    assert secret["metadata"]["name"] == "insight-reconcile-signing-key"
    private = secret["stringData"]["private.pem"]
    public = subprocess.run(
        ["openssl", "pkey", "-pubout"], input=private, capture_output=True, text=True, check=True
    ).stdout
    details = subprocess.run(
        ["openssl", "pkey", "-text", "-noout"], input=private, capture_output=True, text=True, check=True
    ).stdout
    assert "prime256v1" in details
    values = yaml.safe_load((tmp_path / "values.yaml").read_text())
    entry = values["authenticator"]["serviceTokens"]["services"]["reconcile"]
    assert entry["public_keys"] == [public.rstrip("\n")]
    assert entry["roles"] == ["connector-schedules-reader"]
    assert values["global"]["connectorSchedules"]["enabled"] is (airbyte and argo)
    assert values["ingestion"]["templates"]["enabled"] is (airbyte and argo)
    inventory = yaml.safe_load((tmp_path / "inventory.yaml").read_text())
    assert {"name": secret["metadata"]["name"], "enabled": True} in inventory["secrets"]["services"]
    assert private.strip() not in result.stdout + result.stderr


def test_a_later_wizard_failure_removes_the_new_private_key_output(tmp_path: Path) -> None:
    result = run_fragment(tmp_path, True, True, fail_values=True)
    assert result.returncode != 0
    for name in ("inventory.yaml", "values.yaml", ".env.local", "secrets-store.yaml"):
        assert not (tmp_path / name).exists(), name
