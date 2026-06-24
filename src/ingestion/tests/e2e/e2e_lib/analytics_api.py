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

from e2e_lib.config import SessionConfig, TENANT_HEADER, TEST_TENANT_ID

LOG = logging.getLogger("e2e.api")


def _required_cargo_version(repo_root: Path) -> tuple[int, int] | None:
    """Read the required toolchain version from the single source of truth:
    `[workspace.package].rust-version` in src/backend/Cargo.toml.

    A hardcoded constant here silently drifts behind the real requirement (it
    was pinned at 1.92 while the crates moved to 1.95), which let a broken build
    masquerade as "version OK". Reading Cargo.toml keeps the precheck honest.

    Returns None if it can't be determined — the `cargo build` itself remains
    the hard gate (it fails loudly), so the precheck is only for a nicer message.
    """
    cargo_toml = repo_root / "src/backend/Cargo.toml"
    try:
        import tomllib

        data = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))
    except (OSError, ValueError, ImportError):
        return None
    ver = (
        data.get("workspace", {}).get("package", {}).get("rust-version")
        or data.get("package", {}).get("rust-version")
    )
    if not ver:
        return None
    nums = str(ver).split(".")
    try:
        return int(nums[0]), int(nums[1])
    except (IndexError, ValueError):
        return None


def _cargo_version_at_least(cargo: str, *, major: int, minor: int) -> tuple[bool, str]:
    """Return (ok, reported_version). On parse failure: (True, "?") — assume modern."""
    try:
        proc = subprocess.run(
            [cargo, "--version"], capture_output=True, text=True, check=False, timeout=10
        )
    except (OSError, subprocess.SubprocessError):
        return True, "?"
    if proc.returncode != 0:
        return True, "?"
    # Output looks like: "cargo 1.92.0 (abcdef 2025-02-20)"
    parts = proc.stdout.strip().split()
    if len(parts) < 2:
        return True, proc.stdout.strip() or "?"
    version = parts[1]
    nums = version.split(".")
    if len(nums) < 2:
        return True, version
    try:
        cur = (int(nums[0]), int(nums[1]))
    except ValueError:
        return True, version
    return cur >= (major, minor), version


def _resolve_cargo() -> str | None:
    """Find a cargo executable.

    pytest may inherit a PATH without `~/.cargo/bin` even though the user's
    interactive shell has it (`~/.cargo/env` is sourced in .bashrc/.zshrc but
    not necessarily in the env pytest launches from). Try PATH first, then
    well-known rustup locations.
    """
    found = shutil.which("cargo")
    if found:
        return found
    home = os.environ.get("HOME") or ""
    for candidate in (
        os.environ.get("CARGO_HOME", ""),
        f"{home}/.cargo",
        "/usr/local/cargo",
    ):
        if not candidate:
            continue
        path = os.path.join(candidate, "bin", "cargo")
        if os.path.isfile(path) and os.access(path, os.X_OK):
            return path
    return None


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


def build(cfg: SessionConfig) -> Path:
    """Cargo-build the release binary; return the path to the artifact.

    Idempotent: re-running with no source changes is near-instant.

    Raises `ApiSpawnError` (not FileNotFoundError) when cargo isn't installed,
    so the `analytics_api` fixture's skip guard catches it cleanly.
    """
    cargo = _resolve_cargo()
    if cargo is None:
        raise ApiSpawnError(
            "cargo executable not found on PATH or in standard rustup locations. "
            "Install via `rustup` and ensure ~/.cargo/bin is on PATH (or set CARGO_HOME)."
        )
    version = "?"
    required = _required_cargo_version(cfg.repo_root)
    if required is not None:
        ok, version = _cargo_version_at_least(cargo, major=required[0], minor=required[1])
        if not ok:
            raise ApiSpawnError(
                f"cargo {version} is too old — src/backend/Cargo.toml requires "
                f"rust-version ≥ {required[0]}.{required[1]}. "
                f"Run `rustup update stable` and retry."
            )
    # Force cargo to recompile the analytics-api crate from the CURRENT source.
    #
    # The repo is bind-mounted into the runner; on Docker Desktop (macOS) the
    # mtimes cargo reads through that mount do not reliably advance when files
    # are edited on the host, so cargo's fingerprint check misses new/changed
    # sources (most painfully: a new SeaORM migration) and relinks a stale
    # cached object instead of recompiling. The binary then silently lacks the
    # migration — tests fail with NO_ZULIP / size off-by-one and no `down -v`
    # short of a full cold rebuild fixes it. Bumping the mtimes here (a real
    # write the container's FS layer registers) makes cargo recompile the crate
    # every run. Only the analytics-api crate is affected (it is a leaf bin —
    # nothing depends on it); its dependencies stay cached, so the cost is one
    # crate recompile (~1-2 min), not a cold build.
    crate_src = cfg.repo_root / "src/backend/services/analytics-api/src"
    touched = 0
    for rs in crate_src.rglob("*.rs"):
        rs.touch()
        touched += 1
    LOG.info("touched %d analytics-api source files to force a fresh compile", touched)
    LOG.info("cargo build --release -p analytics-api  (cargo=%s, version=%s)", cargo, version)
    try:
        result = subprocess.run(
            [cargo, "build", "--release", "-p", "analytics-api"],
            cwd=str(cfg.repo_root / "src/backend"),
            capture_output=True,
            text=True,
            check=False,
            timeout=600,
        )
    except FileNotFoundError as e:
        raise ApiSpawnError(f"cargo at {cargo} not executable: {e}") from e
    if result.returncode != 0:
        raise ApiSpawnError(
            f"cargo build failed (exit={result.returncode}):\n{result.stderr[-2000:]}"
        )
    binary = cfg.repo_root / "src/backend/target/release/analytics-api"
    if not binary.exists():
        raise ApiSpawnError(f"binary not at expected path: {binary}")
    return binary


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
