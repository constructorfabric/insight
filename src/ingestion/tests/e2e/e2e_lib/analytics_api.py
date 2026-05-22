"""analytics-api binary lifecycle: build, spawn, health-check, terminate.

We build once per session (cargo's incremental compile keeps it fast across
sessions) and spawn the binary directly on the host (per DESIGN §4: a host
binary keeps target/ warm and avoids container I/O on the cargo hot path).

The auth middleware in analytics-api is currently a stub — the binary requires
no Bearer token (auth happens at the API Gateway, which we bypass). All
requests resolve to tenant_id = nil UUID.
"""

from __future__ import annotations

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

from e2e_lib.config import SessionConfig
from e2e_lib.fixture_loader import Fixture

LOG = logging.getLogger("e2e.api")


MIN_CARGO_MAJOR = 1
MIN_CARGO_MINOR = 92  # src/backend/Cargo.toml requires `edition = "2024"`


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
    ok, version = _cargo_version_at_least(cargo, major=MIN_CARGO_MAJOR, minor=MIN_CARGO_MINOR)
    if not ok:
        raise ApiSpawnError(
            f"cargo {version} is too old — src/backend/Cargo.toml requires "
            f"edition2024 (cargo ≥ {MIN_CARGO_MAJOR}.{MIN_CARGO_MINOR}). "
            f"Run `rustup update stable` and retry."
        )
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
        """Return an httpx.Client bound to this process's base URL."""
        return httpx.Client(base_url=self.base_url, timeout=30.0)

    def call_fixture(self, fixture: Fixture) -> ApiResponse:
        """Build a request from the fixture's spec.yaml, execute it, return ApiResponse.

        Handles `{metric_id}` interpolation in the endpoint and uses the spec's
        `method` (default POST). The request body is sent as JSON.
        """
        endpoint = fixture.spec.resolved_endpoint()
        with self.client() as c:
            method = fixture.spec.method.upper()
            kwargs: dict[str, Any] = {}
            if method in ("POST", "PUT") and fixture.spec.request_body:
                kwargs["json"] = fixture.spec.request_body
            LOG.info("→ %s %s", method, endpoint)
            response = c.request(method, endpoint, **kwargs)
            LOG.info("← %d  (%d bytes)", response.status_code, len(response.content))
            return ApiResponse.from_httpx(response)

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
                with httpx.Client(base_url=self.base_url, timeout=2.0) as c:
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
