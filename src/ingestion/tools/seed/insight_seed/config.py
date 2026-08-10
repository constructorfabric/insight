"""Typed parsing of the seeder's environment contract.

Two variables have no safe default and are therefore required rather than
guessed:

* `TENANT_DEFAULT_ID` — rows are tenant-scoped, and rows written under a tenant
  the stand does not use are invisible to every login while the seed still
  reports success.
* `MARIADB_ANALYTICS_DB` — which database holds the analytics catalogue tables
  is a per-stand fact, not a convention.

Everything else keeps a default, because getting it wrong fails loudly on the
first connection attempt instead of producing a silently useless stand.
"""

from __future__ import annotations

import datetime as _dt
import uuid as uuid_mod
from collections.abc import Mapping
from dataclasses import dataclass, replace
from pathlib import Path

#: Prefix on the `reason` of every identity row this seeder writes. It is what
#: lets a preflight check tell demo rows from rows some other writer owns, so
#: `identity.py` composes every reason from it rather than spelling it out.
SEED_REASON_PREFIX = "seed.py "

TENANT_ENV = "TENANT_DEFAULT_ID"
ANALYTICS_DB_ENV = "MARIADB_ANALYTICS_DB"
IDENTITY_DB_ENV = "MARIADB_DB"
CROSS_TENANT_FIXTURE_ENV = "SEED_CROSS_TENANT_FIXTURE"
FORCE_ENV = "SEED_FORCE"
ANCHOR_ENV = "SEED_ANCHOR_DATE"
MANIFEST_PATH_ENV = "SEED_MANIFEST_PATH"
DAYS_ENV = "SEED_DAYS"
DEV_USER_EMAIL_ENV = "DEV_USER_EMAIL"

#: Length of the seeded activity window when nobody pins one.
DEFAULT_SEED_DAYS = 60

#: The clock is read once; see `parse_anchor_date`.
_DERIVED_ANCHOR: _dt.date | None = None

_TRUE = frozenset({"1", "true", "yes", "on"})
_FALSE = frozenset({"0", "false", "no", "off"})


class EnvContractError(Exception):
    """The environment cannot describe a stand to seed.

    Carries every problem found, not the first one: an operator wiring up a new
    stand should see the whole list in one run.
    """

    def __init__(self, problems: tuple[str, ...]) -> None:
        self.problems = problems
        super().__init__("\n".join(f"  - {p}" for p in problems))


@dataclass(frozen=True)
class MariaDb:
    host: str
    port: int
    user: str
    password: str
    database: str

    def with_database(self, database: str) -> MariaDb:
        return replace(self, database=database)


@dataclass(frozen=True)
class ClickHouse:
    host: str
    http_port: int
    user: str
    password: str
    database: str

    @property
    def url(self) -> str:
        return f"http://{self.host}:{self.http_port}"


def parse_tenant_id(env: Mapping[str, str]) -> str:
    """The tenant every seeded row is scoped to. Required, and a real UUID."""
    raw = (env.get(TENANT_ENV) or "").strip()
    if not raw:
        raise EnvContractError(
            (
                f"{TENANT_ENV} is not set. Every seeded row is scoped to a tenant, and "
                "rows written under the wrong one are invisible to every login on the "
                "stand. Set it to the tenant the stand authenticates against "
                "(the chart's global.tenantDefaultId).",
            )
        )
    try:
        uuid_mod.UUID(raw)
    except ValueError as exc:
        raise EnvContractError((f"{TENANT_ENV}={raw!r} is not a UUID: {exc}.",)) from exc
    return raw


def parse_analytics_database(env: Mapping[str, str]) -> str:
    """The database holding the analytics catalogue tables. Required.

    The compose stack keeps them in a database of their own; a chart-deployed
    stand keeps them in `mariadb.database` alongside the rest of the product.
    Neither is a default the other can live with.
    """
    raw = (env.get(ANALYTICS_DB_ENV) or "").strip()
    if not raw:
        raise EnvContractError(
            (
                f"{ANALYTICS_DB_ENV} is not set. It names the database holding the "
                "analytics catalogue tables (metric_definitions, metric_catalog), which "
                "differs per stand: the compose stack uses a separate `analytics` "
                "database, a chart-deployed stand keeps them in `mariadb.database`.",
            )
        )
    return raw


def parse_identity_database(env: Mapping[str, str]) -> str:
    return (env.get(IDENTITY_DB_ENV) or "").strip() or "identity"


def parse_mariadb(env: Mapping[str, str], *, database: str) -> MariaDb:
    return MariaDb(
        host=env.get("MARIADB_HOST", "mariadb"),
        port=int(env.get("MARIADB_PORT", "3306")),
        user=env.get("MARIADB_USER", "insight"),
        password=env.get("MARIADB_PASSWORD", "insight-local"),
        database=database,
    )


def parse_clickhouse(env: Mapping[str, str]) -> ClickHouse:
    return ClickHouse(
        host=env.get("CLICKHOUSE_HOST", "clickhouse"),
        http_port=int(env.get("CLICKHOUSE_HTTP_PORT", "8123")),
        user=env.get("CLICKHOUSE_USER", "insight"),
        password=env.get("CLICKHOUSE_PASSWORD", "insight-local"),
        database=env.get("CLICKHOUSE_DATABASE", "insight"),
    )


def parse_flag(env: Mapping[str, str], name: str, *, default: bool) -> bool:
    raw = (env.get(name) or "").strip().lower()
    if not raw:
        return default
    if raw in _TRUE:
        return True
    if raw in _FALSE:
        return False
    raise EnvContractError(
        (f"{name}={raw!r} is not a boolean; use one of {sorted(_TRUE | _FALSE)}.",)
    )


def cross_tenant_fixture_enabled(env: Mapping[str, str]) -> bool:
    """Whether to write the second tenant's cross-tenant refusal fixture.

    On by default, because the compose stand's test suite asserts against it. A
    cluster stand turns it off: the fixture trips identity-resolution's
    tenant-mismatch guard, which then aborts every scheduled projection run.
    """
    return parse_flag(env, CROSS_TENANT_FIXTURE_ENV, default=True)


def force_enabled(env: Mapping[str, str]) -> bool:
    """Whether to seed a tenant that already holds rows this seeder did not write."""
    return parse_flag(env, FORCE_ENV, default=False)


def parse_anchor_date(env: Mapping[str, str]) -> _dt.date:
    """Last day carrying seeded activity.

    Unset, or the literal `today`, means yesterday UTC — the default tracks the
    calendar because a fixed past anchor ages one day per day and the surfaces
    this data populates are read through a UI whose period is relative to now.
    Determinism means "a given anchor always yields the same bytes", and the
    anchor a run used is recorded in its manifest.

    Lives here, next to the rest of the environment contract, because both the
    row generators and the manifest builder need the same answer: two readers of
    the same variable computing `now()` independently disagree across a UTC
    midnight, and the manifest would then report a window one day off the rows.
    """
    raw = (env.get(ANCHOR_ENV) or "").strip()
    if raw and raw.lower() != "today":
        try:
            return _dt.date.fromisoformat(raw)
        except ValueError as exc:
            raise EnvContractError(
                (f"{ANCHOR_ENV}={raw!r} is not an ISO date (YYYY-MM-DD) or `today`: {exc}.",)
            ) from exc

    # Resolved from the clock ONCE per process. Sharing the reader was not
    # enough: two callers each computing `now()` still disagree if the run
    # straddles a UTC midnight, and the manifest would then report a window the
    # rows do not sit in.
    global _DERIVED_ANCHOR
    if _DERIVED_ANCHOR is None:
        _DERIVED_ANCHOR = _dt.datetime.now(_dt.UTC).date() - _dt.timedelta(days=1)
    return _DERIVED_ANCHOR


def parse_seed_days(env: Mapping[str, str], default: int = DEFAULT_SEED_DAYS) -> int:
    """Length of the seeded activity window, in days. Empty means unset."""
    raw = (env.get(DAYS_ENV) or "").strip()
    if not raw:
        return default
    try:
        days = int(raw)
    except ValueError as exc:
        raise EnvContractError((f"{DAYS_ENV}={raw!r} is not a whole number of days.",)) from exc
    if days < 1:
        raise EnvContractError((f"{DAYS_ENV}={days} must be at least 1.",))
    return days


def parse_manifest_path(env: Mapping[str, str]) -> Path:
    """Where a run writes its manifest.

    The working directory by default, NOT a path derived from this module's
    location: the package is installed (into the toolbox image, into a venv), so
    its own directory is wherever pip put it — writing there means writing into
    site-packages. The compose service runs with the seeder's directory as its
    working directory, so the default lands exactly where the stand suite reads
    it, and a cluster Job points the variable at somewhere writable.
    """
    raw = (env.get(MANIFEST_PATH_ENV) or "").strip()
    return Path(raw) if raw else Path.cwd() / "manifest.json"


def parse_dev_user_email(env: Mapping[str, str]) -> str:
    """The persona the dev-lead login resolves to. Required by every roster build."""
    raw = (env.get(DEV_USER_EMAIL_ENV) or "").strip().lower()
    if not raw:
        raise EnvContractError(
            (
                f"{DEV_USER_EMAIL_ENV} is not set. It names the person who leads the demo dev "
                "team, and the roster is built around them — a stand seeded without it has no "
                "login that resolves to a person.",
            )
        )
    return raw
