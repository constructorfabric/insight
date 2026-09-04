"""Where a test-stand instance's databases are.

The data-path suite talks to two of an instance's containers directly — it seeds
ClickHouse and reads identity state from MariaDB — while every product request goes
through the gateway. Both addresses come from the instance's own env file, the same
one `dev-compose.sh` wrote and `insight_stand` already resolves the gateway from, so a
run cannot be aimed at one instance's API and another's warehouse.
"""

from __future__ import annotations

import os
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

from insight_stand.errors import StandConnectionError
from insight_stand.stand import PUBLISHED_HOST, candidate_env_files, parse_env_file

_REPO_ROOT = Path(__file__).resolve().parents[3]

CLICKHOUSE_PORT_KEY = "CLICKHOUSE_HTTP_PORT"
MARIADB_PORT_KEY = "MARIADB_PORT"


@dataclass(frozen=True)
class InstanceConfig:
    """One instance's databases, and the file that said where they are.

    `source` is carried for the same reason `StandEndpoint` carries it: every
    instance uses the same user and database name and differs only by published
    port, so a run that reaches the wrong warehouse must be able to say which file
    it believed.
    """

    ch_host: str
    ch_http_port: int
    ch_user: str
    ch_password: str
    ch_database: str
    mariadb_host: str
    mariadb_port: int
    mariadb_user: str
    mariadb_password: str
    mariadb_database: str
    source: str

    def __str__(self) -> str:
        return f"clickhouse {self.ch_host}:{self.ch_http_port}, mariadb {self.mariadb_host}:{self.mariadb_port} (from {self.source})"


def _port(values: Mapping[str, str], key: str, source: Path) -> int:
    raw = (values.get(key) or "").strip()
    if not raw:
        raise StandConnectionError(
            f"{source}: {key} is missing — the instance publishes no port to reach"
        )
    try:
        return int(raw)
    except ValueError as exc:
        raise StandConnectionError(f"{source}: {key}={raw!r} is not a port") from exc


def _required(values: Mapping[str, str], key: str, source: Path) -> str:
    value = (values.get(key) or "").strip()
    if not value:
        raise StandConnectionError(f"{source}: {key} is missing")
    return value


def resolve_instance(
    *,
    repo_root: Path | None = None,
    environ: Mapping[str, str] | None = None,
) -> InstanceConfig:
    """Read an instance's database addresses, or raise `StandConnectionError`.

    Refuses an instance whose databases are external: those publish no port for a
    suite to seed, and seeding a shared warehouse is not something this suite may do.
    """
    env = os.environ if environ is None else environ
    root = _REPO_ROOT if repo_root is None else repo_root
    candidates = candidate_env_files(root, env)
    for path in candidates:
        if not path.is_file():
            continue
        values = parse_env_file(path)
        for key in ("CLICKHOUSE_EXTERNAL", "MARIADB_EXTERNAL"):
            if (values.get(key) or "").strip().lower() == "true":
                raise StandConnectionError(
                    f"{path}: {key}=true — this instance's database is not its own, and this suite writes to it"
                )
        return InstanceConfig(
            ch_host=PUBLISHED_HOST,
            ch_http_port=_port(values, CLICKHOUSE_PORT_KEY, path),
            ch_user=_required(values, "CLICKHOUSE_USER", path),
            ch_password=_required(values, "CLICKHOUSE_PASSWORD", path),
            ch_database=values.get("CLICKHOUSE_DATABASE", "insight").strip() or "insight",
            mariadb_host=PUBLISHED_HOST,
            mariadb_port=_port(values, MARIADB_PORT_KEY, path),
            mariadb_user=_required(values, "MARIADB_USER", path),
            mariadb_password=_required(values, "MARIADB_PASSWORD", path),
            mariadb_database=values.get("MARIADB_DATABASE", "analytics").strip() or "analytics",
            source=str(path),
        )
    tried = ", ".join(str(path) for path in candidates)
    raise StandConnectionError(f"cannot find an instance env file — tried {tried}")
