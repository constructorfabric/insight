"""Session config: ports, credentials, paths.

Credentials are randomized per-session so concurrent sessions on the same host
don't share access. Ports default to the e2e-reserved range (30500-30999) to
avoid the local cluster port-forwards and the dbt-local-profile NodePort.
"""

from __future__ import annotations

import os
import secrets
import string
import uuid
from dataclasses import dataclass, field
from pathlib import Path


# Resolve the repo root from this file's location:
# src/ingestion/tests/e2e/lib/config.py -> ../../../../../
_REPO_ROOT = Path(__file__).resolve().parents[5]


# Header the analytics service's tenant middleware reads to resolve the request tenant
# (auth.rs::TENANT_HEADER). The harness sends it on EVERY request.
TENANT_HEADER = "X-Insight-Tenant-Id"

# Session tenant for the whole e2e run. The analytics service's tenant middleware
# rejects the nil UUID (a non-identity value must not pin tenant context), so
# the harness cannot use 0000…0. Instead it seeds metric definitions under this
# non-nil tenant and sends it as `X-Insight-Tenant-Id` on every request. The
# ClickHouse query path does not filter by tenant yet (MVP — handlers.rs), so
# fixture data carries whatever tenant it likes; only the `metrics`-table lookup
# is tenant-scoped, and that is what we align here.
TEST_TENANT_ID = uuid.UUID("11111111-1111-1111-1111-111111111111")


def _random_password(length: int = 24) -> str:
    alphabet = string.ascii_letters + string.digits
    return "".join(secrets.choice(alphabet) for _ in range(length))


@dataclass(frozen=True)
class SessionConfig:
    """All session-wide knobs in one place.

    `run_mode = "host"` (default): pytest runs on the host, compose+CH+MariaDB
    run as Docker containers with ports published on 127.0.0.1.

    `run_mode = "docker"`: pytest runs as a `runner` service on the same
    compose network — CH/MariaDB are reached via the service names
    `clickhouse:8123` and `mariadb:3306`, no host port forwarding required.
    Triggered automatically when env var `E2E_RUN_MODE=docker` is set (the
    runner image sets it). See compose/docker-compose.runner.yml.
    """

    # Filesystem
    repo_root: Path
    compose_dir: Path
    migrations_dir: Path
    dbt_project_dir: Path
    analytics_manifest_dir: Path

    # Runtime mode (where pytest runs relative to CH/MariaDB)
    run_mode: str = "host"  # "host" | "docker"

    # ClickHouse — 30523/30529 avoid the kind cluster's 30123 / 30500 / 30900
    ch_host: str = "127.0.0.1"
    ch_http_port: int = 30523
    ch_native_port: int = 30529
    ch_database: str = "insight"
    ch_user: str = "insight"
    ch_password: str = field(default_factory=_random_password)

    # MariaDB
    mariadb_host: str = "127.0.0.1"
    mariadb_port: int = 30506
    mariadb_database: str = "analytics"
    mariadb_user: str = "insight"
    mariadb_password: str = field(default_factory=_random_password)
    mariadb_root_password: str = field(default_factory=_random_password)

    @classmethod
    def from_env(cls) -> "SessionConfig":
        repo_root = Path(os.environ.get("INSIGHT_REPO_ROOT", _REPO_ROOT)).resolve()
        run_mode = os.environ.get("E2E_RUN_MODE", "host")
        if run_mode == "docker":
            # In docker mode every host/port override comes from the compose
            # file via env. Credentials come from the same .env compose reads.
            return cls(
                repo_root=repo_root,
                compose_dir=repo_root / "src/ingestion/tests/e2e/compose",
                migrations_dir=repo_root / "src/ingestion/scripts/migrations",
                dbt_project_dir=repo_root / "src/ingestion/dbt",
                analytics_manifest_dir=repo_root / "src/backend/services/analytics",
                run_mode="docker",
                ch_host=os.environ.get("E2E_CH_HOST", "clickhouse"),
                ch_http_port=int(os.environ.get("E2E_CH_HTTP_PORT", 8123)),
                ch_native_port=int(os.environ.get("E2E_CH_NATIVE_PORT", 9000)),
                ch_user=os.environ.get("E2E_CH_USER", "insight"),
                ch_password=os.environ["E2E_CH_PASSWORD"],
                mariadb_host=os.environ.get("E2E_MARIADB_HOST", "mariadb"),
                mariadb_port=int(os.environ.get("E2E_MARIADB_PORT", 3306)),
                mariadb_database=os.environ.get("E2E_MARIADB_DATABASE", "analytics"),
                mariadb_user=os.environ.get("E2E_MARIADB_USER", "insight"),
                mariadb_password=os.environ["E2E_MARIADB_PASSWORD"],
                # root pw not used in docker mode — leave as default random
            )
        return cls(
            repo_root=repo_root,
            compose_dir=repo_root / "src/ingestion/tests/e2e/compose",
            migrations_dir=repo_root / "src/ingestion/scripts/migrations",
            dbt_project_dir=repo_root / "src/ingestion/dbt",
            analytics_manifest_dir=repo_root / "src/backend/services/analytics",
            ch_http_port=int(os.environ.get("E2E_CH_HTTP_PORT", 30523)),
            ch_native_port=int(os.environ.get("E2E_CH_NATIVE_PORT", 30529)),
            mariadb_port=int(os.environ.get("E2E_MARIADB_PORT", 30506)),
        )

    @property
    def ch_http_url(self) -> str:
        return f"http://{self.ch_host}:{self.ch_http_port}"

    @property
    def mariadb_dsn(self) -> str:
        """SeaORM / SQLAlchemy-style URL for analytics."""
        return (
            f"mysql://{self.mariadb_user}:{self.mariadb_password}"
            f"@{self.mariadb_host}:{self.mariadb_port}/{self.mariadb_database}"
        )

    def compose_env(self) -> dict[str, str]:
        """Env passed to `docker compose` to substitute into docker-compose.yml."""
        return {
            "CLICKHOUSE_DB": self.ch_database,
            "CLICKHOUSE_USER": self.ch_user,
            "CLICKHOUSE_PASSWORD": self.ch_password,
            "MARIADB_DATABASE": self.mariadb_database,
            "MARIADB_USER": self.mariadb_user,
            "MARIADB_PASSWORD": self.mariadb_password,
            "MARIADB_ROOT_PASSWORD": self.mariadb_root_password,
            "CH_HTTP_PORT": str(self.ch_http_port),
            "CH_NATIVE_PORT": str(self.ch_native_port),
            "MARIADB_PORT": str(self.mariadb_port),
        }
