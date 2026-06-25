"""docker compose lifecycle for the e2e data tier.

The e2e runner ATTACHES to the repo-root docker-compose.yml stack rather than
owning its own ClickHouse + MariaDB. This module only matters in host mode
(pytest on the host): it brings up the root stack's `clickhouse` + `mariadb`
services — and only those — so a developer iterating on the host has the data
tier available. It is idempotent: if a dev stack (`./dev-compose.sh up`) is
already running, compose attaches to it.

It deliberately does NOT tear anything down: the data tier belongs to the root
stack. Stop it with `./dev-compose.sh down` when you are finished.

In docker mode the runner's own compose dependencies start CH/MariaDB before
pytest runs, so this module is unused there.

We wrap the `docker compose` CLI as a subprocess (not the Python docker SDK) so
a failure in tests is reproducible by hand with the same command.
"""

from __future__ import annotations

import logging
import os
import subprocess
import time
from pathlib import Path
from typing import Mapping

from e2e_lib.config import SessionConfig

LOG = logging.getLogger("e2e.compose")

# The e2e data tier is always the LOCAL clickhouse + mariadb (never the
# *_EXTERNAL profiles), so we enable both profiles unconditionally and target
# only those two services — never the backend/frontend services in the root
# compose, which have no profile and would otherwise be pulled in.
_DATA_TIER = ["clickhouse", "mariadb"]


class ComposeError(RuntimeError):
    pass


def up(cfg: SessionConfig) -> None:
    """Bring up the root stack's data tier and wait until both report healthy.

    Idempotent: attaches to an already-running dev stack with the same project.
    """
    LOG.info("docker compose up (clickhouse + mariadb)")
    _run(cfg, ["up", "-d", "--quiet-pull", *_DATA_TIER])
    _wait_healthy(cfg, services=_DATA_TIER, timeout_s=90.0)


def logs(cfg: SessionConfig, service: str, *, tail: int = 100) -> str:
    """Capture recent logs for a service — for failure diagnostics."""
    result = subprocess.run(
        _compose_cmd(cfg) + ["logs", "--tail", str(tail), service],
        env=_compose_env(cfg),
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    return result.stdout + result.stderr


def _run(
    cfg: SessionConfig,
    args: list[str],
    *,
    check: bool = True,
    timeout: float = 180.0,
) -> subprocess.CompletedProcess[str]:
    cmd = _compose_cmd(cfg) + args
    LOG.debug("running: %s", " ".join(cmd))
    result = subprocess.run(
        cmd,
        env=_compose_env(cfg),
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    if check and result.returncode != 0:
        raise ComposeError(
            f"docker compose {' '.join(args)} failed (exit={result.returncode}):\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result


def _compose_cmd(cfg: SessionConfig) -> list[str]:
    """`docker compose` invocation against the repo-root stack's data tier.

    Uses the root compose file plus the committed, test-specific env file
    (`compose/e2e.env`, override via `E2E_ENV_FILE`) — the same file ../e2e.sh
    uses — so creds and ports line up with a running `./dev-compose.sh up`.
    """
    root_compose = cfg.repo_root / "docker-compose.yml"
    env_override = os.environ.get("E2E_ENV_FILE")
    env_file = (
        Path(env_override)
        if env_override
        else cfg.repo_root / "src/ingestion/tests/e2e/compose/e2e.env"
    )
    return [
        "docker", "compose",
        "--env-file", str(env_file),
        "-f", str(root_compose),
        "--profile", "local-clickhouse",
        "--profile", "local-mariadb",
    ]


def _compose_env(cfg: SessionConfig) -> Mapping[str, str]:
    # Credentials/ports are supplied to the root stack via --env-file; we only
    # pass the ambient environment through (e.g. DOCKER_HOST, PATH).
    return dict(os.environ)


def _wait_healthy(cfg: SessionConfig, services: list[str], timeout_s: float) -> None:
    deadline = time.monotonic() + timeout_s
    pending = set(services)
    while pending and time.monotonic() < deadline:
        still_pending = set()
        for svc in pending:
            if _is_healthy(cfg, svc):
                LOG.info("service %s is healthy", svc)
            else:
                still_pending.add(svc)
        pending = still_pending
        if pending:
            time.sleep(1.0)
    if pending:
        for svc in pending:
            LOG.error("service %s did not become healthy in %ss; recent logs:\n%s",
                      svc, timeout_s, logs(cfg, svc))
        raise ComposeError(f"services not healthy in {timeout_s}s: {sorted(pending)}")


def _is_healthy(cfg: SessionConfig, service: str) -> bool:
    """Returns True iff `docker compose ps` reports the service as `(healthy)`."""
    result = subprocess.run(
        _compose_cmd(cfg) + ["ps", "--format", "json", service],
        env=_compose_env(cfg),
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )
    if result.returncode != 0:
        return False
    # `docker compose ps --format json` emits NDJSON (one container per line)
    import json

    for line in result.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        # Health field is "healthy" | "unhealthy" | "starting" | "" (no healthcheck)
        if entry.get("Health") == "healthy":
            return True
    return False
