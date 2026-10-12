"""A warm install's Bronze, and the upgrade that brings it onto the canonical tenant.

A warm install holds Bronze written under a legacy tenant key. In warm mode a spec's
Bronze is seeded under that key, the tenant transition script runs over it the way
an upgrade would, and the spec's assertions must then hold for the canonical tenant
unchanged — so every metric spec doubles as an upgrade test.
"""

from __future__ import annotations

import os
import subprocess
from collections.abc import MutableMapping
from dataclasses import dataclass
from pathlib import Path

from insight_datapath.instance import InstanceConfig
from insight_datapath.process import tail

TRANSITION_SCRIPT = Path("src/ingestion/scripts/migrate-tenant-identity.sh")
WARM_KEY_ENV = "INSIGHT_DATAPATH_WARM_TENANT_KEY"


class TransitionError(RuntimeError):
    pass


@dataclass(frozen=True)
class WarmTenant:
    legacy_key: str
    canonical: str


def warm_tenant(option: str | None, canonical: str) -> WarmTenant | None:
    """The warm-mode tenant pair, from `--warm-tenant-key` or its env variable."""
    key = option or os.environ.get(WARM_KEY_ENV) or ""
    if not key:
        return None
    if key == canonical:
        raise TransitionError(
            f"warm tenant key {key!r} equals the canonical tenant: there is nothing to upgrade"
        )
    return WarmTenant(legacy_key=key, canonical=canonical)


@dataclass(frozen=True)
class TenantEnv:
    """The variables dbt's `tenant_key()` reads; empty unless the run is warm."""

    values: dict[str, str]

    def apply(self, environ: MutableMapping[str, str]) -> None:
        for name, value in self.values.items():
            if value:
                environ[name] = value
            else:
                environ.pop(name, None)


def tenant_env(warm: WarmTenant | None) -> TenantEnv:
    if warm is None:
        return TenantEnv({"INSIGHT_TENANT_ID": "", "INSIGHT_TENANT_KEY": ""})
    return TenantEnv({"INSIGHT_TENANT_ID": warm.canonical, "INSIGHT_TENANT_KEY": warm.legacy_key})


def apply_transition(cfg: InstanceConfig, warm: WarmTenant, *, repo_root: Path) -> None:
    script = repo_root / TRANSITION_SCRIPT
    if not script.is_file():
        raise TransitionError(f"warm mode needs the tenant transition script at {script}")
    env = {
        **os.environ,
        "CLICKHOUSE_URL": f"http://{cfg.ch_host}:{cfg.ch_http_port}",
        "CLICKHOUSE_USER": cfg.ch_user,
        "CLICKHOUSE_PASSWORD": cfg.ch_password,
    }
    result = subprocess.run(
        [
            "bash", str(script),
            "--canonical-tenant", warm.canonical,
            "--legacy-tenant-key", warm.legacy_key,
        ],
        env=env, capture_output=True, text=True, check=False, timeout=600,
    )
    if result.returncode != 0:
        raise TransitionError(
            f"{script.name} exited {result.returncode}:\n{tail(result.stdout)}\n{tail(result.stderr)}"
        )
