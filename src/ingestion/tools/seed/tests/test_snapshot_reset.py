"""A snapshot over bronze the seed rewrites is pruned to its rebuilt source.

Snapshots refuse a dbt full refresh, so the silver step's `--full-refresh`
appends to them. A stand re-seeded with a different roster would otherwise keep
the previous roster's entities in the snapshot, and from there in history and
the identity chain, for good.
"""

from __future__ import annotations

import logging
import pathlib
import re
from dataclasses import dataclass, field
from typing import Any

import pytest

from insight_seed import silver
from insight_seed.generators import people, snapshots
from insight_seed.generators.insert import RESET_TARGETS

_INGESTION = pathlib.Path(silver.__file__).resolve().parents[3]
_GENERATORS = pathlib.Path(snapshots.__file__).resolve().parent
_MODEL_DIRS = (_INGESTION / "connectors", _INGESTION / "silver", _INGESTION / "dbt" / "models")
_SNAPSHOT_SCHEMA = "staging"

_SNAPSHOT_CALL = re.compile(r"\{\{\s*snapshot\(")
_SOURCE = re.compile(r"source\(\s*'(\w+)'\s*,\s*'(\w+)'\s*\)")
_REF = re.compile(r"ref\(\s*'(\w+)'\s*\)")
_SOURCE_SCHEMA = re.compile(
    r"^\s*-\s*name:\s*(\w+)\s*\n(?:[ \t]+(?!-)\S.*\n){0,3}?[ \t]+schema:\s*(\w+)", re.M
)
_SOURCE_REF = re.compile(
    r"source_ref\s*=\s*(?:source\(\s*'(\w+)'\s*,\s*'(\w+)'\s*\)|ref\(\s*'(\w+)'\s*\))"
)
_MODEL_SCHEMA = re.compile(r"schema\s*=\s*'(\w+)'")

Relation = tuple[str, str]


def _models() -> dict[str, str]:
    return {
        path.stem: path.read_text()
        for directory in _MODEL_DIRS
        for path in directory.rglob("*.sql")
        if "tests" not in path.parts
    }


def _source_schemas() -> dict[str, str]:
    """Source name to schema; dbt defaults the schema to the source's name."""
    schemas: dict[str, str] = {}
    for directory in _MODEL_DIRS:
        for path in directory.rglob("*.yml"):
            text = path.read_text()
            if "sources:" in text:
                schemas.update(dict(_SOURCE_SCHEMA.findall(text)))
    return schemas


def _bronze_read_by(model: str, models: dict[str, str], schemas: dict[str, str]) -> set[Relation]:
    """Every source relation `model` reads, through any chain of refs."""
    found: set[Relation] = set()
    pending = [model]
    seen: set[str] = set()
    while pending:
        name = pending.pop()
        if name in seen or name not in models:
            continue
        seen.add(name)
        body = models[name]
        found.update(
            (schemas.get(source, source), table) for source, table in _SOURCE.findall(body)
        )
        pending.extend(_REF.findall(body))
    return found


def _snapshots_over_seeded_bronze() -> dict[Relation, set[Relation]]:
    seeded_bronze = {target for target in RESET_TARGETS if target[0].startswith("bronze_")}
    models = _models()
    schemas = _source_schemas()

    found: dict[Relation, set[Relation]] = {}
    for name, body in models.items():
        if not _SNAPSHOT_CALL.search(body):
            continue
        read = _bronze_read_by(name, models, schemas) & seeded_bronze
        if read:
            found[(_SNAPSHOT_SCHEMA, name)] = read
    return found


def _source_ref_of(model: str, models: dict[str, str], schemas: dict[str, str]) -> Relation:
    """The relation a snapshot model's `source_ref` names."""
    match = _SOURCE_REF.search(models[model])
    assert match, f"{model}: no source_ref in its snapshot() call"
    source, table, ref = match.groups()
    if ref is None:
        return (schemas.get(source, source), table)
    schema = _MODEL_SCHEMA.search(models[ref])
    assert schema, f"{ref}: no schema in its config"
    return (schema.group(1), ref)


def test_every_snapshot_over_seeded_bronze_is_registered() -> None:
    found = _snapshots_over_seeded_bronze()

    assert found, "no snapshot reads seeded bronze: the scan no longer finds them"
    assert set(snapshots.SEEDED_SNAPSHOTS) == set(found), (
        "SEEDED_SNAPSHOTS must name every snapshot over bronze the seed rewrites: "
        f"registered {sorted(snapshots.SEEDED_SNAPSHOTS)}, found {found}"
    )


def test_every_registered_snapshot_is_pruned_against_its_source_ref() -> None:
    models = _models()
    schemas = _source_schemas()

    for snapshot, source in snapshots.SEEDED_SNAPSHOTS.items():
        expected = _source_ref_of(snapshot[1], models, schemas)
        assert source == expected, f"{snapshot}: registered {source}, source_ref is {expected}"


def test_every_model_a_snapshot_reads_is_rebuilt_before_the_prune() -> None:
    models = _models()

    for snapshot, (schema, table) in snapshots.SEEDED_SNAPSHOTS.items():
        if schema.startswith("bronze_"):
            continue
        assert table in models, f"{snapshot}: source {table} is not a dbt model"
        assert table in snapshots.source_models(), f"{snapshot}: {table} is not rebuilt"


def test_the_stale_keys_are_this_tenants_keys_the_source_lacks() -> None:
    query = snapshots.stale_keys_query(
        ("staging", "example__users_snapshot"), ("staging", "example__users_latest")
    )

    assert query.startswith("SELECT DISTINCT unique_key FROM `staging`.`example__users_snapshot`")
    assert query.count("tenant_id = {tenant:String}") == 2, f"should scope both sides: {query!r}"
    assert "NOT IN (SELECT unique_key FROM `staging`.`example__users_latest` FINAL" in query, (
        f"should read the deduplicated source: {query!r}"
    )


def test_the_delete_removes_only_listed_keys_of_one_tenant() -> None:
    statement = snapshots.delete_statement(("staging", "example__users_snapshot"))

    assert statement.startswith("ALTER TABLE `staging`.`example__users_snapshot` DELETE")
    assert "tenant_id = {tenant:String}" in statement, f"should scope to one tenant: {statement!r}"
    assert "has({keys:Array(String)}, unique_key)" in statement, (
        f"should delete listed keys: {statement!r}"
    )


@pytest.mark.parametrize(
    ("count", "expected_sizes"),
    [(0, []), (1, [1]), (3, [3]), (4, [3, 1]), (7, [3, 3, 1])],
)
def test_the_stale_keys_are_deleted_in_bounded_batches(
    count: int, expected_sizes: list[int]
) -> None:
    keys = [f"key-{index}" for index in range(count)]

    sizes = [len(batch) for batch in snapshots.batches(keys, size=3)]

    assert sizes == expected_sizes, f"should batch {count} keys as {expected_sizes}: {sizes}"


def test_the_seeded_employee_key_is_the_connectors_tenant_source_and_employee_id() -> None:
    tenant = "00000000-0000-4000-8000-000000000001"

    key = people.employee_unique_key(tenant, "example-employee")

    assert key == f"{tenant}-seed-bamboohr-example-employee", f"should match the connector: {key!r}"


def test_the_scan_follows_a_ref_chain_to_its_bronze() -> None:
    models = {
        "example__users_snapshot": "{{ snapshot(source_ref=ref('example__users_latest')) }}",
        "example__users_latest": "SELECT * FROM {{ source('example', 'users') }}",
    }

    found = _bronze_read_by("example__users_snapshot", models, {"example": "bronze_example"})

    assert found == {("bronze_example", "users")}, f"should resolve through the ref: {found!r}"


@dataclass
class _Client:
    """Answers EXISTS and the source count; records nothing else worth asserting."""

    source_rows: int
    commands: list[str] = field(default_factory=list)

    def command(self, sql: str, **_: Any) -> int:
        self.commands.append(sql)
        return self.source_rows if sql.startswith("SELECT count()") else 1

    def query(self, sql: str, **_: Any) -> Any:
        return type("Result", (), {"result_rows": []})()


@pytest.mark.parametrize(
    ("source_rows", "warned"), [(0, True), (5, False)], ids=["empty source", "held source"]
)
def test_an_empty_source_is_reported_before_its_snapshot_is_pruned(
    caplog: pytest.LogCaptureFixture, source_rows: int, warned: bool
) -> None:
    snapshot = ("staging", "bamboohr__employees_snapshot")

    with caplog.at_level(logging.WARNING, logger=snapshots.LOG.name):
        snapshots.prune_to_source(
            _Client(source_rows), snapshot, "00000000-0000-4000-8000-000000000001"
        )

    assert any("holds nothing" in record.message for record in caplog.records) is warned, (
        f"should {'warn' if warned else 'stay quiet'} for {source_rows} source rows: {caplog.text}"
    )
