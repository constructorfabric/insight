"""Bronze through a connector's own models into the shared identity inputs.

The identity lane's question is what a connector tells identity about an account,
so its tests seed a connector's bronze and read `identity.identity_inputs` after the
connector's dbt models and the shared union ran over it. This is the front half of
what `spec_runner.run_spec` does for a metric spec, without the silver classes, the
enrich step or the gold build a metric needs and an identity input does not.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.fixture_loader import prepare_rows
from insight_datapath.reset import clear
from insight_datapath.tracked_models import TrackedModels

IDENTITY_INPUTS = "identity_inputs"


class ConnectorPathError(RuntimeError):
    """The seeded bronze reaches no identity input, so there is nothing to assert."""


class ConnectorPath:
    """Seeds code-built bronze rows and builds every model between them and identity."""

    def __init__(self, ch_seeder: CHSeeder, dbt_runner: DbtRunner, *, schemas_dir: Path) -> None:
        self._seeder = ch_seeder
        self._dbt = dbt_runner
        self._schemas_dir = schemas_dir
        self._tracked = TrackedModels(dbt_runner, ch_seeder)

    def build(self, bronze: dict[str, list[dict[str, Any]]]) -> None:
        """Clear the last test's relations, seed `bronze`, rebuild the identity inputs."""
        clear(self._seeder.cfg, self._seeder.ledger.drain())

        rows: dict[str, list[dict[str, Any]]] = {}
        schemas: dict[str, dict[str, Any]] = {}
        for table, records in bronze.items():
            rows[table], schemas[table] = prepare_rows(self._schemas_dir, table, records)
        self._seeder.seed_bronze(rows, schemas)

        touched = {tuple(table.split(".", 1)) for table in bronze}
        staging, silver = self._dbt.derive_selectors({(s, t) for s, t in touched})
        self._tracked.build(staging, with_ancestors=True)
        if IDENTITY_INPUTS not in silver:
            raise ConnectorPathError(
                f"none of {sorted(bronze)} feeds {IDENTITY_INPUTS}; the models derived were "
                f"{staging} and {silver}"
            )
        self._tracked.run([IDENTITY_INPUTS], full_refresh=True)
