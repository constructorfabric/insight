"""Connector *enrich* step: run a connector's pre-built enrich binary.

Some connectors materialize part of their silver via a compiled "enrich" binary
that reads the connector's staging tables and writes back into `staging.*`, which
dbt then unions into `silver.*` via `union_by_tag`. This is a first-class step in
the connector pipeline, NOT a per-connector special case: any connector whose
`descriptor.yaml` declares an `images.enrich` block participates.

Build vs run split (deliberate):
  * The BINARY IS BUILT FROM THE CONNECTOR'S OWN `Dockerfile` (the same one that
    ships the prod image — no duplicated build recipe) and BAKED INTO THE RUNNER
    IMAGE: docker-compose.runner.yml declares a build-only service per enrich
    connector and wires it as a named build context, and Dockerfile.runner does
    `COPY --from=<name> … /usr/local/bin/`. So the rig compiles nothing itself and
    there is no docker-in-docker — the binary is simply on PATH inside the runner.
  * This module only DISCOVERS the steps (from descriptors) and RUNS the on-PATH
    binary, between the staging and silver dbt builds — mirrors prod
    `dbt(tag:<c>) -> <c>-enrich -> dbt(silver)`. Per-test it runs ONLY the enrich
    of the connector whose bronze the test seeded (a jira test must not run
    youtrack-enrich), scoped to the seeded `source_id`(s).
"""

from __future__ import annotations

import logging
import os
import shutil
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path

import yaml

from e2e_lib import clickhouse as ch
from e2e_lib.config import SessionConfig

LOG = logging.getLogger("e2e.enrich")

_CONNECTORS_GLOB = "src/ingestion/connectors/**/descriptor.yaml"


class EnrichError(RuntimeError):
    """A connector enrich step failed to run, or its binary is not on PATH."""


@dataclass(frozen=True)
class EnrichStep:
    """A connector's declared enrich step (from descriptor.yaml.images.enrich)."""

    name: str  # connector name, e.g. "jira" — also its dbt tag
    namespace: str  # bronze schema, e.g. "bronze_jira"
    binary: str  # binary name on PATH in the runner, e.g. "jira-enrich"


def discover_enrich_steps(repo_root: Path) -> list[EnrichStep]:
    """Every connector descriptor that declares `images.enrich`.

    The binary name is the enrich crate's `[package].name` (the connector's
    Dockerfile installs it under that name on PATH) — read from the Cargo.toml in
    the `images.enrich.context` directory.
    """
    steps: list[EnrichStep] = []
    for desc_path in sorted(repo_root.glob(_CONNECTORS_GLOB)):
        try:
            doc = yaml.safe_load(desc_path.read_text(encoding="utf-8")) or {}
        except yaml.YAMLError as e:
            LOG.warning("skipping unreadable descriptor %s: %s", desc_path, e)
            continue
        enrich = (doc.get("images") or {}).get("enrich")
        if not enrich:
            continue
        namespace = (doc.get("connection") or {}).get("namespace")
        if not namespace:
            LOG.warning("descriptor %s has images.enrich but no connection.namespace; skipping", desc_path)
            continue
        context = (desc_path.parent / enrich.get("context", "./enrich")).resolve()
        cargo_toml = context / "Cargo.toml"
        if not cargo_toml.is_file():
            LOG.warning("enrich crate Cargo.toml not found at %s (descriptor %s); skipping", cargo_toml, desc_path)
            continue
        try:
            binary = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))["package"]["name"]
        except (KeyError, ValueError) as e:
            LOG.warning("cannot read [package].name from %s: %s; skipping", cargo_toml, e)
            continue
        steps.append(
            EnrichStep(
                name=doc.get("name") or namespace.removeprefix("bronze_"),
                namespace=namespace,
                binary=binary,
            )
        )
    return steps


class EnrichRunner:
    """Session-scoped: discover enrich steps once; run their on-PATH binaries."""

    def __init__(self, cfg: SessionConfig):
        """Discover enrich steps from connector descriptors once per session."""
        self.cfg = cfg
        self.steps = discover_enrich_steps(cfg.repo_root)

    def steps_for(self, schemas: set[str]) -> list[EnrichStep]:
        """Enrich steps whose bronze namespace is among the seeded schemas."""
        return [s for s in self.steps if s.namespace in schemas]

    def discover_source_ids(self, step: EnrichStep, tables: set[tuple[str, str]]) -> list[str]:
        """Distinct non-empty `source_id`s across the seeded tables in the step's namespace.

        enrich is scoped per connector instance (`--insight-source-id`); the rig
        derives the instances to enrich from the data the test actually seeded.
        """
        found: set[str] = set()
        for schema, table in sorted(tables):
            if schema != step.namespace:
                continue
            cols = ch.query(
                self.cfg,
                f"SELECT name FROM system.columns WHERE database = '{schema}' AND table = '{table}' AND name = 'source_id'",
            )
            if not cols:
                continue
            rows = ch.query(
                self.cfg,
                f"SELECT DISTINCT source_id FROM `{schema}`.`{table}` WHERE source_id IS NOT NULL AND source_id != ''",
            )
            found.update(str(r[0]) for r in rows)
        return sorted(found)

    def run(self, step: EnrichStep, source_ids: list[str], *, timeout_s: float = 180.0) -> None:
        """Run the baked-in enrich binary (on PATH) once per connector instance."""
        if not source_ids:
            LOG.warning(
                "enrich %s: no source_id found in seeded %s tables; skipping (nothing to enrich)",
                step.name,
                step.namespace,
            )
            return
        if shutil.which(step.binary) is None:
            raise EnrichError(
                f"{step.name} enrich binary {step.binary!r} not found on PATH — it should be baked "
                f"into the runner image (docker-compose.runner.yml `{step.name}-enrich` service + "
                f"Dockerfile.runner COPY --from); rebuild with `./e2e.sh build`."
            )
        for sid in source_ids:
            env = os.environ.copy()
            env.update(
                {
                    "CLICKHOUSE_HOST": self.cfg.ch_host,
                    "CLICKHOUSE_PORT": str(self.cfg.ch_http_port),
                    "CLICKHOUSE_USER": self.cfg.ch_user,
                    "CLICKHOUSE_PASSWORD": self.cfg.ch_password,
                    "INSIGHT_SOURCE_ID": sid,
                    "RUST_LOG": env.get("RUST_LOG", "info"),
                }
            )
            LOG.info("running %s enrich for source_id=%s", step.name, sid)
            result = subprocess.run(
                [
                    step.binary,
                    "--insight-source-id", sid,
                    "--clickhouse-host", self.cfg.ch_host,
                    "--clickhouse-port", str(self.cfg.ch_http_port),
                    "--clickhouse-user", self.cfg.ch_user,
                ],
                env=env,
                capture_output=True,
                text=True,
                check=False,
                timeout=timeout_s,
            )
            if result.returncode != 0:
                raise EnrichError(
                    f"{step.name} enrich failed for source_id={sid} (exit={result.returncode}):\n"
                    f"stdout tail:\n{result.stdout[-1500:]}\nstderr tail:\n{result.stderr[-1500:]}"
                )
