from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

HERE = Path(__file__).resolve()
SUBCHART = HERE.parents[1]
REPO_ROOT = HERE.parents[6]
UMBRELLA = REPO_ROOT / "charts" / "insight"

TENANT = "3e1d5a65-434c-95b4-8c1b-eb8f53a39bab"

UMBRELLA_BASE = [
    "--set",
    f"global.tenantDefaultId={TENANT}",
    "--set",
    "clickhouse.host=ch",
    "--set",
    "clickhouse.username=u",
    "--set",
    "clickhouse.password=p",
    "--set",
    "clickhouse.database=insight",
    "--set",
    "mariadb.host=m",
    "--set",
    "mariadb.username=insight",
    "--set",
    "mariadb.password=pw",
    "--set",
    "mariadb.database=insight",
    "--set",
    "redis.host=redis",
    "--set",
    "redis.password=rp",
    "--set",
    "redpanda.brokers=rp:9092",
    "--set",
    "ingestion.reconcile.tenantId=default",
    "--set",
    "authenticator.oidc.issuerUrl=https://idp",
    "--set",
    "authenticator.oidc.clientId=c",
    "--set",
    "authenticator.oidc.clientSecret=s",
    "--set",
    "authenticator.oidc.redirectUri=https://x/cb",
    "--set",
    "authenticator.oidc.sourceType=ms-entra",
]


def render(chart: Path, *extra: str) -> tuple[int, str, str]:
    proc = subprocess.run(
        ["helm", "template", "contract-test", str(chart), *extra],
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    return proc.returncode, proc.stdout, proc.stderr


@pytest.fixture(scope="session")
def umbrella_deps() -> Path:
    proc = subprocess.run(
        ["helm", "dependency", "update", str(UMBRELLA)], capture_output=True, text=True, timeout=300, check=False
    )
    assert proc.returncode == 0, proc.stderr
    return UMBRELLA
