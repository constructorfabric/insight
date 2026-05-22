"""Per-xdist-worker context.

Resolves the pytest-xdist worker id once per worker; provides a schema suffix
that ch-seeder and dbt-runner use to namespace tables.
"""

from __future__ import annotations

import os
from dataclasses import dataclass


@dataclass(frozen=True)
class WorkerContext:
    """One per pytest worker.

    For serial runs (no xdist), `worker_id == "master"` and `schema_suffix == ""`.
    For xdist workers, `worker_id == "gw{N}"` and `schema_suffix == "_w{N}"`.
    """

    worker_id: str
    schema_suffix: str

    @classmethod
    def from_env(cls) -> "WorkerContext":
        wid = os.environ.get("PYTEST_XDIST_WORKER", "master")
        if wid == "master":
            return cls(worker_id="master", schema_suffix="")
        # xdist worker ids look like "gw0", "gw1", ... — strip the "gw" for a clean suffix
        n = wid.removeprefix("gw")
        return cls(worker_id=wid, schema_suffix=f"_w{n}")

    def schema(self, base: str) -> str:
        """Apply the worker suffix to a schema name. `bronze_jira` -> `bronze_jira_w0`."""
        return f"{base}{self.schema_suffix}"
