from __future__ import annotations

import os
import sqlite3
import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[4]
GITOPS = ROOT / "deploy/gitops"
SQL = GITOPS / "environments/local/airbyte-manifest-versions.sql"


def test_manifest_seed_fills_missing_majors_without_overwriting_updater_rows() -> None:
    with sqlite3.connect(":memory:") as database:
        database.execute(
            "CREATE TABLE declarative_manifest_image_version (major_version INTEGER PRIMARY KEY, "
            "image_version TEXT, image_sha TEXT, created_at TEXT, updated_at TEXT)"
        )
        existing = (6, "6.99.0", "sha256:synthetic-updater-value", "synthetic-created", "synthetic-updated")
        database.execute("INSERT INTO declarative_manifest_image_version VALUES (?, ?, ?, ?, ?)", existing)
        database.executescript(SQL.read_text())
        first = database.execute("SELECT * FROM declarative_manifest_image_version ORDER BY major_version").fetchall()
        database.executescript(SQL.read_text())
        assert (
            database.execute("SELECT * FROM declarative_manifest_image_version ORDER BY major_version").fetchall()
            == first
        )
        assert [row[0] for row in first] == [5, 6, 7]
        assert first[1] == existing
        for major, version, digest, created, updated in (first[0], first[2]):
            assert str(major) == version.split(".")[0]
            assert digest.startswith("sha256:") and len(digest) == 71
            assert created and updated


def fake_tool(tmp_path: Path, name: str, text: str) -> None:
    path = tmp_path / name
    path.write_text("#!/bin/bash\n" + text)
    path.chmod(0o755)


def test_local_seed_script_uses_explicit_context_pod_credentials_and_fail_fast_sql(tmp_path: Path) -> None:
    args, stdin = tmp_path / "args", tmp_path / "stdin"
    fake_tool(tmp_path, "kubectl", 'printf "%s\\n" "$@" > "$ARGS"\ncat > "$STDIN"\n')
    result = subprocess.run(
        ["bash", str(GITOPS / "scripts/local-airbyte-manifest-seed.sh")],
        env={
            **os.environ,
            "PATH": f"{tmp_path}:/usr/bin:/bin",
            "ENV": "local",
            "KUBE_CTX": "synthetic-context",
            "NS_INFRA": "synthetic-infra",
            "ARGS": str(args),
            "STDIN": str(stdin),
        },
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    arguments = args.read_text().splitlines()
    assert arguments[:7] == [
        "--context",
        "synthetic-context",
        "-n",
        "synthetic-infra",
        "exec",
        "-i",
        "statefulset/airbyte-db",
    ]
    assert (
        "ON_ERROR_STOP=1" in arguments[-1] and "POSTGRES_USER:?" in arguments[-1] and "POSTGRES_DB:?" in arguments[-1]
    )
    assert stdin.read_text() == SQL.read_text()


@pytest.mark.parametrize("missing", ["ENV", "KUBE_CTX", "NS_INFRA", "nonlocal", "kubectl-failure"])
def test_seed_refuses_missing_inputs_other_environments_and_command_failure(tmp_path: Path, missing: str) -> None:
    marker = tmp_path / "called"
    fake_tool(tmp_path, "kubectl", 'touch "$MARKER"\nexit 9\n')
    env = {
        **os.environ,
        "PATH": f"{tmp_path}:/usr/bin:/bin",
        "ENV": "local",
        "KUBE_CTX": "synthetic-context",
        "NS_INFRA": "synthetic-infra",
        "MARKER": str(marker),
    }
    if missing == "nonlocal":
        env["ENV"] = "synthetic-other"
    elif missing != "kubectl-failure":
        del env[missing]
    result = subprocess.run(
        ["bash", str(GITOPS / "scripts/local-airbyte-manifest-seed.sh")], env=env, capture_output=True, check=False
    )
    assert result.returncode != 0
    assert marker.exists() == (missing == "kubectl-failure")


@pytest.mark.parametrize(
    "target,tool",
    [
        ("local-mock-source-install", "helm"),
        ("local-mock-source-remove", "helm"),
        ("local-mock-airbyte-api-install", "helm"),
        ("local-mock-airbyte-api-remove", "helm"),
    ],
)
def test_mock_targets_work_without_inventory_with_explicit_context_and_namespace(
    tmp_path: Path, target: str, tool: str
) -> None:
    args = tmp_path / "args"
    fake_tool(tmp_path, tool, 'printf "%s\\n" "$@" > "$ARGS"\n')
    result = subprocess.run(
        [
            "make",
            target,
            "ENV=local",
            "KUBE_CTX=synthetic-context",
            "NS_APP=synthetic-app",
            f"INVENTORY={tmp_path}/absent.yaml",
            f"VALUES={tmp_path}/absent-values.yaml",
            "MOCK_TOOLBOX_IMAGE=synthetic-toolbox:test",
            "MOCK_AIRBYTE_UPSTREAM=http://synthetic.invalid:8001",
        ],
        cwd=GITOPS,
        env={**os.environ, "PATH": f"{tmp_path}:{os.environ['PATH']}", "ARGS": str(args)},
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    arguments = args.read_text().splitlines()
    assert arguments[arguments.index("--kube-context") + 1] == "synthetic-context"
    assert arguments[arguments.index("--namespace") + 1] == "synthetic-app"


def test_local_profile_routes_ingestion_reconcile_and_platform_through_one_override(tmp_path: Path) -> None:
    result = subprocess.run(
        ["bash", str(GITOPS / "scripts/local-airbyte-proxy-profile.sh")],
        env={**os.environ, "ENV": "local", "NS_APP": "synthetic-app"},
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    overlay = tmp_path / "profile.yaml"
    overlay.write_text(result.stdout)
    expected = "http://mock-airbyte-api.synthetic-app.svc.cluster.local:8080"
    assert yaml.safe_load(result.stdout) == {"airbyte": {"apiUrl": expected}}
    rendered = subprocess.run(
        [
            "helm",
            "template",
            "synthetic",
            str(ROOT / "charts/insight"),
            "-f",
            str(overlay),
            "--set",
            "ingestion.templates.enabled=true",
            "--set",
            "ingestion.reconcile.enabled=true",
            "--set-string",
            "ingestion.reconcile.tenantId=synthetic",
            "--set-string",
            "global.tenantDefaultId=synthetic",
            "--set-string",
            "clickhouse.host=synthetic.invalid",
            "--set-string",
            "mariadb.host=synthetic.invalid",
            "--set-string",
            "redis.host=synthetic.invalid",
            "--set-string",
            "clickhouse.database=synthetic",
            "--set-string",
            "clickhouse.username=synthetic",
            "--set-string",
            "mariadb.database=synthetic",
            "--set-string",
            "mariadb.username=synthetic",
            "--set-string",
            "redpanda.brokers=synthetic.invalid:9092",
            "--set-string",
            "ingestion.toolboxImage=synthetic-toolbox:test",
            "--set-string",
            "authenticator.oidc.issuerUrl=https://synthetic.invalid",
            "--set-string",
            "authenticator.oidc.redirectUri=https://synthetic.invalid/callback",
            "--set-string",
            "authenticator.oidc.resolveBy=email",
            "--set-string",
            "identityResolution.rosterSourceType=synthetic",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert rendered.returncode == 0, rendered.stderr
    objects = [item for item in yaml.safe_load_all(rendered.stdout) if item]
    ingestion = [
        item
        for item in objects
        if item["kind"] == "WorkflowTemplate" and item["metadata"]["name"] in {"airbyte-sync", "ingestion-pipeline"}
    ]
    assert len(ingestion) == 2
    for item in ingestion:
        assert expected in json_text(item)
    reconcile = [item for item in objects if item["kind"] == "CronWorkflow" and "reconcile" in item["metadata"]["name"]]
    assert len(reconcile) == 1 and expected in json_text(reconcile[0])
    platform = [item for item in objects if item["kind"] == "ConfigMap" and "AIRBYTE_API_URL" in item.get("data", {})]
    assert len(platform) == 1 and platform[0]["data"]["AIRBYTE_API_URL"] == expected
    mover = (ROOT / "src/ingestion/reconcile-connectors/python/sweep/mover.py").read_text()
    assert 'source.get("AIRBYTE_URL")' in mover


def json_text(value: object) -> str:
    import json

    return json.dumps(value)


def test_ordinary_airbyte_setup_seeds_only_the_local_environment() -> None:
    makefile = (GITOPS / "Makefile").read_text()
    start = makefile.index("system-airbyte:")
    recipe = makefile[start : makefile.index("\n# Argo Workflows:", start)]
    assert recipe.index("bash scripts/airbyte-setup.sh") < recipe.index("$(MAKE) local-airbyte-manifest-seed")
    assert 'if [ "$(ENV)" = local ]; then $(MAKE) local-airbyte-manifest-seed; fi' in recipe
