from __future__ import annotations

import json
import os
import subprocess
import sys
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))

from transform_plan import Json, Manifest, plan_versions, read_manifest

ROOT = Path(__file__).resolve().parents[1]


def model(version: int = 1, full_refresh: bool | None = None) -> dict[str, Json]:
    return {
        "resource_type": "model",
        "relation_name": None,
        "database": "example_database",
        "schema": "silver",
        "alias": "example",
        "config": {"meta": {"transform_version": version}, "full_refresh": full_refresh},
    }


def manifest_data(version: int = 1) -> dict[str, Json]:
    return {
        "nodes": {
            "model.example.root": model(version),
            "model.example.silver": model(full_refresh=False),
            "model.example.gold": model(),
            "model.example.unrelated": model(),
            "test.example.check": {"resource_type": "test"},
        },
        "sources": {},
        "child_map": {
            "model.example.root": ["model.example.silver"],
            "model.example.silver": ["model.example.gold", "test.example.check"],
        },
        "parent_map": {"model.example.silver": ["model.example.root"], "model.example.gold": ["model.example.silver"]},
    }


def parsed(tmp_path: Path, version: int = 1) -> Manifest:
    return read_manifest(manifest_data(version), tmp_path / "dbt", tmp_path / "connectors")


def current(manifest: Manifest) -> dict[str, int]:
    return {name: item.version for name, item in manifest.models.items()}


def test_empty_history_baselines_every_model_without_rebuilding(tmp_path: Path) -> None:
    manifest = parsed(tmp_path, version=7)
    plan = plan_versions(manifest, {})
    assert plan.baseline == tuple(sorted(manifest.models))
    assert plan.rebuild == plan.changed == plan.new == plan.removed == plan.incremental_only == plan.connectors == ()


@pytest.mark.parametrize("database", [None, "", "example_database"])
def test_relation_uses_dbt_resolved_components_without_a_relation_name(tmp_path: Path, database: Json) -> None:
    data = manifest_data()
    data["nodes"] = {"model.example.root": {**model(), "database": database}}
    manifest = read_manifest(data, tmp_path / "dbt", tmp_path / "connectors")
    expected = "example_database.silver.example" if database else "silver.example"
    assert manifest.models["model.example.root"].relation == expected


def test_equal_versions_plan_nothing(tmp_path: Path) -> None:
    manifest = parsed(tmp_path)
    plan = plan_versions(manifest, current(manifest))
    assert all(value == () for key, value in plan.fields().items() if key != "counts")


@pytest.mark.parametrize("version", [2, 7])
def test_any_version_difference_rebuilds_all_model_descendants(tmp_path: Path, version: int) -> None:
    manifest = parsed(tmp_path, version=version)
    applied = current(manifest)
    applied["model.example.root"] = 3
    plan = plan_versions(manifest, applied)
    assert plan.changed == ("model.example.root",), f"version={version}"
    assert plan.rebuild == ("model.example.gold", "model.example.root", "model.example.silver"), f"version={version}"
    assert plan.baseline == plan.new == plan.removed == ()


def test_new_models_rebuild_their_descendants(tmp_path: Path) -> None:
    manifest = parsed(tmp_path)
    applied = current(manifest)
    del applied["model.example.root"]
    plan = plan_versions(manifest, applied)
    assert plan.new == ("model.example.root",)
    assert plan.rebuild == ("model.example.gold", "model.example.root", "model.example.silver")
    assert plan.changed == plan.removed == ()


def test_removed_models_are_reported_without_rebuilding(tmp_path: Path) -> None:
    manifest = parsed(tmp_path)
    plan = plan_versions(manifest, {**current(manifest), "model.example.removed": 1})
    assert plan.removed == ("model.example.removed",)
    assert plan.rebuild == plan.changed == plan.new == ()


@pytest.mark.parametrize("bronze", [True, False])
def test_locks_follow_bronze_source_declarations_and_descriptor_names(tmp_path: Path, bronze: bool) -> None:
    data = manifest_data(version=2)
    sources: dict[str, Json] = {}
    for directory, name in (("first", "example-connector-a"), ("second", "example-connector-b")):
        connector = tmp_path / "connectors" / "category" / directory
        connector.mkdir(parents=True)
        (connector / "descriptor.yaml").write_text(f"name: {name}\n")
        sources[f"source.example.{directory}"] = {
            "schema": f"bronze_{directory}" if bronze else "config",
            "original_file_path": f"../connectors/category/{directory}/dbt/schema.yml",
        }
    data["sources"] = sources
    data["parent_map"] = {
        "model.example.root": ["source.example.first", "source.example.second"],
        "model.example.silver": ["model.example.root"],
        "model.example.gold": ["model.example.silver"],
    }
    manifest = read_manifest(data, tmp_path / "dbt", tmp_path / "connectors")
    applied = current(manifest)
    applied["model.example.gold"] = 3
    plan = plan_versions(manifest, applied)
    expected = ("example-connector-a", "example-connector-b") if bronze else ()
    assert plan.connectors == expected, f"bronze={bronze}"


def test_full_refresh_false_is_incremental_only_within_the_rebuild(tmp_path: Path) -> None:
    manifest = parsed(tmp_path, version=2)
    applied = dict.fromkeys(manifest.models, 1)
    plan = plan_versions(manifest, applied)
    assert plan.incremental_only == ("model.example.silver",)
    assert set(plan.incremental_only) <= set(plan.rebuild)


@pytest.mark.parametrize("invalid", [None, True, 0, -1, "2"])
def test_invalid_versions_refuse_to_plan(tmp_path: Path, invalid: Json) -> None:
    data = manifest_data()
    data["nodes"] = {"model.example.root": {**model(), "config": {"meta": {"transform_version": invalid}}}}
    with pytest.raises(ValueError, match="positive integer"):
        read_manifest(data, tmp_path / "dbt", tmp_path / "connectors")


@dataclass
class Warehouse:
    directory: Path
    rows: list[dict[str, Json]] = field(default_factory=list)
    fail_read: bool = False

    def prepare(self) -> None:
        (self.directory / "state.json").write_text(json.dumps({"rows": self.rows, "fail_read": self.fail_read}))

    @property
    def requests(self) -> list[tuple[dict[str, list[str]], str]]:
        path = self.directory / "requests.jsonl"
        if not path.exists():
            return []
        return [tuple(json.loads(line)) for line in path.read_text().splitlines()]


@pytest.fixture
def warehouse(tmp_path: Path) -> Iterator[Warehouse]:
    warehouse = Warehouse(tmp_path)
    warehouse.prepare()
    yield warehouse


@pytest.fixture
def environment(tmp_path: Path, warehouse: Warehouse) -> dict[str, str]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "sitecustomize.py").write_text(
        "import io, json, os, pathlib, urllib.error, urllib.parse, urllib.request\n"
        "def urlopen(request, timeout):\n"
        "    directory = pathlib.Path(os.environ['TEST_WAREHOUSE'])\n"
        "    state = json.loads((directory / 'state.json').read_text())\n"
        "    parameters = urllib.parse.parse_qs(urllib.parse.urlparse(request.full_url).query)\n"
        "    body = request.data.decode()\n"
        "    with (directory / 'requests.jsonl').open('a') as output:\n"
        "        output.write(json.dumps([parameters, body]) + '\\n')\n"
        "    if state['fail_read'] and body.startswith('SELECT'):\n"
        "        raise urllib.error.HTTPError(request.full_url, 500, 'error', {}, io.BytesIO(b'synthetic read failure'))\n"
        "    rows = state['rows'] if body.startswith('SELECT') else []\n"
        "    return io.BytesIO('\\n'.join(json.dumps(row) for row in rows).encode())\n"
        "urllib.request.urlopen = urlopen\n"
    )
    python = bin_dir / "python3"
    python.write_text(f'#!/bin/sh\nPYTHONPATH="{bin_dir}:$PYTHONPATH" exec "{sys.executable}" "$@"\n')
    python.chmod(0o755)
    dbt = bin_dir / "dbt"
    dbt.write_text(
        f"#!{sys.executable}\n"
        "import json, os, pathlib, sys\n"
        "assert sys.argv[1] == 'parse', sys.argv\n"
        "target = pathlib.Path(os.environ['DBT_TARGET_PATH'])\n"
        "target.mkdir(parents=True)\n"
        f"(target / 'manifest.json').write_text({json.dumps(manifest_data(version=7))!r})\n"
    )
    dbt.chmod(0o755)
    return {
        **os.environ,
        "PATH": f"{bin_dir}:{os.environ['PATH']}",
        "PYTHONPATH": f"{bin_dir}:{ROOT / 'python'}:{ROOT.parent / 'scripts'}",
        "CONNECTORS_DIR": str(tmp_path / "connectors"),
        "RECONCILE_DEST_CLICKHOUSE_HOST": "127.0.0.1",
        "RECONCILE_DEST_CLICKHOUSE_PORT": "1",
        "TEST_WAREHOUSE": str(warehouse.directory),
        "RECONCILE_DEST_CLICKHOUSE_USERNAME": "example-user",
        "RECONCILE_DEST_CLICKHOUSE_PASSWORD": "example-password",
        "RECONCILE_DEST_CLICKHOUSE_PROTOCOL": "http",
        "RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE": "true",
        "RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME": "example_cluster",
        "RECONCILE_DRY_RUN": "0",
        "RECONCILE_RUN_ID": "example-pass",
        "INSIGHT_NAMESPACE": "example",
        "ARGO_SERVICE_ACCOUNT": "example-reconcile",
        "INSIGHT_LOG_LEVEL": "info",
        "INSIGHT_VERSION": "1.2.3",
    }


@pytest.mark.parametrize("dry_run", [True, False])
def test_only_first_adoption_writes_current_baseline_rows(
    environment: dict[str, str], warehouse: Warehouse, dry_run: bool
) -> None:
    args = ["bash", str(ROOT / "transform-plan.sh")]
    if dry_run:
        args.append("--dry-run")
    result = subprocess.run(args, env=environment, text=True, capture_output=True, check=False)
    assert result.returncode == 0, result.stderr
    assert result.stdout == ""
    lines = [json.loads(line) for line in result.stderr.splitlines()]
    assert len(lines) == 1
    event = lines[0]
    assert set(event) == {"timestamp", "level", "fields", "target"}
    assert event["target"] == "transform_plan"
    assert event["fields"]["event"] == "transform.planned"
    assert event["fields"]["run_id"] == "example-pass"
    assert event["fields"]["counts"]["baseline"] == 4
    assert event["fields"]["counts"]["rebuild"] == 0
    assert len(warehouse.requests) == (1 if dry_run else 2)
    parameters, sql = warehouse.requests[0]
    assert "argMax(transform_version, applied_at)" in sql
    assert "GROUP BY model" in sql
    assert parameters["select_sequential_consistency"] == ["1"]
    if dry_run:
        return
    parameters, body = warehouse.requests[1]
    assert parameters["query"] == ["INSERT INTO ingestion_history.transform_versions FORMAT JSONEachRow"]
    assert parameters["insert_quorum"] == ["auto"]
    assert parameters["insert_quorum_parallel"] == ["0"]
    rows = [json.loads(line) for line in body.splitlines()]
    assert {row["model"] for row in rows} == set(manifest_data()["nodes"]) - {"test.example.check"}
    assert {row["applied_by"] for row in rows} == {"baseline"}
    assert {row["release"] for row in rows} == {"1.2.3"}
    assert {row["relation"] for row in rows} == {"example_database.silver.example"}
    assert len({row["applied_at"] for row in rows}) == 1
    assert {row["model"]: row["transform_version"] for row in rows}["model.example.root"] == 7


@pytest.mark.parametrize("release", ["1.2.3", None])
def test_baseline_rows_record_insight_version(
    environment: dict[str, str], warehouse: Warehouse, release: str | None
) -> None:
    if release is None:
        environment.pop("INSIGHT_VERSION")
    else:
        environment["INSIGHT_VERSION"] = release

    result = subprocess.run(
        [sys.executable, "-m", "transform_plan"], env=environment, text=True, capture_output=True, check=False
    )
    assert result.returncode == 0, result.stderr
    assert len(warehouse.requests) == 2
    rows = [json.loads(line) for line in warehouse.requests[1][1].splitlines()]
    assert len(rows) == 4
    assert {row["release"] for row in rows} == {release}, f"INSIGHT_VERSION={release!r}"


@pytest.mark.parametrize("version", [1, "1"])
def test_existing_history_is_read_without_any_write(
    environment: dict[str, str], warehouse: Warehouse, version: Json
) -> None:
    warehouse.rows = [{"model": name, "transform_version": version} for name in parsed(Path("/tmp")).models]
    warehouse.prepare()
    result = subprocess.run(
        [sys.executable, "-m", "transform_plan"], env=environment, text=True, capture_output=True, check=False
    )
    assert result.returncode == 0, result.stderr
    assert len(warehouse.requests) == 1
    event = json.loads(result.stderr)
    assert event["fields"]["changed"] == ["model.example.root"]
    assert event["fields"]["counts"]["rebuild"] == 3


def test_clickhouse_read_failure_fails_the_planner_without_affecting_reconcile(
    environment: dict[str, str], warehouse: Warehouse
) -> None:
    warehouse.fail_read = True
    warehouse.prepare()
    result = subprocess.run(
        ["bash", str(ROOT / "transform-plan.sh")], env=environment, text=True, capture_output=True, check=False
    )
    assert result.returncode == 1, result.stderr
    assert result.stdout == ""
    event = json.loads(result.stderr)
    assert event["level"] == "ERROR"
    assert event["fields"]["event"] == "transform.failed"
    assert "synthetic read failure" in event["fields"]["error"]
    assert len(warehouse.requests) == 1

    script = f'''
set -euo pipefail
export AIRBYTE_URL=http://127.0.0.1:1
source "{ROOT}/lib/reconcile.sh"
log_line() {{ printf '%s %s\\n' "$1" "$2"; }}
log_init() {{ :; }}
log_close() {{ :; }}
log_run_summary() {{ printf 'summary %s %s\\n' "$1" "$2"; }}
disc_load_instances() {{ :; }}
reconcile_prune_removed_instances() {{ :; }}
reconcile_gc_orphans() {{ :; }}
sweep_run() {{ printf 'sweep completed\\n'; }}
reconcile_run 0 1 1 "" ""
printf 'reconcile continued\\n' 
'''
    result = subprocess.run(["bash", "-c", script], env=environment, text=True, capture_output=True, check=False)
    assert result.returncode == 0, result.stderr
    assert "ERROR transform planner failed" not in result.stdout
    assert result.stdout.index("sweep completed") < result.stdout.index("summary 0 0")
    assert "summary 0 0" in result.stdout
    assert result.stdout.endswith("reconcile continued\n")
    assert result.stderr == ""
    assert len(warehouse.requests) == 1
