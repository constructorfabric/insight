"""docker compose lifecycle: up, healthcheck wait, down.

Wraps `docker compose` as a subprocess. We deliberately do not use the Python
docker SDK — the compose CLI is what developers use directly when debugging,
and keeping the interface symmetrical means a failure in tests is reproducible
by hand.
"""

from __future__ import annotations

import logging
import subprocess
import time
from typing import Mapping

from e2e_lib.config import SessionConfig

LOG = logging.getLogger("e2e.compose")


class ComposeError(RuntimeError):
    pass


def up(cfg: SessionConfig) -> None:
    """Bring up the compose stack and wait until both services report healthy.

    Idempotent: if containers are already running with the same name, compose
    will simply attach to them.
    """
    LOG.info("docker compose up")
    _run(cfg, ["up", "-d", "--quiet-pull"])
    _wait_healthy(cfg, services=["clickhouse", "mariadb"], timeout_s=90.0)


def down(cfg: SessionConfig, *, remove_volumes: bool = True) -> None:
    """Tear the compose stack down."""
    LOG.info("docker compose down (volumes=%s)", remove_volumes)
    args = ["down"]
    if remove_volumes:
        args.append("-v")
    _run(cfg, args, check=False)


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
    return ["docker", "compose", "-f", str(cfg.compose_dir / "docker-compose.yml")]


def _compose_env(cfg: SessionConfig) -> Mapping[str, str]:
    import os

    env = dict(os.environ)
    env.update(cfg.compose_env())
    return env


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
