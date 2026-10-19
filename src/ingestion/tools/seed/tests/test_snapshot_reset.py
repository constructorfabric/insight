"""A snapshot over bronze the seed rewrites is pruned to that bronze.

Snapshots refuse a dbt full refresh, so the silver step's `--full-refresh`
appends to them. A stand re-seeded with a different roster would otherwise keep
the previous roster's people in the snapshot, and from there in the identity
chain, for good.
"""

from __future__ import annotations

import pathlib
import re

from insight_seed import silver
from insight_seed.generators import snapshots
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
_PRUNE_CALL = re.compile(r'prune_to_bronze\(\s*client,\s*\("(\w+)",\s*"(\w+)"\)')

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


def test_every_snapshot_over_seeded_bronze_is_registered() -> None:
    found = _snapshots_over_seeded_bronze()

    assert found, "no snapshot reads seeded bronze: the scan no longer finds them"
    assert set(snapshots.SEEDED_SNAPSHOTS) == set(found), (
        "SEEDED_SNAPSHOTS must name every snapshot over bronze the seed rewrites: "
        f"registered {sorted(snapshots.SEEDED_SNAPSHOTS)}, found {sorted(found)}"
    )


def test_every_registered_snapshot_takes_its_keys_from_bronze_it_reads() -> None:
    models = _models()
    schemas = _source_schemas()

    for snapshot, sources in snapshots.SEEDED_SNAPSHOTS.items():
        read = _bronze_read_by(snapshot[1], models, schemas)
        strays = {source.bronze for source in sources} - read
        assert sources, f"{snapshot} names no bronze to keep its keys from"
        assert not strays, f"{snapshot} does not read {strays}; it reads {read}"


def test_every_registered_snapshot_is_pruned_by_a_generator() -> None:
    called: set[Relation] = set()
    for path in _GENERATORS.glob("*.py"):
        called.update(_PRUNE_CALL.findall(path.read_text()))

    assert called == set(snapshots.SEEDED_SNAPSHOTS), f"pruned {called}"


def test_the_prune_keeps_other_tenants_and_the_keys_bronze_holds() -> None:
    statement = snapshots.prune_statement(
        ("staging", "example__users_snapshot"),
        (
            snapshots.KeySource(("bronze_example", "users")),
            snapshots.KeySource(("bronze_example", "members"), "concat(source_id, '-', member_id)"),
        ),
    )

    assert statement.startswith("ALTER TABLE `staging`.`example__users_snapshot` DELETE")
    assert "WHERE tenant_id = {tenant:String} AND unique_key NOT IN (" in statement, (
        f"should scope to one tenant: {statement!r}"
    )
    assert (
        "SELECT unique_key FROM `bronze_example`.`users` WHERE tenant_id = {tenant:String}"
        " UNION ALL "
        "SELECT concat(source_id, '-', member_id) FROM `bronze_example`.`members`"
        " WHERE tenant_id = {tenant:String}"
    ) in statement, f"should keep every key this tenant's bronze holds: {statement!r}"


def test_the_scan_follows_a_ref_chain_to_its_bronze() -> None:
    models = {
        "example__users_snapshot": "{{ snapshot(source_ref=ref('example__users_latest')) }}",
        "example__users_latest": "SELECT * FROM {{ source('example', 'users') }}",
    }

    found = _bronze_read_by("example__users_snapshot", models, {"example": "bronze_example"})

    assert found == {("bronze_example", "users")}, f"should resolve through the ref: {found!r}"
