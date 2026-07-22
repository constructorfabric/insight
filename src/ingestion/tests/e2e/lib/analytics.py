"""analytics binary lifecycle: build, spawn, health-check, terminate.

We build once per session (cargo's incremental compile keeps it fast across
sessions) and spawn the binary directly on the host (per DESIGN §4: a host
binary keeps target/ warm and avoids container I/O on the cargo hot path).

analytics runs auth-ENABLED (NGINX_BFF R1): the `cf-gears-oidc-authn-plugin`
verifies a signed ES256 gateway JWT on every data route and maps its `tenant_id`
claim to the request tenant. The rig mints those JWTs and serves the JWKS +
OIDC discovery over a self-signed TLS front (see lib/gateway_jwt.py); the removed
`X-Insight-Tenant-Id` header is gone. `/health` is public (host gear, no auth).
`metric_seed.seed_test_metrics` re-homes the seeded metric definitions onto
`config.TEST_TENANT_ID`, which is the tenant the harness's default bearer carries.
"""

from __future__ import annotations

import json
import logging
import os
import shutil
import socket
import subprocess
import tempfile
import time
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import httpx
import yaml

from lib import api_coverage
from lib.config import TEST_TENANT_ID, SessionConfig
from lib.gateway_jwt import AUDIENCE, GatewayAuth

LOG = logging.getLogger("e2e.api")

# Seconds to wait for analytics's `/health` after spawn. The gears host binds
# its HTTP listener only AFTER initialising every system gear — MariaDB connect,
# sea-orm migrations, metric-catalog seed, boot probes (`/health` itself answers
# 200 immediately once bound). On a cold first run — a freshly-initialised
# MariaDB container, an unwarmed InnoDB buffer pool — that pre-bind work
# routinely overruns a tight gate, so the /health poll sees "connection
# refused" the whole time and the fixture errors before the API ever comes up.
# The wait returns the instant /health answers, so a generous ceiling costs
# nothing on the warm path and only absorbs cold-start jitter. Override on an
# especially slow host (e.g. a constrained CI runner) via the env var.
_HEALTH_TIMEOUT_S = float(
    os.environ.get(
        "E2E_API_HEALTH_TIMEOUT_S", "120"
    )  # RULE-DEFAULTS-OK: rig readiness ceiling, not a data-config input
)


@dataclass(frozen=True)
class ApiResponse:
    """Deserialized analytics response.

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
    def from_httpx(cls, response: httpx.Response) -> ApiResponse:
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
        return cls(status_code=response.status_code, items=items, page_info=page_info, raw=body)


class ApiSpawnError(RuntimeError):
    pass


def locate_binary(cfg: SessionConfig) -> Path:
    """Locate the analytics binary baked into the runner image.

    The rig no longer compiles analytics. The binary is built FROM ITS OWN
    Dockerfile (`src/backend/services/analytics/Dockerfile`, the same one that
    ships the prod image — no build-recipe duplication) and baked onto PATH at
    `/usr/local/bin/analytics` via docker-compose.runner.yml `additional_contexts`
    + a Dockerfile.runner `COPY --from=analytics …`. Same pattern as the connector
    enrich binaries (see lib/enrich.py).

    Falls back to a PATH lookup and a host-mode cargo target (for running pytest
    directly on the host with a manual `cargo build`), then fails clearly.
    """
    candidates: list[Path] = []
    which = shutil.which("analytics")
    if which:
        candidates.append(Path(which))
    candidates.append(Path("/usr/local/bin/analytics"))  # baked into the runner image
    candidates.append(cfg.repo_root / "src/backend/target/release/analytics")  # host-mode manual build
    for c in candidates:
        if c.exists():
            LOG.info("using analytics binary at %s", c)
            return c
    raise ApiSpawnError(
        "analytics binary not found — it should be baked into the runner image at "
        "/usr/local/bin/analytics (docker-compose.runner.yml `analytics` service "
        "+ Dockerfile.runner COPY --from). Rebuild with `./e2e.sh build`."
    )


def find_free_port() -> int:
    """Ask the kernel for a currently-unused TCP port on loopback."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class AnalyticsProcess:
    """A spawned, health-checked analytics process bound to loopback."""

    def __init__(self, cfg: SessionConfig, binary: Path, port: int, identity_url: str = ""):
        self.cfg = cfg
        self.binary = binary
        self.port = port
        # Identity base URL for GET /v1/persons/{email}. Empty → the handler 500s
        # ("identity not configured"); the rig passes the in-process stub's URL so
        # the endpoint exercises its real 200/404 contract (#1691).
        self.identity_url = identity_url
        # In docker mode the pytest process and the binary live in the same
        # container, so localhost is the same loopback either way.
        self.base_url = f"http://127.0.0.1:{port}"
        # Gateway-JWT auth (NGINX_BFF R1): mints per-tenant ES256 bearers and
        # serves the JWKS/discovery over a self-signed TLS front the plugin
        # trusts. Started on construction; torn down in `stop()`.
        self.auth = GatewayAuth()
        self._rig_config_path: Path | None = None
        self._proc: subprocess.Popen[str] | None = None
        # Child stdout+stderr stream to a file (not a PIPE) so the startup log
        # can be tailed into an error message even while the process is still
        # running — a blocking read on a live PIPE would hang, which is why the
        # old health-timeout path surfaced no logs at all.
        self._log_fh: Any = None
        self._log_path: Path | None = None

    def _write_rig_config(self) -> Path:
        """Write a per-spawn gears host config with the oidc-authn-plugin wired
        to this rig's TLS discovery front (issuer + self-signed CA). Leaf values
        (DB/ClickHouse/bind) still come from `APP__*` env overrides."""
        cfg = {
            "server": {"home_dir": "/tmp"},
            "logging": {"default": {"console_level": "info"}},
            "gears": {
                "api-gateway": {
                    "config": {
                        "bind_addr": f"127.0.0.1:{self.port}",
                        "enable_docs": True,
                        "cors_enabled": True,
                        # Auth ENABLED: the plugin verifies the signed gateway JWT.
                        "auth_disabled": False,
                    }
                },
                "gear-orchestrator": {"config": {}},
                "grpc-hub": {"config": {"listen_addr": "uds:///tmp/analytics-grpc.sock"}},
                "authn-resolver": {"config": {"vendor": "hyperspot"}},
                "oidc-authn-plugin": {
                    "config": {
                        "vendor": "hyperspot",
                        "priority": 50,
                        "jwt": {
                            "supported_algorithms": ["ES256"],
                            "clock_skew_leeway": "60s",
                            "require_audience": True,
                            "expected_audience": [AUDIENCE],
                            "trusted_issuers": [{"issuer": self.auth.issuer}],
                            "claim_mapping": {
                                "subject_id": "sub",
                                "subject_tenant_id": "tenant_id",
                                "subject_type": "sub_type",
                                "token_scopes": "roles",
                            },
                            "required_claims": [],
                        },
                        "http_client": {"request_timeout": "5s", "custom_ca_certificate_paths": [self.auth.ca_path]},
                        "s2s_oauth": {
                            "discovery_url": self.auth.issuer,
                            "default_subject_type": "service",
                            "token_cache": {"ttl": "300s", "max_entries": 100},
                        },
                    }
                },
                "authz-resolver": {"config": {"vendor": "hyperspot"}},
                "static-authz-plugin": {"config": {"vendor": "hyperspot", "priority": 100}},
                "tenant-resolver": {"config": {"vendor": "hyperspot"}},
                "single-tenant-tr-plugin": {"config": {"vendor": "hyperspot", "priority": 20}},
                "analytics": {
                    "config": {
                        "database_url": "",
                        "clickhouse_url": "",
                        "clickhouse_database": "insight",
                        "identity_url": "",
                        "redis_url": "",
                        "metric_catalog": {"tenant_default_id": None},
                    }
                },
            },
        }
        fh = tempfile.NamedTemporaryFile(  # noqa: SIM115 — path used by the spawned binary
            mode="w", suffix=".yaml", prefix=f"analytics-cfg-{self.port}-", delete=False
        )
        yaml.safe_dump(cfg, fh, sort_keys=False)
        fh.close()
        self._rig_config_path = Path(fh.name)
        return self._rig_config_path

    def start(self) -> None:
        env = os.environ.copy()
        # analytics is now a gears-rust host (toolkit::bootstrap::run_server):
        # the `api-gateway` system gear is the REST host. This rig runs analytics
        # auth-ENABLED (NGINX_BFF R1): tenant comes from a signed gateway JWT, not
        # a header. The oidc-authn-plugin verification config (issuer + self-
        # signed CA of `self.auth`'s TLS discovery front) is nested/list-shaped
        # and not env-injectable, so it is written into a per-spawn config file
        # passed with `-c`. Per-run scalar values are injected as `APP__*` env
        # overrides (direct Popen execve preserves the hyphenated gear-name
        # segments, unlike the compose sh entrypoint).
        config_path = self._write_rig_config()
        # bind_addr: loopback-only (PRD cpt-bronze-to-api-e2e-constraint-loopback-only).
        # The REST host bind belongs to the api-gateway gear, so override it there.
        bind_addr = f"127.0.0.1:{self.port}"
        env.update(
            {
                "APP__gears__api-gateway__config__bind_addr": bind_addr,
                # Per-spawn UDS so parallel workers / re-runs don't collide on
                # the config's fixed grpc-hub socket path.
                "APP__gears__grpc-hub__config__listen_addr": f"uds:///tmp/analytics-grpc-{self.port}.sock",
                "APP__gears__analytics__config__database_url": self.cfg.mariadb_dsn,
                "APP__gears__analytics__config__clickhouse_url": self.cfg.ch_http_url,
                "APP__gears__analytics__config__clickhouse_database": self.cfg.ch_database,
                "APP__gears__analytics__config__clickhouse_user": self.cfg.ch_user,
                "APP__gears__analytics__config__clickhouse_password": self.cfg.ch_password,
                # Single-tenant catalog-resolution hint. The request tenant comes
                # from the signed JWT's `tenant_id`; platform metric definitions
                # seed under GLOBAL_TENANT (nil) and stay visible via
                # `InsightTenantId IN [tenant, nil]`, so this need not match the
                # seeded bronze tenant.
                "APP__gears__analytics__config__metric_catalog__tenant_default_id": "00000000-0000-0000-0000-000000000001",
                # No redis_url — leave config default (empty).
                "RUST_LOG": env.get("RUST_LOG", "info"),
            }
        )
        # Only override identity_url when the rig wired a stub (see __init__).
        if self.identity_url:
            env["APP__gears__analytics__config__identity_url"] = self.identity_url
        # Stream the binary's stdout+stderr to a temp file rather than a PIPE so
        # `_read_log_tail` can surface it on a health-timeout (process still
        # alive → a PIPE read would block forever). Cleaned up in `stop()`.
        self._log_fh = tempfile.NamedTemporaryFile(  # noqa: SIM115 — handle lives until stop()
            mode="w", suffix=".log", prefix=f"analytics-{self.port}-", delete=False
        )
        self._log_path = Path(self._log_fh.name)
        LOG.info("spawning analytics (gears host) on 127.0.0.1:%d (startup log: %s)", self.port, self._log_path)
        self._proc = subprocess.Popen(
            [str(self.binary), "-c", str(config_path), "run"],
            env=env,
            stdout=self._log_fh,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self._wait_healthy(timeout_s=_HEALTH_TIMEOUT_S)

    def stop(self) -> None:
        if self._proc is not None:
            LOG.info("terminating analytics (pid=%d)", self._proc.pid)
            self._proc.terminate()
            try:
                self._proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                LOG.warning("analytics did not exit on SIGTERM; killing")
                self._proc.kill()
                self._proc.wait(timeout=5)
            self._proc = None
        # Close + remove the startup-log temp file (best-effort; runs even when
        # the process never started so a failed `start()` leaks nothing).
        if self._log_fh is not None:
            try:
                self._log_fh.close()
            except (OSError, ValueError):
                pass
            self._log_fh = None
        if self._log_path is not None:
            try:
                self._log_path.unlink(missing_ok=True)
            except OSError:
                pass
            self._log_path = None
        # Tear down the gateway-JWT TLS front + its per-spawn config file.
        self.auth.stop()
        if self._rig_config_path is not None:
            try:
                self._rig_config_path.unlink(missing_ok=True)
            except OSError:
                pass
            self._rig_config_path = None

    def is_running(self) -> bool:
        return self._proc is not None and self._proc.poll() is None

    def bearer(self, tenant_id: str) -> str:
        """A signed gateway JWT scoped to `tenant_id` (for cross-tenant cases)."""
        return self.auth.mint(tenant_id)

    def client(self) -> httpx.Client:
        """Return an httpx.Client bound to this process's base URL.

        Every request carries a signed gateway JWT (Authorization: Bearer) whose
        `tenant_id` claim is the test tenant — analytics verifies it and maps the
        claim to the request tenant (NGINX_BFF R1). `/health` is public.
        """
        return httpx.Client(
            base_url=self.base_url,
            timeout=30.0,
            headers=self.auth.auth_header(str(TEST_TENANT_ID)),
            # Record every (method, path, status) the suite exercises so the
            # endpoint-coverage gate (lib/api_coverage.py) can diff it
            # against the OpenAPI spec. This is THE chokepoint — metric tests
            # (call_request) and smoke tests both go through client().
            event_hooks={"response": [api_coverage.record_response]},
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

    def _read_log_tail(self, limit: int = 4000) -> str:
        """Return the tail of the child's captured stdout+stderr.

        Reads the on-disk log file (not the PIPE), so it works whether the
        process has already exited or is still running — the latter is the
        health-timeout case the old PIPE-based read could not surface without
        blocking on a pipe that never reaches EOF.
        """
        if self._log_fh is not None:
            try:
                self._log_fh.flush()
            except (OSError, ValueError):
                pass
        if self._log_path is None:
            return ""
        try:
            return self._log_path.read_text(errors="replace")[-limit:]
        except OSError:
            return ""

    def _wait_healthy(self, *, timeout_s: float) -> None:
        deadline = time.monotonic() + timeout_s
        last_err: Exception | None = None
        while time.monotonic() < deadline:
            if not self.is_running():
                code = self._proc.returncode if self._proc else "?"
                raise ApiSpawnError(f"analytics exited during startup (code={code}):\n{self._read_log_tail()}")
            try:
                with httpx.Client(base_url=self.base_url, timeout=2.0) as c:
                    r = c.get("/health")  # public host-gear route; no auth needed
                    if r.status_code == 200:
                        LOG.info("analytics is healthy at %s", self.base_url)
                        return
            except Exception as e:
                last_err = e
            time.sleep(0.5)
        # Timed out with the process still alive — almost always cold-start
        # latency (raise the ceiling via E2E_API_HEALTH_TIMEOUT_S), occasionally
        # a wedged boot. Surface the binary's own log either way — the old code
        # swallowed it on this path, leaving only an opaque "connection refused".
        raise ApiSpawnError(
            f"analytics did not become healthy in {timeout_s:.0f}s; "
            f"last error: {last_err}\n"
            f"--- analytics startup log (tail) ---\n{self._read_log_tail()}"
        )


@contextmanager
def spawn(cfg: SessionConfig):
    """Context manager: locate the binary, spawn, yield, stop."""
    binary = locate_binary(cfg)
    port = find_free_port()
    proc = AnalyticsProcess(cfg, binary, port)
    proc.start()
    try:
        yield proc
    finally:
        proc.stop()
