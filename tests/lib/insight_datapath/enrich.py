"""A connector's enrich step, run as the image the descriptor pins.

Some connectors materialize part of their silver through a compiled binary that reads
the connector's staging tables and writes back into `staging.*`, which dbt then unions
into `silver.*`. The pipeline is `dbt(tag:<connector>) -> enrich -> dbt(silver)`, and a
spec that seeds such a connector runs the step between its two builds — skipping it
yields silver built on output that was never written, which is a green build with wrong
numbers.

The image is the one the connector's descriptor pins, which is also the reference the
chart hands the deployed job, so what runs here is the shipped artefact. It joins the
instance's own compose project, so it reaches ClickHouse by service name.
"""

from __future__ import annotations

import logging
import os
import subprocess
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

import yaml

from insight_datapath import clickhouse as ch
from insight_datapath.instance import InstanceConfig

LOG = logging.getLogger("datapath.enrich")

_CONNECTORS_GLOB = "src/ingestion/connectors/**/descriptor.yaml"

_OVERLAY = "deploy/compose/docker-compose.enrich.yml"

#: What the binary is given, matching the deployed job's arguments. The output
#: database is fixed in the binary, so there is nothing to point elsewhere.
_INTERNAL_CLICKHOUSE_HOST = "clickhouse"
_INTERNAL_CLICKHOUSE_PORT = "8123"


class EnrichError(RuntimeError):
    """A connector's enrich step failed, or its image could not be run."""


@dataclass(frozen=True)
class EnrichStep:
    """A connector's declared enrich step."""

    name: str
    namespace: str
    image: str


def discover_enrich_steps(repo_root: Path) -> list[EnrichStep]:
    """Every connector descriptor declaring `images.enrich`, with the image it pins."""
    steps: list[EnrichStep] = []
    for descriptor_path in sorted(repo_root.glob(_CONNECTORS_GLOB)):
        try:
            document = yaml.safe_load(descriptor_path.read_text(encoding="utf-8")) or {}
        except yaml.YAMLError as error:
            LOG.warning("skipping unreadable descriptor %s: %s", descriptor_path, error)
            continue
        enrich = (document.get("images") or {}).get("enrich")
        if not enrich:
            continue
        namespace = (document.get("connection") or {}).get("namespace")
        image = enrich.get("image")
        if not namespace or not image:
            LOG.warning(
                "%s declares images.enrich without a namespace or image; skipping", descriptor_path
            )
            continue
        steps.append(
            EnrichStep(
                name=document.get("name") or namespace.removeprefix("bronze_"),
                namespace=namespace,
                image=image,
            )
        )
    return steps


class EnrichRunner:
    """Discovers the declared steps once, and runs one against the instance."""

    def __init__(
        self, cfg: InstanceConfig, *, repo_root: Path, project: str, env_file: Path
    ) -> None:
        self.cfg = cfg
        self.repo_root = repo_root
        self.project = project
        self.env_file = env_file
        self.steps = discover_enrich_steps(repo_root)

    def steps_for(self, schemas: set[str]) -> list[EnrichStep]:
        """The steps whose bronze namespace the spec seeded."""
        return [step for step in self.steps if step.namespace in schemas]

    def discover_source_ids(self, step: EnrichStep, tables: set[tuple[str, str]]) -> list[str]:
        """The connector instances to enrich, taken from what the spec actually seeded."""
        found: set[str] = set()
        for schema, table in sorted(tables):
            if schema != step.namespace:
                continue
            has_column = ch.query(
                self.cfg,
                f"SELECT name FROM system.columns WHERE database = '{schema}' "
                f"AND table = '{table}' AND name = 'source_id'",
            )
            if not has_column:
                continue
            rows = ch.query(
                self.cfg,
                f"SELECT DISTINCT source_id FROM `{schema}`.`{table}` "
                "WHERE source_id IS NOT NULL AND source_id != ''",
            )
            found.update(str(row[0]) for row in rows)
        return sorted(found)

    def run(self, step: EnrichStep, source_ids: Sequence[str], *, timeout_s: float = 600.0) -> None:
        """Run the step once per connector instance the spec seeded."""
        if not source_ids:
            LOG.warning(
                "enrich %s: the spec seeded no source id, so there is nothing to enrich", step.name
            )
            return
        for source_id in source_ids:
            LOG.info("running %s enrich for source_id=%s", step.name, source_id)
            result = subprocess.run(
                [
                    "docker",
                    "compose",
                    "--project-name",
                    self.project,
                    "--env-file",
                    str(self.env_file),
                    "-f",
                    "docker-compose.yml",
                    "-f",
                    _OVERLAY,
                    "run",
                    "--rm",
                    "--no-deps",
                    "enrich",
                    f"--insight-source-id={source_id}",
                    f"--clickhouse-host={_INTERNAL_CLICKHOUSE_HOST}",
                    f"--clickhouse-port={_INTERNAL_CLICKHOUSE_PORT}",
                    f"--clickhouse-user={self.cfg.ch_user}",
                    "--batch-size=10000",
                ],
                cwd=self.repo_root,
                env={
                    **os.environ,
                    "ENRICH_IMAGE": step.image,
                    "COMPOSE_PROJECT_NAME": self.project,
                },
                capture_output=True,
                text=True,
                check=False,
                timeout=timeout_s,
            )
            if result.returncode != 0:
                raise EnrichError(
                    f"{step.name} enrich failed for source_id={source_id} (exit={result.returncode})\n"
                    f"image: {step.image}\n"
                    f"stdout tail:\n{result.stdout[-1500:]}\nstderr tail:\n{result.stderr[-1500:]}"
                )
