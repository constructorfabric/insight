"""Session config: ports, credentials, paths.

The e2e data tier (ClickHouse + MariaDB) is provided by the repo-root
`docker-compose.yml`; the runner ATTACHES to it rather than spinning up its own
pair. Connection settings therefore default to the same values the root stack
uses — the committed defaults are `insight` / `insight-local`, matching
`.env.compose.example`. When your root stack runs with custom credentials or
ports, point the matching `E2E_*` env var at them (the runner override wires
these up from the root `.env.compose` automatically).
"""

from __future__ import annotations

import os
import uuid
from dataclasses import dataclass
from pathlib import Path


# Resolve the repo root from this file's location:
# src/ingestion/tests/e2e/e2e_lib/config.py -> ../../../../../
_REPO_ROOT = Path(__file__).resolve().parents[5]


# Header analytics-api's tenant middleware reads to resolve the request tenant
# (auth.rs::TENANT_HEADER). The harness sends it on EVERY request.
TENANT_HEADER = "X-Insight-Tenant-Id"

# Session tenant for the whole e2e run. analytics-api's tenant middleware
# rejects the nil UUID (a non-identity value must not pin tenant context), so
# the harness cannot use 0000…0. Instead it seeds metric definitions under this
# non-nil tenant and sends it as `X-Insight-Tenant-Id` on every request. The
# ClickHouse query path does not filter by tenant yet (MVP — handlers.rs), so
# fixture data carries whatever tenant it likes; only the `metrics`-table lookup
# is tenant-scoped, and that is what we align here.
TEST_TENANT_ID = uuid.UUID("11111111-1111-1111-1111-111111111111")


# Defaults shared with the root docker-compose.yml data tier (see
# .env.compose.example). A real .env.compose may override these; export the
# matching E2E_* var so the harness connects with the same values.
_DEFAULT_DB_USER = "insight"
_DEFAULT_DB_PASSWORD = "insight-local"


@dataclass(frozen=True)
class SessionConfig:
    """All session-wide knobs in one place.

    `run_mode = "host"` (default): pytest runs on the host and connects to the
    root stack's published loopback ports (127.0.0.1:8123 / :3306).

    `run_mode = "docker"`: pytest runs as the `runner` service on the root
    `insight` network — CH/MariaDB are reached via the service names
    `clickhouse:8123` and `mariadb:3306`, no host port forwarding required.
    Triggered automatically when env var `E2E_RUN_MODE=docker` is set (the
    runner image sets it). See compose/docker-compose.runner.yml.
    """

    # Filesystem
    repo_root: Path
    migrations_dir: Path
    dbt_project_dir: Path
    analytics_api_manifest_dir: Path

    # Runtime mode (where pytest runs relative to CH/MariaDB)
    run_mode: str = "host"  # "host" | "docker"

    # ClickHouse — root stack's container ports (8123 HTTP / 9000 native).
    ch_host: str = "127.0.0.1"
    ch_http_port: int = 8123
    ch_native_port: int = 9000
    ch_database: str = "insight"
    ch_user: str = _DEFAULT_DB_USER
    ch_password: str = _DEFAULT_DB_PASSWORD

    # MariaDB
    mariadb_host: str = "127.0.0.1"
    mariadb_port: int = 3306
    mariadb_database: str = "analytics"
    mariadb_user: str = _DEFAULT_DB_USER
    mariadb_password: str = _DEFAULT_DB_PASSWORD

    @classmethod
    def from_env(cls) -> "SessionConfig":
        repo_root = Path(os.environ.get("INSIGHT_REPO_ROOT", _REPO_ROOT)).resolve()
        run_mode = os.environ.get("E2E_RUN_MODE", "host")

        # Connection identity is shared across both modes: it must match the
        # root stack the runner attaches to. Defaults track .env.compose.example.
        common = dict(
            repo_root=repo_root,
            migrations_dir=repo_root / "src/ingestion/scripts/migrations",
            dbt_project_dir=repo_root / "src/ingestion/dbt",
            analytics_api_manifest_dir=repo_root / "src/backend/services/analytics-api",
            ch_user=os.environ.get("E2E_CH_USER", _DEFAULT_DB_USER),
            ch_password=os.environ.get("E2E_CH_PASSWORD", _DEFAULT_DB_PASSWORD),
            mariadb_user=os.environ.get("E2E_MARIADB_USER", _DEFAULT_DB_USER),
            mariadb_password=os.environ.get("E2E_MARIADB_PASSWORD", _DEFAULT_DB_PASSWORD),
            mariadb_database=os.environ.get("E2E_MARIADB_DATABASE", "analytics"),
        )

        if run_mode == "docker":
            # In-network: reach CH/MariaDB by their root service names + container
            # ports. The runner override supplies these from the root .env.compose.
            return cls(
                **common,
                run_mode="docker",
                ch_host=os.environ.get("E2E_CH_HOST", "clickhouse"),
                ch_http_port=int(os.environ.get("E2E_CH_HTTP_PORT", "8123")),
                ch_native_port=int(os.environ.get("E2E_CH_NATIVE_PORT", "9000")),
                mariadb_host=os.environ.get("E2E_MARIADB_HOST", "mariadb"),
                mariadb_port=int(os.environ.get("E2E_MARIADB_PORT", "3306")),
            )

        # host mode: attach to the root stack's published loopback ports. Fall
        # back to the root compose's own port vars so a customised .env.compose
        # is honoured when the developer exports it.
        return cls(
            **common,
            run_mode="host",
            ch_host=os.environ.get("E2E_CH_HOST", "127.0.0.1"),
            ch_http_port=int(
                os.environ.get("E2E_CH_HTTP_PORT", os.environ.get("CLICKHOUSE_HTTP_PORT", "8123"))
            ),
            ch_native_port=int(
                os.environ.get("E2E_CH_NATIVE_PORT", os.environ.get("CLICKHOUSE_NATIVE_PORT", "9000"))
            ),
            mariadb_host=os.environ.get("E2E_MARIADB_HOST", "127.0.0.1"),
            mariadb_port=int(os.environ.get("E2E_MARIADB_PORT", os.environ.get("MARIADB_PORT", "3306"))),
        )

    @property
    def ch_http_url(self) -> str:
        return f"http://{self.ch_host}:{self.ch_http_port}"

    @property
    def mariadb_dsn(self) -> str:
        """SeaORM / SQLAlchemy-style URL for analytics-api."""
        return (
            f"mysql://{self.mariadb_user}:{self.mariadb_password}"
            f"@{self.mariadb_host}:{self.mariadb_port}/{self.mariadb_database}"
        )
