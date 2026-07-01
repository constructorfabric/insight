"""analytics-api binary lifecycle: build, spawn, health-check, terminate.

We build once per session (cargo's incremental compile keeps it fast across
sessions) and spawn the binary directly on the host (per DESIGN §4: a host
binary keeps target/ warm and avoids container I/O on the cargo hot path).

analytics-api requires no Bearer token (auth happens at the API Gateway, which
we bypass), but its tenant middleware rejects requests without a resolvable
non-nil tenant. The harness therefore sends `X-Insight-Tenant-Id` with
`config.TEST_TENANT_ID` on every request — including /health polling — and
`metric_seed.seed_test_metrics` re-homes the seeded metric definitions onto
that tenant.
"""

from __future__ import annotations

import json
import logging
import os
import shutil
import socket
import subprocess
import time
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import httpx

from lib.config import SessionConfig, TENANT_HEADER, TEST_TENANT_ID

LOG = logging.getLogger("e2e.api")


@dataclass(frozen=True)
class ApiResponse:
    """Deserialized analytics-api response.

    For metric-query endpoints the body is `{items: [...], page_info: {...}}`.
    For other endpoints (e.g. /v1/metrics) the body is a bare list — we
    normalize: `items` always holds the row-like payload, `raw` holds the
    full deserialized JSON, `page_info` is empty when the endpoint doesn't
    return pagination.
    """

    status_code: int
    items: list[dict[str, Any]]
    page_info: dict[str, Any] = field(default_factory=dict)
    raw: Any = None

    @classmethod
    def from_httpx(cls, response: httpx.Response) -> "ApiResponse":
        try:
            body = response.json() if response.content else None
        except Exception:
            body = None
        items: list[dict[str, Any]] = []
        page_info: dict[str, Any] = {}
        if isinstance(body, dict) and "items" in body:
            items = list(body.get("items") or [])
            page_info = body.get("page_info") or {}
        elif isinstance(body, list):
            items = list(body)
        return cls(
            status_code=response.status_code,
            items=items,
            page_info=page_info,
            raw=body,
        )


class ApiSpawnError(RuntimeError):
    pass


def locate_binary(cfg: SessionConfig) -> Path:
    """Locate the analytics-api binary baked into the runner image.

    The rig no longer compiles analytics-api. The binary is built FROM ITS OWN
    Dockerfile (`src/backend/services/analytics-api/Dockerfile`, the same one that
    ships the prod image — no build-recipe duplication) and baked onto PATH at
    `/usr/local/bin/analytics-api` via docker-compose.runner.yml `additional_contexts`
    + a Dockerfile.runner `COPY --from=analytics-api …`. Same pattern as the connector
    enrich binaries (see lib/enrich.py).

    Falls back to a PATH lookup and a host-mode cargo target (for running pytest
    directly on the host with a manual `cargo build`), then fails clearly.
    """
    candidates: list[Path] = []
    which = shutil.which("analytics-api")
    if which:
        candidates.append(Path(which))
    candidates.append(Path("/usr/local/bin/analytics-api"))  # baked into the runner image
    candidates.append(cfg.repo_root / "src/backend/target/release/analytics-api")  # host-mode manual build
    for c in candidates:
        if c.exists():
            LOG.info("using analytics-api binary at %s", c)
            return c
    raise ApiSpawnError(
        "analytics-api binary not found — it should be baked into the runner image at "
        "/usr/local/bin/analytics-api (docker-compose.runner.yml `analytics-api` service "
        "+ Dockerfile.runner COPY --from). Rebuild with `./e2e.sh build`."
    )


def find_free_port() -> int:
    """Ask the kernel for a currently-unused TCP port on loopback."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class AnalyticsApiProcess:
    """A spawned, health-checked analytics-api process bound to loopback."""

    def __init__(self, cfg: SessionConfig, binary: Path, port: int):
        self.cfg = cfg
        self.binary = binary
        self.port = port
        # In docker mode the pytest process and the binary live in the same
        # container, so localhost is the same loopback either way.
        self.base_url = f"http://127.0.0.1:{port}"
        self._proc: subprocess.Popen[str] | None = None

    def start(self) -> None:
        env = os.environ.copy()
        # bind_addr: 127.0.0.1 keeps the port loopback-only (PRD constraint
        # cpt-bronze-to-api-e2e-constraint-loopback-only). In docker mode the
        # pytest process is in the same container, so loopback is enough.
        bind_addr = f"127.0.0.1:{self.port}"
        env.update(
            {
                "ANALYTICS__database_url": self.cfg.mariadb_dsn,
                "ANALYTICS__clickhouse_url": self.cfg.ch_http_url,
                "ANALYTICS__clickhouse_database": self.cfg.ch_database,
                "ANALYTICS__clickhouse_user": self.cfg.ch_user,
                "ANALYTICS__clickhouse_password": self.cfg.ch_password,
                "ANALYTICS__bind_addr": bind_addr,
                # Single-tenant fallback. Since #522 the tenant_middleware
                # rejects header-less requests (including /health) with 400
                # unless a non-nil default tenant is configured. The rig sends
                # no X-Insight-Tenant-Id header, so we pin a non-nil default.
                # A non-nil value is required — ConfigTenantAuthorization::new
                # filters out a nil default. Platform metric definitions are
                # seeded under GLOBAL_TENANT (Uuid::nil()) and remain visible to
                # any resolved tenant via `InsightTenantId IN [tenant, nil]`,
                # and the data-plane queries skip tenant isolation in MVP, so
                # this default never has to match the seeded bronze tenant.
                "ANALYTICS__metric_catalog__tenant_default_id": "00000000-0000-0000-0000-000000000001",
                # No identity_url / redis_url — leave defaults (empty strings)
                "RUST_LOG": env.get("RUST_LOG", "info"),
            },
        )
        LOG.info("spawning analytics-api on 127.0.0.1:%d", self.port)
        self._proc = subprocess.Popen(
            [str(self.binary)],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self._wait_healthy(timeout_s=30.0)

    def stop(self) -> None:
        if self._proc is None:
            return
        LOG.info("terminating analytics-api (pid=%d)", self._proc.pid)
        self._proc.terminate()
        try:
            self._proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            LOG.warning("analytics-api did not exit on SIGTERM; killing")
            self._proc.kill()
            self._proc.wait(timeout=5)
        self._proc = None

    def is_running(self) -> bool:
        return self._proc is not None and self._proc.poll() is None

    def client(self) -> httpx.Client:
        """Return an httpx.Client bound to this process's base URL.

        Every request carries `X-Insight-Tenant-Id`: the tenant middleware sits
        in front of all routes (including `/health`) and rejects requests with
        no resolvable tenant, so the header is mandatory, not per-endpoint.
        """
        return httpx.Client(
            base_url=self.base_url,
            timeout=30.0,
            headers={TENANT_HEADER: str(TEST_TENANT_ID)},
        )

    def call_request(self, request: dict) -> tuple[int, Any]:
        """Execute a `case.request` ({url, method, body}). Return (status_code, json|text).

        Used by the YAML rig; the primary endpoint is the batch
        `POST /v1/metrics/queries`. The body is sent as JSON.
        """
        url = request["url"]
        method = str(request.get("method", "POST")).upper()
        body = request.get("body")
        with self.client() as c:
            kwargs: dict[str, Any] = {}
            if body is not None:
                kwargs["json"] = body
            LOG.info("→ %s %s", method, url)
            response = c.request(method, url, **kwargs)
            LOG.info("← %d  (%d bytes)", response.status_code, len(response.content))
            try:
                payload = response.json()
            except json.JSONDecodeError:
                payload = response.text
            return response.status_code, payload

    def _wait_healthy(self, *, timeout_s: float) -> None:
        deadline = time.monotonic() + timeout_s
        last_err: Exception | None = None
        while time.monotonic() < deadline:
            if not self.is_running():
                stdout = self._proc.stdout.read() if self._proc and self._proc.stdout else ""
                raise ApiSpawnError(
                    f"analytics-api exited during startup (code={self._proc.returncode if self._proc else '?'}):\n"
                    f"{stdout[-2000:]}"
                )
            try:
                with httpx.Client(
                    base_url=self.base_url,
                    timeout=2.0,
                    headers={TENANT_HEADER: str(TEST_TENANT_ID)},
                ) as c:
                    r = c.get("/health")
                    if r.status_code == 200:
                        LOG.info("analytics-api is healthy at %s", self.base_url)
                        return
            except Exception as e:
                last_err = e
            time.sleep(0.5)
        raise ApiSpawnError(
            f"analytics-api did not become healthy in {timeout_s}s; last error: {last_err}"
        )


@contextmanager
def spawn(cfg: SessionConfig):
    """Context manager: build (if needed), spawn, yield, stop."""
    binary = build(cfg)
    port = find_free_port()
    proc = AnalyticsApiProcess(cfg, binary, port)
    proc.start()
    try:
        yield proc
    finally:
        proc.stop()
