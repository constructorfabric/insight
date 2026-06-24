"""Connector *enrich* step: build + run a connector's compiled enrich binary.

Some connectors materialize part of their silver via a compiled "enrich" binary
that reads the connector's staging tables and writes back into `staging.*`, which
dbt then unions into `silver.*` via `union_by_tag`. This is a first-class step in
the connector pipeline, NOT a per-connector special case: any connector whose
`descriptor.yaml` declares an `images.enrich` block participates.

Prod order (per descriptor):
    dbt(tag:<connector>)  ->  <connector>-enrich  ->  dbt(<descriptor.dbt_select>)

The e2e rig mirrors that between its staging and silver dbt builds (see
specs/test_fixtures.py). Discovery is data-driven from the descriptors, so when a
new connector (e.g. youtrack) gains an `images.enrich` + `dbt_select`, the rig
picks it up with no framework change.

The enrich binary contract (shared by the family — verified against jira-enrich):
  - built with `cargo build --release --features io` from `<descriptor>/<context>`
    (the `images.enrich.context`, conventionally `./enrich`); binary name = the
    crate's `[package].name`.
  - CLI/env: `--insight-source-id` (INSIGHT_SOURCE_ID), `--clickhouse-host`
    (CLICKHOUSE_HOST), `--clickhouse-port` (CLICKHOUSE_PORT, HTTP), `--clickhouse-user`
    (CLICKHOUSE_USER); password via CLICKHOUSE_PASSWORD env. Reads+writes `staging.*`.
"""

from __future__ import annotations

import logging
import os
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path

import yaml

from e2e_lib import clickhouse as ch
from e2e_lib.analytics_api import _resolve_cargo  # shared cargo locator
from e2e_lib.config import SessionConfig

LOG = logging.getLogger("e2e.enrich")

_CONNECTORS_GLOB = "src/ingestion/connectors/**/descriptor.yaml"


class EnrichError(RuntimeError):
    pass


@dataclass(frozen=True)
class EnrichStep:
    """A connector's declared enrich step (from descriptor.yaml.images.enrich)."""

    name: str  # connector name, e.g. "jira" — also its dbt tag
    namespace: str  # bronze schema, e.g. "bronze_jira"
    crate_dir: Path  # cargo crate dir (descriptor dir / images.enrich.context)
    binary_name: str  # crate [package].name, e.g. "jira-enrich"
    dbt_select: str  # descriptor.dbt_select — silver routing (e.g. "tag:silver,tag:jira+")

    @property
    def binary_path(self) -> Path:
        return self.crate_dir / "target" / "release" / self.binary_name


def discover_enrich_steps(repo_root: Path) -> list[EnrichStep]:
    """Every connector descriptor that declares `images.enrich`."""
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
        context = enrich.get("context", "./enrich")
        crate_dir = (desc_path.parent / context).resolve()
        cargo_toml = crate_dir / "Cargo.toml"
        if not cargo_toml.is_file():
            LOG.warning("enrich crate not found at %s (descriptor %s); skipping", cargo_toml, desc_path)
            continue
        try:
            binary_name = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))["package"]["name"]
        except (KeyError, ValueError) as e:
            LOG.warning("cannot read [package].name from %s: %s; skipping", cargo_toml, e)
            continue
        steps.append(
            EnrichStep(
                name=doc.get("name") or namespace.removeprefix("bronze_"),
                namespace=namespace,
                crate_dir=crate_dir,
                binary_name=binary_name,
                dbt_select=doc.get("dbt_select") or "",
            )
        )
    return steps


class EnrichRunner:
    """Session-scoped: discover enrich steps once, build each crate at most once."""

    def __init__(self, cfg: SessionConfig):
        self.cfg = cfg
        self.steps = discover_enrich_steps(cfg.repo_root)
        self._built: set[Path] = set()

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

    def ensure_built(self, step: EnrichStep, *, timeout_s: float = 600.0) -> None:
        if step.crate_dir in self._built:
            return
        cargo = _resolve_cargo()
        if cargo is None:
            raise EnrichError(
                "cargo not found on PATH or standard rustup locations — cannot build "
                f"enrich binary for connector {step.name!r}."
            )
        LOG.info("cargo build --release --features io (%s) at %s", step.binary_name, step.crate_dir)
        result = subprocess.run(
            [cargo, "build", "--release", "--features", "io", "--manifest-path", str(step.crate_dir / "Cargo.toml")],
            cwd=str(step.crate_dir),
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_s,
        )
        if result.returncode != 0:
            raise EnrichError(
                f"cargo build failed for {step.name} enrich (exit={result.returncode}):\n{result.stderr[-2000:]}"
            )
        if not step.binary_path.exists():
            raise EnrichError(f"enrich binary not at expected path: {step.binary_path}")
        self._built.add(step.crate_dir)

    def run(self, step: EnrichStep, source_ids: list[str], *, timeout_s: float = 180.0) -> None:
        """Build (once) then run the enrich binary for each connector instance."""
        if not source_ids:
            LOG.warning(
                "enrich %s: no source_id found in seeded %s tables; skipping (nothing to enrich)",
                step.name,
                step.namespace,
            )
            return
        self.ensure_built(step)
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
                    str(step.binary_path),
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
