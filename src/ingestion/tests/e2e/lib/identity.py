"""identity service lifecycle for the contract suite (issue #1753).

The identity service is the system-under-test here (unlike lib/identity_stub.py,
which fakes it FOR analytics). The suite is implementation-agnostic; the
implementation is selected EXPLICITLY via `E2E_IDENTITY_IMPLEMENTATION`
(default `dotnet`):

  dotnet  — the .NET `Insight.Identity.Api` published app baked into the
            runner image (compose/Dockerfile.runner). Self-migrates its
            schema at startup (DbUp).
  rust    — the `identity-resolution` binary. The harness runs its
            `migrate` subcommand first (the Rust service does not migrate at
            server start), then boots the server with an analytics-style
            gears rig config. Requires the binary baked into the runner image
            (a build-only compose service + COPY, added together with the
            Rust service's Dockerfile on the cutover branch).

Both run against the same throwaway `identity_e2e` MariaDB database and the
same seed — one command per implementation, no test-file changes:

    E2E_IDENTITY_IMPLEMENTATION=dotnet ./e2e.sh test identity/
    E2E_IDENTITY_IMPLEMENTATION=rust   ./e2e.sh test identity/

Implementation-specific surface (e.g. the deprecated persons lookup the Rust
port dropped) is gated by the explicit `capabilities` of the selection —
NEVER probed from the service's runtime behavior, so a product regression
cannot masquerade as a capability gap.

Auth is the same gateway-JWT rig analytics uses (lib/gateway_jwt.py): the .NET
service verifies the ES256 JWT against the rig's JWKS — fetched from the rig's
plain-HTTP twin (explicit `auth_gateway_jwks_url`, RequireHttpsMetadata=false);
the Rust host's oidc-authn-plugin uses the rig's TLS discovery front + CA.
Claims contract: `sub` = the CALLER's person_id, `tenant_id` = the sole tenant
authority, `sub_type` user|service, `roles` scopes.
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

import yaml

from lib import api_coverage
from lib.analytics import ApiSpawnError, find_free_port
from lib.config import TEST_TENANT_ID, SessionConfig
from lib.gateway_jwt import AUDIENCE, GatewayAuth

LOG = logging.getLogger("e2e.identity")

# The identity service owns its own MariaDB database — separate from the
# analytics one the compose stack pre-creates.
IDENTITY_DATABASE = "identity_e2e"

IMPLEMENTATIONS = ("dotnet", "rust")


def implementation_from_env() -> str:
    """The EXPLICIT implementation selection (E2E_IDENTITY_IMPLEMENTATION).

    Fails fast on an unknown value, and on the removed E2E_IDENTITY_URL
    external mode: pointing the suite at an arbitrary URL while seeding the
    local throwaway database would silently test one deployment's HTTP
    surface against another's data.
    """
    if os.environ.get("E2E_IDENTITY_URL"):
        raise ApiSpawnError(
            "E2E_IDENTITY_URL is not supported: the harness seeds its own throwaway "
            "database, so an external target would answer from different data. Use "
            "E2E_IDENTITY_IMPLEMENTATION=dotnet|rust (the harness boots the service)."
        )
    impl = os.environ.get("E2E_IDENTITY_IMPLEMENTATION", "dotnet")
    if impl not in IMPLEMENTATIONS:
        raise ApiSpawnError(
            f"unknown E2E_IDENTITY_IMPLEMENTATION={impl!r} (expected one of {IMPLEMENTATIONS})"
        )
    return impl


def supports_deprecated_person_lookup(implementation: str) -> bool:
    """GET /v1/persons/{email} exists only in the .NET service — the Rust port
    dropped it (approved removal, zero callers). A capability of the EXPLICIT
    selection, never probed from runtime behavior."""
    return implementation == "dotnet"

_HEALTH_TIMEOUT_S = float(
    os.environ.get(
        "E2E_IDENTITY_HEALTH_TIMEOUT_S", "120"
    )  # RULE-DEFAULTS-OK: rig readiness ceiling, not a data-config input
)


def locate_app(cfg: SessionConfig) -> list[str]:
    """Spawn command for the .NET published app.

    Baked into the runner image at /opt/insight-identity
    (compose/Dockerfile.runner COPY --from=identity, plus the aspnetcore
    runtime). Host-mode fallback: a manual `dotnet publish` output under
    src/backend/services/identity/publish/.
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


def locate_rust_app(cfg: SessionConfig) -> list[str]:
    """Spawn command for the Rust `identity-resolution` binary.

    Baked into the runner image the same way the analytics binary is (a
    build-only compose service from the Rust service's own Dockerfile + a
    Dockerfile.runner COPY — both land with the cutover branch). Host-mode
    fallback: a manual `cargo build --release`.
    """
    candidates: list[Path] = []
    which = shutil.which("identity-resolution")
    if which:
        candidates.append(Path(which))
    candidates.append(Path("/usr/local/bin/identity-resolution"))  # runner image
    candidates.append(cfg.repo_root / "src/backend/target/release/identity-resolution")
    for c in candidates:
        if c.exists():
            LOG.info("using identity-resolution binary at %s", c)
            return [str(c)]
    raise ApiSpawnError(
        "identity-resolution binary not found — bake it into the runner image "
        "(docker-compose.runner.yml build-only service + Dockerfile.runner COPY, "
        "added with the Rust service's Dockerfile on the cutover branch) or "
        "`cargo build --release -p identity-resolution` for host mode."
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
    """A spawned, health-checked identity service bound to loopback."""

    def __init__(self, cfg: SessionConfig, port: int, implementation: str = "dotnet"):
        if implementation not in IMPLEMENTATIONS:
            raise ApiSpawnError(f"unknown implementation {implementation!r}")
        self.cfg = cfg
        self.port = port
        self.implementation = implementation
        self.base_url = f"http://127.0.0.1:{port}"
        self.auth = GatewayAuth()
        self._proc: subprocess.Popen[str] | None = None
        self._log_fh: Any = None
        self._log_path: Path | None = None
        self._rig_config_path: Path | None = None

    @property
    def supports_deprecated_person_lookup(self) -> bool:
        return supports_deprecated_person_lookup(self.implementation)

    def start(self) -> None:
        create_identity_database(self.cfg)
        if self.implementation == "rust":
            self._start_rust()
        else:
            self._start_dotnet()
        self._wait_healthy(timeout_s=_HEALTH_TIMEOUT_S)

    def _start_dotnet(self) -> None:
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
        self._spawn(cmd, env)

    # -- rust ---------------------------------------------------------------

    def _rust_env(self) -> dict[str, str]:
        """Leaf-config env overrides for the gears host (direct Popen execve
        preserves the hyphenated gear-name segments)."""
        env = os.environ.copy()
        env.update(
            {
                "APP__gears__api-gateway__config__bind_addr": f"127.0.0.1:{self.port}",
                "APP__gears__grpc-hub__config__listen_addr": f"uds:///tmp/identity-resolution-grpc-{self.port}.sock",
                "APP__gears__identity-resolution__config__database_url": identity_dsn(self.cfg),
                # The Rust service reads ClickHouse over HTTP (insight-clickhouse
                # client), not the native port the .NET service uses.
                "APP__gears__identity-resolution__config__clickhouse_url": self.cfg.ch_http_url,
                "APP__gears__identity-resolution__config__clickhouse_database": self.cfg.ch_database,
                "APP__gears__identity-resolution__config__clickhouse_user": self.cfg.ch_user,
                "APP__gears__identity-resolution__config__clickhouse_password": self.cfg.ch_password,
                "RUST_LOG": env.get("RUST_LOG", "info"),
            }
        )
        return env

    def _write_rust_rig_config(self) -> Path:
        """Per-spawn gears host config with the oidc-authn-plugin wired to the
        rig's TLS discovery front — the same shape AnalyticsProcess writes."""
        cfg = {
            "server": {"home_dir": "/tmp"},
            "logging": {"default": {"console_level": "info"}},
            "gears": {
                "api-gateway": {
                    "config": {
                        "bind_addr": f"127.0.0.1:{self.port}",
                        "enable_docs": False,
                        "cors_enabled": False,
                        "auth_disabled": False,
                    }
                },
                "gear-orchestrator": {"config": {}},
                "grpc-hub": {"config": {"listen_addr": f"uds:///tmp/identity-resolution-grpc-{self.port}.sock"}},
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
                        "http_client": {
                            "request_timeout": "5s",
                            "custom_ca_certificate_paths": [self.auth.ca_path],
                        },
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
                "identity-resolution": {
                    "config": {
                        "database_url": "",
                        "org_chart_source_type": "bamboohr",
                        "expand_subordinates": True,
                        "max_depth": 16,
                        "clickhouse_url": "",
                        "clickhouse_database": "identity",
                        "clickhouse_user": "",
                        "clickhouse_password": "",
                    }
                },
            },
        }
        fh = tempfile.NamedTemporaryFile(  # noqa: SIM115 — path used by the spawned binary
            mode="w", suffix=".yaml", prefix=f"identity-resolution-cfg-{self.port}-", delete=False
        )
        yaml.safe_dump(cfg, fh, sort_keys=False)
        fh.close()
        self._rig_config_path = Path(fh.name)
        return self._rig_config_path

    def _start_rust(self) -> None:
        cmd = locate_rust_app(self.cfg)
        config_path = self._write_rust_rig_config()
        env = self._rust_env()
        # The Rust service does NOT migrate at server start — run its migrate
        # subcommand first (schema + first-admin bootstrap), synchronously.
        # NB: this CLI (`-c <cfg> migrate`; bare = server) exists on the
        # cutover branch (feat/identity-resolution-cutover-prep, commit
        # b9984cd5) — the main-branch binary has no subcommands yet, so rust
        # mode becomes runnable once that branch lands (together with the
        # runner-image bake of the binary).
        migrate = subprocess.run(  # noqa: S603 — harness-controlled argv
            [*cmd, "-c", str(config_path), "migrate"],
            env=env,
            capture_output=True,
            text=True,
            timeout=300,
            check=False,
        )
        if migrate.returncode != 0:
            raise ApiSpawnError(
                f"identity-resolution migrate failed (rc={migrate.returncode}):\n"
                f"{migrate.stdout[-2000:]}\n{migrate.stderr[-2000:]}"
            )
        self._spawn([*cmd, "-c", str(config_path)], env)

    # -- shared spawn ---------------------------------------------------------

    def _spawn(self, cmd: list[str], env: dict[str, str]) -> None:
        self._log_fh = tempfile.NamedTemporaryFile(  # noqa: SIM115 — handle lives until stop()
            mode="w", suffix=".log", prefix=f"identity-{self.port}-", delete=False
        )
        self._log_path = Path(self._log_fh.name)
        LOG.info(
            "spawning identity (%s) on 127.0.0.1:%d (startup log: %s)",
            self.implementation,
            self.port,
            self._log_path,
        )
        self._proc = subprocess.Popen(
            cmd,
            env=env,
            stdout=self._log_fh,
            stderr=subprocess.STDOUT,
            text=True,
        )

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
        if self._rig_config_path is not None:
            try:
                self._rig_config_path.unlink(missing_ok=True)
            except OSError:
                pass
            self._rig_config_path = None

    def is_running(self) -> bool:
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
    """Context manager: provision DB, migrate (rust), spawn, yield, stop.

    The implementation comes from E2E_IDENTITY_IMPLEMENTATION (explicit,
    default dotnet) — see the module docstring.
    """
    proc = IdentityProcess(cfg, find_free_port(), implementation=implementation_from_env())
    proc.start()
    try:
        yield proc
    finally:
        proc.stop()
