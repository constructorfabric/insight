from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

GITOPS = Path(__file__).resolve().parents[5] / "deploy/gitops"
TARGETS = [
    "local-mock-source-install",
    "local-mock-source-remove",
    "local-mock-airbyte-api-install",
    "local-mock-airbyte-api-remove",
    "local-mock-airbyte-api-profile",
    "local-airbyte-manifest-seed",
]


@pytest.fixture
def installation(tmp_path: Path) -> tuple[Path, dict[str, str], Path]:
    deploy = tmp_path / "deploy"
    deploy.mkdir()
    shutil.copyfile(GITOPS / "Makefile", deploy / "Makefile")
    shutil.copytree(GITOPS / "scripts", deploy / "scripts")
    shutil.copytree(GITOPS / "environments/local", deploy / "environments/local")
    binaries = tmp_path / "bin"
    binaries.mkdir()
    calls = tmp_path / "calls"
    for command in ["kubectl", "helm", "envsubst"]:
        binary = binaries / command
        binary.write_text(
            '#!/bin/bash\nprintf "%s %s\\n" "${0##*/}" "$*" >> "$CALLS_FILE"\n'
            'if [[ "${0##*/}" == kubectl && "$1 $2" == "config view" ]]; then\n'
            '  [[ "$*" != *"--context missing"* ]] || exit 1\n'
            '  printf "%s" "$API_SERVER"\n'
            "fi\n"
        )
        binary.chmod(0o755)
    env = {
        **os.environ,
        "PATH": f"{binaries}:{os.environ['PATH']}",
        "CALLS_FILE": str(calls),
        "ENV": "local",
        "KUBE_CTX": "synthetic-local",
        "NS_APP": "synthetic-app",
        "NS_INFRA": "synthetic-infra",
        "API_SERVER": "https://127.0.0.1:6443",
        "MOCK_TOOLBOX_IMAGE": "synthetic-toolbox:test",
        "MOCK_AIRBYTE_UPSTREAM": "http://synthetic-airbyte:8001",
    }
    return deploy, env, calls


@pytest.mark.parametrize(
    "server,accepted",
    [
        ("https://127.0.0.1:6443", True),
        ("https://localhost:6443", True),
        ("https://[::1]:6443", True),
        ("https://synthetic.orb.local:6443", True),
        ("https://host.docker.internal:6443", True),
        ("https://synthetic.local:6443", True),
        ("https://10.0.0.2:6443", True),
        ("https://172.16.0.2:6443", True),
        ("https://192.168.0.2:6443", True),
        ("https://[fd00::2]:6443", True),
        ("https://remote.example.invalid:6443", False),
        ("https://203.0.113.10:6443", False),
        ("https://172.32.0.2:6443", False),
        ("https://127.0.0.1.example.invalid:6443", False),
        ("https://synthetic.local.example.invalid:6443", False),
        ("https://user@localhost:6443", False),
        ("https://localhost:invalid", False),
        ("https://localhost:0", False),
        ("file://localhost", False),
        ("not a URL", False),
        ("", False),
    ],
)
def test_guard_accepts_only_local_api_servers(
    installation: tuple[Path, dict[str, str], Path], server: str, accepted: bool
) -> None:
    deploy, env, calls = installation
    result = subprocess.run(
        ["bash", "scripts/local-harness-guard.sh", env["NS_APP"]],
        cwd=deploy,
        env={**env, "API_SERVER": server},
        capture_output=True,
        text=True,
        check=False,
    )
    assert (result.returncode == 0) == accepted, f"server={server!r}: {result.stderr}"
    assert calls.read_text() == (
        "kubectl config view --minify --context synthetic-local -o jsonpath={.clusters[0].cluster.server}\n"
    )


@pytest.mark.parametrize("target", TARGETS)
@pytest.mark.parametrize("context,server", [("synthetic-local", "https://remote.example.invalid"), ("missing", "")])
def test_every_target_refuses_remote_or_missing_contexts_before_mutation(
    installation: tuple[Path, dict[str, str], Path], target: str, context: str, server: str
) -> None:
    deploy, env, calls = installation
    result = subprocess.run(
        ["make", target],
        cwd=deploy,
        env={**env, "KUBE_CTX": context, "API_SERVER": server},
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0, f"target={target}: {result.stdout}"
    assert all("config view --minify" in line for line in calls.read_text().splitlines()), target
    assert not (deploy / ".deploy").exists(), target


@pytest.mark.parametrize("target", TARGETS)
def test_every_target_proceeds_after_verifying_a_local_server(
    installation: tuple[Path, dict[str, str], Path], target: str
) -> None:
    deploy, env, calls = installation
    result = subprocess.run(["make", target], cwd=deploy, env=env, capture_output=True, text=True, check=False)
    assert result.returncode == 0, f"target={target}: {result.stderr}"
    commands = calls.read_text().splitlines()
    assert "kubectl config view --minify --context synthetic-local" in commands[0], target
    assert any("config view" not in command for command in commands), target


@pytest.mark.parametrize("variable,value", [("ENV", ""), ("ENV", "synthetic-other"), ("KUBE_CTX", ""), ("NS_APP", "")])
def test_guard_refuses_invalid_required_inputs_before_reading_config(
    installation: tuple[Path, dict[str, str], Path], variable: str, value: str
) -> None:
    deploy, env, calls = installation
    env[variable] = value
    result = subprocess.run(
        ["bash", "scripts/local-harness-guard.sh", env["NS_APP"]], cwd=deploy, env=env, capture_output=True, check=False
    )
    assert result.returncode != 0, variable
    assert not calls.exists(), variable
