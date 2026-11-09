from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from dataclasses import asdict, dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import cast

import yaml
from insight_logging import configure
from sweep.ledger import Ledger, LedgerError

type Json = None | bool | int | float | str | list[Json] | dict[str, Json]

TABLE = "ingestion_history.transform_versions"
PARSE_TIMEOUT_SECS = 180


@dataclass(frozen=True)
class Model:
    relation: str
    version: int
    incremental_only: bool


@dataclass(frozen=True)
class Manifest:
    models: dict[str, Model]
    children: dict[str, tuple[str, ...]]
    parents: dict[str, tuple[str, ...]]
    source_connectors: dict[str, str]


@dataclass(frozen=True)
class Plan:
    baseline: tuple[str, ...]
    changed: tuple[str, ...]
    new: tuple[str, ...]
    removed: tuple[str, ...]
    rebuild: tuple[str, ...]
    incremental_only: tuple[str, ...]
    connectors: tuple[str, ...]

    def fields(self) -> dict[str, object]:
        names = asdict(self)
        return {**names, "counts": {key: len(value) for key, value in names.items()}}


def object_value(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def string_value(value: Json) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError("expected a nonempty string")
    return value


def version_value(value: Json) -> int:
    if type(value) is not int or value <= 0:
        raise ValueError("transform_version must be a positive integer")
    return value


def stored_version_value(value: Json) -> int:
    return version_value(int(value) if isinstance(value, str) else value)


def graph_value(value: Json) -> dict[str, tuple[str, ...]]:
    result: dict[str, tuple[str, ...]] = {}
    for key, items in object_value(value).items():
        if not isinstance(items, list):
            raise TypeError("expected a dependency list")
        result[key] = tuple(string_value(item) for item in items)
    return result


def read_manifest(data: dict[str, Json], project: Path, connectors: Path) -> Manifest:
    models: dict[str, Model] = {}
    for name, raw in object_value(data["nodes"]).items():
        node = object_value(raw)
        if node["resource_type"] != "model":
            continue
        config = object_value(node["config"])
        version = version_value(object_value(config["meta"])["transform_version"])
        relation_parts = [string_value(node["schema"]), string_value(node["alias"])]
        if node["database"]:
            relation_parts.insert(0, string_value(node["database"]))
        relation = ".".join(relation_parts)
        models[name] = Model(relation, version, config["full_refresh"] is False)

    source_connectors: dict[str, str] = {}
    for name, raw in object_value(data["sources"]).items():
        source = object_value(raw)
        if not string_value(source["schema"]).startswith("bronze_"):
            continue
        declaration = (project / string_value(source["original_file_path"])).resolve()
        relative = declaration.relative_to(connectors.resolve())
        if len(relative.parts) < 3:
            raise ValueError(f"{name}: source declaration has no connector directory")
        descriptor_path = connectors.joinpath(*relative.parts[:2], "descriptor.yaml")
        descriptor = object_value(yaml.safe_load(descriptor_path.read_text()))
        source_connectors[name] = string_value(descriptor["name"])

    return Manifest(models, graph_value(data["child_map"]), graph_value(data["parent_map"]), source_connectors)


def closure(roots: set[str], graph: dict[str, tuple[str, ...]]) -> set[str]:
    visited: set[str] = set()
    pending = list(roots)
    while pending:
        name = pending.pop()
        if name in visited:
            continue
        visited.add(name)
        if name in graph:
            pending.extend(graph[name])
    return visited


def plan_versions(manifest: Manifest, applied: dict[str, int]) -> Plan:
    deployed = manifest.models
    if not applied:
        return Plan(tuple(sorted(deployed)), (), (), (), (), (), ())

    changed = {name for name in deployed.keys() & applied.keys() if deployed[name].version != applied[name]}
    new = deployed.keys() - applied.keys()
    removed = applied.keys() - deployed.keys()
    rebuild = closure(changed | new, manifest.children) & deployed.keys()
    incremental = {name for name in rebuild if deployed[name].incremental_only}
    ancestors = closure(rebuild, manifest.parents)
    connectors = {manifest.source_connectors[name] for name in ancestors & manifest.source_connectors.keys()}
    return Plan(
        (),
        tuple(sorted(changed)),
        tuple(sorted(new)),
        tuple(sorted(removed)),
        tuple(sorted(rebuild)),
        tuple(sorted(incremental)),
        tuple(sorted(connectors)),
    )


class AppliedVersions(Ledger):
    def latest(self) -> dict[str, int]:
        rows = self._select(
            f"SELECT model, argMax(transform_version, applied_at) AS transform_version FROM {TABLE} GROUP BY model"
        )
        return {string_value(row["model"]): stored_version_value(row["transform_version"]) for row in rows}

    def baseline(self, models: dict[str, Model], release: str | None) -> None:
        applied_at = datetime.now(UTC).strftime("%Y-%m-%d %H:%M:%S.%f")
        rows = [
            {
                "model": name,
                "relation": model.relation,
                "transform_version": model.version,
                "release": release,
                "applied_at": applied_at,
                "applied_by": "baseline",
            }
            for name, model in sorted(models.items())
        ]
        if rows:
            payload = "\n".join(json.dumps(row) for row in rows).encode()
            self._post(payload, query=f"INSERT INTO {TABLE} FORMAT JSONEachRow")


def parse_project(project: Path, connectors: Path, dbt: str) -> Manifest:
    with tempfile.TemporaryDirectory(prefix="transform-plan-") as temporary:
        directory = Path(temporary)
        environment = {
            **os.environ,
            "CLICKHOUSE_HOST": os.environ["RECONCILE_DEST_CLICKHOUSE_HOST"],
            "CLICKHOUSE_PORT": os.environ["RECONCILE_DEST_CLICKHOUSE_PORT"],
            "CLICKHOUSE_USER": os.environ["RECONCILE_DEST_CLICKHOUSE_USERNAME"],
            "CLICKHOUSE_PASSWORD": os.environ["RECONCILE_DEST_CLICKHOUSE_PASSWORD"],
            "CLICKHOUSE_PROTOCOL": os.environ["RECONCILE_DEST_CLICKHOUSE_PROTOCOL"],
            "CLICKHOUSE_CLUSTER_MODE": os.environ["RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE"],
            "CLICKHOUSE_CLUSTER_NAME": os.environ["RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME"],
            "DBT_SEND_ANONYMOUS_USAGE_STATS": "false",
            "DBT_TARGET_PATH": str(directory / "target"),
            "DBT_LOG_PATH": str(directory / "logs"),
        }
        environment.pop("CLICKHOUSE_URL", None)
        subprocess.run(
            [
                sys.executable,
                str(Path(__file__).resolve().parents[2] / "scripts/dbt_profiles.py"),
                "--target",
                "transform_plan",
                "--profiles-dir",
                str(directory),
                "--correlated-subqueries",
            ],
            env=environment,
            capture_output=True,
            text=True,
            check=True,
            timeout=30,
        )
        result = subprocess.run(
            [dbt, "parse", "--no-partial-parse", "--project-dir", str(project), "--profiles-dir", str(directory)],
            env=environment,
            capture_output=True,
            text=True,
            check=False,
            timeout=PARSE_TIMEOUT_SECS,
        )
        if result.returncode:
            raise ValueError(f"dbt parse failed: {result.stdout}\n{result.stderr}")
        data = json.loads((directory / "target/manifest.json").read_text())
        return read_manifest(object_value(data), project, connectors)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    logger = configure("transform_plan", run_id=os.environ.get("RECONCILE_RUN_ID"))
    try:
        connectors = Path(os.environ["CONNECTORS_DIR"]).resolve()
        manifest = parse_project(connectors.parent / "dbt", connectors, "dbt")
        ledger = cast(AppliedVersions, AppliedVersions.from_env())
        applied = ledger.latest()
        plan = plan_versions(manifest, applied)
        if plan.baseline and not args.dry_run:
            ledger.baseline(manifest.models, os.environ.get("INSIGHT_VERSION"))
        logger.info(
            "transform version plan", extra={"event": "transform.planned", "dry_run": args.dry_run, **plan.fields()}
        )
        return 0
    except (OSError, ValueError, TypeError, KeyError, LedgerError, subprocess.SubprocessError) as error:
        logger.error("transform planner failed", extra={"event": "transform.failed", "error": str(error)})
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
