"""identity service lifecycle for the contract suite (issue #1753).

The identity service is the system-under-test here (unlike lib/identity_stub.py,
which fakes it FOR analytics). The suite is implementation-agnostic: today the
harness boots the .NET `Insight.Identity.Api` published app (baked into the
runner image, see compose/Dockerfile.runner); when the Rust `identity-resolution`
port takes over, only this module's spawn path changes — the tests target
whatever answers the base URL. `E2E_IDENTITY_URL` short-circuits the spawn
entirely and points the suite at an already-running deployment.

Auth is the same gateway-JWT rig analytics uses (lib/gateway_jwt.py): the .NET
service verifies the ES256 JWT against the rig's JWKS — fetched from the rig's
plain-HTTP twin (the .NET verifier takes an explicit `auth_gateway_jwks_url`
with RequireHttpsMetadata=false, no CA plumbing needed). Claims contract:
`sub` = the CALLER's person_id, `tenant_id` = the sole tenant authority,
`sub_type` user|service, `roles` scopes.

The service self-migrates its `identity` MariaDB database at startup (DbUp),
so the harness only has to CREATE DATABASE + GRANT (root creds from the compose
.env) before the spawn; the seed fixture (lib/identity_seed.py) fills it after
the first /health.
"""

from __future__ import annotations

import logging
import os
import shutil
import subprocess
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path
from typing import Any

import httpx
import pymysql

from lib import api_coverage
from lib.analytics import ApiSpawnError, find_free_port
from lib.config import TEST_TENANT_ID, SessionConfig
from lib.gateway_jwt import GatewayAuth

LOG = logging.getLogger("e2e.identity")

# The identity service owns its own MariaDB database — separate from the
# analytics one the compose stack pre-creates.
IDENTITY_DATABASE = "identity_e2e"

_HEALTH_TIMEOUT_S = float(
    os.environ.get(
        "E2E_IDENTITY_HEALTH_TIMEOUT_S", "120"
    )  # RULE-DEFAULTS-OK: rig readiness ceiling, not a data-config input
)


def locate_app(cfg: SessionConfig) -> list[str]:
    """Resolve the spawn command for the identity implementation under test.

    The .NET published app is baked into the runner image at
    /opt/insight-identity (compose/Dockerfile.runner COPY --from=identity, plus
    the aspnetcore runtime). Host-mode fallback: a manual `dotnet publish`
    output under src/backend/services/identity/publish/.
    """
    candidates = [
        Path("/opt/insight-identity/Insight.Identity.Api.dll"),  # runner image
        cfg.repo_root / "src/backend/services/identity/publish/Insight.Identity.Api.dll",
    ]
    dotnet = shutil.which("dotnet")
    for dll in candidates:
        if dll.exists():
            if not dotnet:
                raise ApiSpawnError(f"found {dll} but no `dotnet` runtime on PATH")
            LOG.info("using identity app at %s", dll)
            return [dotnet, str(dll)]
    raise ApiSpawnError(
        "identity app not found — it should be baked into the runner image at "
        "/opt/insight-identity (docker-compose.runner.yml `identity` service + "
        "Dockerfile.runner COPY --from). Rebuild with `./e2e.sh build`."
    )


def identity_dsn(cfg: SessionConfig) -> str:
    """MariaDB URL for the identity database (the service's own DB)."""
    return (
        f"mysql://{cfg.mariadb_user}:{cfg.mariadb_password}"
        f"@{cfg.mariadb_host}:{cfg.mariadb_port}/{IDENTITY_DATABASE}"
    )


def create_identity_database(cfg: SessionConfig) -> None:
    """CREATE DATABASE + GRANT for the identity service (root creds).

    Mirrors the umbrella's mariadb-init-svcdbs Hook Job: the service expects
    its (empty) database to exist and self-migrates the schema on boot.
    Idempotent — IF NOT EXISTS + re-GRANT are no-ops on re-runs.
    """
    conn = pymysql.connect(
        host=cfg.mariadb_host,
        port=cfg.mariadb_port,
        user="root",
        password=cfg.mariadb_root_password,
        charset="utf8mb4",
        autocommit=True,
    )
    try:
        with conn.cursor() as cur:
            cur.execute(
                f"CREATE DATABASE IF NOT EXISTS `{IDENTITY_DATABASE}` "
                "CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
            )
            cur.execute(f"GRANT ALL PRIVILEGES ON `{IDENTITY_DATABASE}`.* TO %s@'%%'", (cfg.mariadb_user,))
            cur.execute("FLUSH PRIVILEGES")
    finally:
        conn.close()
    LOG.info("identity database `%s` provisioned", IDENTITY_DATABASE)


class IdentityProcess:
    """A spawned, health-checked identity service bound to loopback.

    When `external_url` is set (E2E_IDENTITY_URL), nothing is spawned — the
    instance only owns the auth rig and the recording client factory.
    """

    def __init__(self, cfg: SessionConfig, port: int, external_url: str = ""):
        self.cfg = cfg
        self.port = port
        self.base_url = external_url or f"http://127.0.0.1:{port}"
        self._external = bool(external_url)
        self.auth = GatewayAuth()
        self._proc: subprocess.Popen[str] | None = None
        self._log_fh: Any = None
        self._log_path: Path | None = None

    def start(self) -> None:
        if self._external:
            LOG.info("targeting external identity at %s (no spawn)", self.base_url)
            self._wait_healthy(timeout_s=_HEALTH_TIMEOUT_S)
            return
        create_identity_database(self.cfg)
        cmd = locate_app(self.cfg)
        env = os.environ.copy()
        env.update(
            {
                # Loopback-only bind (cpt-bronze-to-api-e2e-constraint-loopback-only).
                "ASPNETCORE_URLS": self.base_url,
                "IDENTITY__identity__bind_addr": f"127.0.0.1:{self.port}",
                "IDENTITY__mariadb__url": identity_dsn(self.cfg),
                # Gateway-JWT verification: issuer = the rig's TLS front (the
                # token `iss`, validated as a string); JWKS from the rig's
                # plain-HTTP twin (explicit URL, RequireHttpsMetadata=false).
                "IDENTITY__identity__auth_gateway_issuer": self.auth.issuer,
                "IDENTITY__identity__auth_gateway_jwks_url": self.auth.http_jwks_url,
                # ClickHouse for persons-seed (identity.identity_inputs). The
                # .NET service speaks the NATIVE protocol (Octonica, port 9000)
                # — not the HTTP port analytics uses.
                "IDENTITY__clickhouse__host": self.cfg.ch_host,
                "IDENTITY__clickhouse__port": str(self.cfg.ch_native_port),
                "IDENTITY__clickhouse__user": self.cfg.ch_user,
                "IDENTITY__clickhouse__password": self.cfg.ch_password,
                "IDENTITY__clickhouse__database": self.cfg.ch_database,
                "DOTNET_ENVIRONMENT": "Production",
            }
        )
        self._log_fh = tempfile.NamedTemporaryFile(  # noqa: SIM115 — handle lives until stop()
            mode="w", suffix=".log", prefix=f"identity-{self.port}-", delete=False
        )
        self._log_path = Path(self._log_fh.name)
        LOG.info("spawning identity on 127.0.0.1:%d (startup log: %s)", self.port, self._log_path)
        self._proc = subprocess.Popen(
            cmd,
            env=env,
            stdout=self._log_fh,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self._wait_healthy(timeout_s=_HEALTH_TIMEOUT_S)

    def stop(self) -> None:
        if self._proc is not None:
            LOG.info("terminating identity (pid=%d)", self._proc.pid)
            self._proc.terminate()
            try:
                self._proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                LOG.warning("identity did not exit on SIGTERM; killing")
                self._proc.kill()
                self._proc.wait(timeout=5)
            self._proc = None
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
        self.auth.stop()

    def is_running(self) -> bool:
        if self._external:
            return True
        return self._proc is not None and self._proc.poll() is None

    # -- tokens ------------------------------------------------------------

    def bearer(self, *, sub: str, tenant: str = str(TEST_TENANT_ID), sub_type: str = "user") -> str:
        """A signed gateway JWT for caller `sub` (a person_id) in `tenant`."""
        return self.auth.mint(tenant, sub=sub, sub_type=sub_type)

    def client(self, *, sub: str, tenant: str = str(TEST_TENANT_ID)) -> httpx.Client:
        """Recording httpx client authenticated as person `sub` in `tenant`.

        Every response is recorded into the IDENTITY coverage ledger (separate
        from the analytics one — the coverage gate is spec-scoped).
        """
        return httpx.Client(
            base_url=self.base_url,
            timeout=30.0,
            headers={"Authorization": f"Bearer {self.bearer(sub=sub, tenant=tenant)}"},
            event_hooks={"response": [api_coverage.record_identity_response]},
        )

    # -- internals -----------------------------------------------------------

    def _read_log_tail(self, limit: int = 4000) -> str:
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
                raise ApiSpawnError(f"identity exited during startup (code={code}):\n{self._read_log_tail()}")
            try:
                with httpx.Client(base_url=self.base_url, timeout=2.0) as c:
                    r = c.get("/health")  # public, no auth
                    if r.status_code == 200:
                        LOG.info("identity is healthy at %s", self.base_url)
                        return
            except Exception as e:
                last_err = e
            time.sleep(0.5)
        raise ApiSpawnError(
            f"identity did not become healthy in {timeout_s:.0f}s; "
            f"last error: {last_err}\n"
            f"--- identity startup log (tail) ---\n{self._read_log_tail()}"
        )


@contextmanager
def spawn(cfg: SessionConfig):
    """Context manager: provision DB, spawn (or attach), yield, stop."""
    external = os.environ.get("E2E_IDENTITY_URL", "")
    proc = IdentityProcess(cfg, find_free_port(), external_url=external)
    proc.start()
    try:
        yield proc
    finally:
        proc.stop()
