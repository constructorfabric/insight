"""Materialize dbt models for one test and register what they wrote.

Every suite that seeds bronze and then builds the models above it needs the same
pairing: record the relations the build is about to write into the per-test
truncate ledger, then invoke dbt. Skipping the recording half leaves a test's
staging and silver rows in place for the next test, and the incremental models
read them instead of the seed — so the pairing lives here rather than at each
call site.
"""

from __future__ import annotations

from collections.abc import Sequence

from lib.ch_seeder import CHSeeder
from lib.dbt_runner import DbtRunner
from lib.worker import WorkerContext


class TrackedModels:
    """dbt builds whose output the next test truncates.

    INVARIANT: every relation a test materializes is in the truncate ledger by
    the time dbt touches it. Recording precedes the invocation so a build that
    raises partway still leaves its targets registered for the next test to
    clean; registering a relation dbt never got to is harmless, since the
    ledger truncates `IF EXISTS`.
    """

    def __init__(self, dbt_runner: DbtRunner, seeder: CHSeeder) -> None:
        self._dbt = dbt_runner
        self._seeder = seeder

    def build(self, models: Sequence[str], *, worker_ctx: WorkerContext, with_ancestors: bool = False) -> None:
        """`dbt build` the named models. `with_ancestors` prefixes each with `+`,
        which pulls the connector's `<connector>__bronze_promoted` view."""
        if not models:
            return
        self._record(models)
        prefix = "+" if with_ancestors else ""
        self._dbt.build(" ".join(f"{prefix}{model}" for model in sorted(models)), worker_ctx=worker_ctx)

    def run(self, models: Sequence[str], *, worker_ctx: WorkerContext, full_refresh: bool = False) -> None:
        """`dbt run` the named models — for models whose own dbt tests a
        single fixture's partial seed cannot satisfy."""
        if not models:
            return
        self._record(models)
        self._dbt.run(" ".join(sorted(models)), worker_ctx=worker_ctx, full_refresh=full_refresh)

    def _record(self, models: Sequence[str]) -> None:
        for schema, table in self._dbt.materialized_relations(models):
            self._seeder.ledger.record(schema, table)
