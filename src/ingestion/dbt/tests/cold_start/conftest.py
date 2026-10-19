"""Rig for the cold-start lane: what a dbt run can build when bronze is absent.

No rows, no seeding, no connector. The only question here is which models a run
can build against the warehouse as it stands, so a test starts from the
databases a deploy creates and nothing else — the state a fresh install is in
before its first connector has ever synced.

The warehouse is reset between tests, so connection details come from the
environment with no defaults: a suite that silently fell back to localhost would
empty somebody else's warehouse. See README.md for the two commands that set it
up.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path

import clickhouse_connect
import pytest

# tests/cold_start -> tests -> dbt -> ingestion
DBT_DIR = Path(__file__).resolve().parents[2]
INGESTION_DIR = DBT_DIR.parent
CREATE_DATABASES = INGESTION_DIR / "scripts/create-databases.sh"
RUN_DBT = INGESTION_DIR / "scripts/bootstrap-db/run-dbt.sh"
PROFILE_WRITER = INGESTION_DIR / "scripts/dbt_profiles.py"

#: ClickHouse ships these; everything else in the warehouse belongs to a test.
SYSTEM_DATABASES = frozenset({"system", "default", "INFORMATION_SCHEMA", "information_schema"})

#: One dbt parse plus whatever the run does; a cold run builds almost nothing.
TIMEOUT_S = 600.0


def _env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"{name} is not set. This suite needs a ClickHouse it may empty; see README.md.")
    return value


@dataclass(frozen=True)
class Warehouse:
    """The ClickHouse under test and the entry points that target it."""

    host: str
    port: int
    user: str
    password: str
    database: str
    profiles_dir: Path

    @property
    def env(self) -> dict[str, str]:
        """What every entry point reads its connection from."""
        return {
            **os.environ,
            "CLICKHOUSE_URL": f"http://{self.host}:{self.port}",
            "CLICKHOUSE_HOST": self.host,
            "CLICKHOUSE_PORT": str(self.port),
            "CLICKHOUSE_PROTOCOL": "http",
            "CLICKHOUSE_USER": self.user,
            "CLICKHOUSE_PASSWORD": self.password,
            "CLICKHOUSE_DATABASE": self.database,
        }

    def client(self, database: str = "default"):
        return clickhouse_connect.get_client(
            host=self.host, port=self.port, username=self.user, password=self.password, database=database
        )

    def execute(self, sql: str, parameters: dict[str, str] | None = None) -> None:
        """Run a statement. Identifiers ride `parameters` as `{name:Identifier}`,
        never the string — ClickHouse quotes them server-side."""
        with self.client() as client:
            client.command(sql, parameters=parameters)

    def databases(self) -> set[str]:
        with self.client() as client:
            return {row[0] for row in client.query("SELECT name FROM system.databases").result_rows} - SYSTEM_DATABASES

    def relations(self) -> set[str]:
        """Every table and view a run could have left, as `database.name`."""
        with self.client() as client:
            rows = client.query(
                "SELECT database, name FROM system.tables WHERE database NOT IN %(system)s",
                parameters={"system": list(SYSTEM_DATABASES)},
            ).result_rows
        return {f"{database}.{name}" for database, name in rows}

    def rows_in(self, database: str, name: str) -> int:
        with self.client() as client:
            counted = client.query(
                "SELECT count() FROM {database:Identifier}.{name:Identifier}",
                parameters={"database": database, "name": name},
            )
        return int(counted.result_rows[0][0])

    def run_dbt_sh(self, *args: str) -> subprocess.CompletedProcess[str]:
        """The deploy's own entry point, judging nothing about how it went."""
        return subprocess.run(
            [str(RUN_DBT), *args], env=self.env, capture_output=True, text=True, check=False, timeout=TIMEOUT_S
        )

    def source_tables(self, schema: str) -> set[str]:
        """The tables dbt expects in `schema`, read from the project's own sources."""
        listed = self._run(
            str(Path(sys.executable).parent / "dbt"),
            "--quiet",
            "ls",
            "--resource-type",
            "source",
            "--output",
            "json",
            "--output-keys",
            "schema",
            "identifier",
            "--profiles-dir",
            str(self.profiles_dir),
            cwd=DBT_DIR,
        )
        sources = (json.loads(line) for line in listed.splitlines() if line.startswith("{"))
        return {source["identifier"] for source in sources if source["schema"] == schema}

    def unbuildable_models(self) -> set[str]:
        """The names `list_unbuildable_models` reports for this warehouse."""
        listed = self._run(
            str(Path(sys.executable).parent / "dbt"),
            "--quiet",
            "run-operation",
            "list_unbuildable_models",
            "--profiles-dir",
            str(self.profiles_dir),
            cwd=DBT_DIR,
        )
        return set(listed.split())

    def _run(self, *argv: str, cwd: Path | None = None) -> str:
        completed = subprocess.run(
            list(argv), env=self.env, cwd=cwd, capture_output=True, text=True, check=False, timeout=TIMEOUT_S
        )
        if completed.returncode != 0:
            pytest.fail(
                f"{Path(argv[0]).name} failed ({completed.returncode}):\n"
                f"{completed.stdout[-4000:]}\n{completed.stderr[-4000:]}",
                pytrace=False,
            )
        return completed.stdout


@pytest.fixture(scope="session")
def warehouse(tmp_path_factory: pytest.TempPathFactory) -> Warehouse:
    profiles_dir = tmp_path_factory.mktemp("profiles")
    candidate = Warehouse(
        host=_env("CLICKHOUSE_HOST"),
        port=int(_env("CLICKHOUSE_HTTP_PORT")),
        user=_env("CLICKHOUSE_USER"),
        password=_env("CLICKHOUSE_PASSWORD"),
        database=_env("CLICKHOUSE_DATABASE"),
        profiles_dir=profiles_dir,
    )
    # The deploy's own profile writer, so the rig connects the way a deployment
    # does rather than inventing a second connection shape.
    candidate._run(
        sys.executable,
        str(PROFILE_WRITER),
        "--target",
        "bootstrap",
        "--correlated-subqueries",
        "--profiles-dir",
        str(profiles_dir),
    )
    return candidate


@pytest.fixture(scope="module")
def fresh_install(warehouse: Warehouse) -> Iterator[Warehouse]:
    """A warehouse holding exactly what a deploy creates before the first sync.

    Created through `create-databases.sh` rather than by hand: it is the one
    creation site a deployment has, so a test cannot disagree with it about
    which databases stand at this point.
    """
    for database in warehouse.databases():
        warehouse.execute("DROP DATABASE IF EXISTS {database:Identifier}", {"database": database})

    created = subprocess.run(
        ["bash", str(CREATE_DATABASES)],
        env=warehouse.env,
        capture_output=True,
        text=True,
        check=False,
        timeout=TIMEOUT_S,
    )
    if created.returncode != 0:
        pytest.fail(f"create-databases.sh failed:\n{created.stdout[-4000:]}\n{created.stderr[-4000:]}", pytrace=False)

    yield warehouse
