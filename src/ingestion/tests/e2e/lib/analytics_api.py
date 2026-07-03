"""analytics-api binary lifecycle: build, spawn, health-check, terminate.

We build once per session (cargo's incremental compile keeps it fast across
sessions) and spawn the binary directly on the host (per DESIGN §4: a host
binary keeps target/ warm and avoids container I/O on the cargo hot path).

analytics-api runs auth-disabled (auth happens at the API Gateway, which we
bypass), so the gears host injects a default-tenant SecurityContext. `/health`
is served by the api-gateway host gear (public, off the tenant path). For data
routes the harness sends `X-Insight-Tenant-Id: config.TEST_TENANT_ID`, which the
tenant-override layer honors, and `metric_seed.seed_test_metrics` re-homes the
seeded metric definitions onto that tenant. The header is harmless on /health.
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
        # analytics-api is now a gears-rust host (toolkit::bootstrap::run_server):
        # the `api-gateway` system gear is the REST host and structural config
        # (gears list, auth_disabled, resolvers, grpc-hub) lives in the checked-in
        # config file, which we pass with `-c`. Per-run values are injected as
        # `APP__*` env overrides (direct Popen execve preserves the hyphenated
        # gear-name segments, unlike the compose sh entrypoint).
        config_path = self.cfg.analytics_api_manifest_dir / "config" / "insight.yaml"
        # bind_addr: loopback-only (PRD cpt-bronze-to-api-e2e-constraint-loopback-only).
        # The REST host bind belongs to the api-gateway gear, so override it there.
        bind_addr = f"127.0.0.1:{self.port}"
        env.update(
            {
                "APP__gears__api-gateway__config__bind_addr": bind_addr,
                # Per-spawn UDS so parallel workers / re-runs don't collide on
                # the config's fixed grpc-hub socket path.
                "APP__gears__grpc-hub__config__listen_addr": f"uds:///tmp/analytics-grpc-{self.port}.sock",
                "APP__gears__analytics_api__config__database_url": self.cfg.mariadb_dsn,
                "APP__gears__analytics_api__config__clickhouse_url": self.cfg.ch_http_url,
                "APP__gears__analytics_api__config__clickhouse_database": self.cfg.ch_database,
                "APP__gears__analytics_api__config__clickhouse_user": self.cfg.ch_user,
                "APP__gears__analytics_api__config__clickhouse_password": self.cfg.ch_password,
                # Single-tenant catalog-resolution hint. Under auth_disabled the
                # host injects DEFAULT_TENANT_ID; the rig additionally sends
                # `X-Insight-Tenant-Id: TEST_TENANT_ID` on every request, which
                # the tenant-override layer honors. Platform metric definitions
                # seed under GLOBAL_TENANT (nil) and stay visible via
                # `InsightTenantId IN [tenant, nil]`, so this need not match the
                # seeded bronze tenant.
                "APP__gears__analytics_api__config__metric_catalog__tenant_default_id": "00000000-0000-0000-0000-000000000001",
                # No identity_url / redis_url — leave config defaults (empty).
                "RUST_LOG": env.get("RUST_LOG", "info"),
            },
        )
        LOG.info("spawning analytics-api (gears host) on 127.0.0.1:%d", self.port)
        self._proc = subprocess.Popen(
            [str(self.binary), "-c", str(config_path), "run"],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        # The gears host inits every system gear + runs migrations/probes, so
        # allow a little more startup headroom than the old bare axum server.
        self._wait_healthy(timeout_s=60.0)

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

        Every request carries `X-Insight-Tenant-Id` so the tenant-override layer
        pins data routes to the test tenant. `/health` (host gear) ignores it.
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
