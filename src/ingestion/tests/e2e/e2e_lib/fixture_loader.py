"""Fixture loader: read fixtures/<name>/ into a typed Fixture value.

A fixture is a folder with:

    fixtures/<name>/
      bronze/<schema>.<table>.csv  - one or more bronze inputs
      spec.yaml                    - test config (JSON-schema-validated)
      expected/response.csv        - expected API response shape (flat CSV)

The loader is invoked at pytest collection time so misshapen fixtures fail
fast — long before the docker compose stack even comes up.
"""

from __future__ import annotations

import logging
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import jsonschema
import pandas as pd
import yaml

from e2e_lib.spec_schema import SPEC_SCHEMA

LOG = logging.getLogger("e2e.fixture")


class FixtureError(ValueError):
    """Raised for any malformed fixture — message is shown to pytest at collect time."""


# Bronze CSV filename convention: `<schema>.<table>.csv`.
# Examples:
#   bronze_bamboohr.employees.csv
#   bronze_jira.changelogs.csv
_BRONZE_CSV_NAME = re.compile(r"^(?P<schema>bronze_[a-z0-9_]+)\.(?P<table>[a-z0-9_]+)\.csv$")

# Silver CSV filename convention: just `<table>.csv` (schema is always "silver").
# Used to bypass Rust enrich for tests that need to start from already-processed
# silver events (e.g. class_task_field_history). See HOW-TO-ADD-FIXTURE.md.
_SILVER_CSV_NAME = re.compile(r"^(?P<table>[a-z0-9_]+)\.csv$")


@dataclass(frozen=True)
class BronzeCsv:
    """One CSV file mapping onto a bronze table."""

    path: Path
    schema: str  # e.g. "bronze_bamboohr"
    table: str   # e.g. "employees"

    @property
    def full_name(self) -> str:
        return f"{self.schema}.{self.table}"


@dataclass(frozen=True)
class SilverCsv:
    """One CSV file mapping onto a silver table.

    Tests that include silver/* files trade some pipeline coverage (the
    Rust enrich + dbt union steps) for ability to exercise downstream
    gold views. Use bronze-only when possible; reach for silver/ when the
    pipeline above silver depends on a Rust binary not in the rig.
    """

    path: Path
    table: str   # e.g. "class_task_field_history"
    schema: str = "silver"

    @property
    def full_name(self) -> str:
        return f"{self.schema}.{self.table}"


@dataclass(frozen=True)
class SpecYaml:
    """Parsed and validated spec.yaml."""

    spec_version: int
    endpoint: str
    request_body: dict[str, Any]
    key_columns: list[str]
    dbt_selector: str | None = None
    method: str = "POST"
    description: str = ""
    metric_id: str | None = None
    float_tolerance: float = 1e-6

    @classmethod
    def from_dict(cls, raw: dict) -> "SpecYaml":
        # JSON Schema does the heavy lifting; this constructor only translates
        # validated dict -> dataclass. Defaults mirror the schema's `default` keys.
        return cls(
            spec_version=raw["spec_version"],
            endpoint=raw["endpoint"],
            request_body=raw["request_body"],
            dbt_selector=raw.get("dbt_selector"),
            key_columns=list(raw["key_columns"]),
            method=raw.get("method", "POST"),
            description=raw.get("description", ""),
            metric_id=raw.get("metric_id"),
            float_tolerance=float(raw.get("float_tolerance", 1e-6)),
        )

    def resolved_endpoint(self) -> str:
        """Interpolate `{metric_id}` placeholder if present in `endpoint`."""
        if "{metric_id}" not in self.endpoint:
            return self.endpoint
        if not self.metric_id:
            raise FixtureError(
                f"endpoint references {{metric_id}} but no metric_id in spec: {self.endpoint}"
            )
        return self.endpoint.replace("{metric_id}", self.metric_id)


@dataclass(frozen=True)
class Fixture:
    """A loaded fixture folder."""

    name: str
    root: Path
    spec: SpecYaml
    bronze_csvs: list[BronzeCsv] = field(default_factory=list)
    silver_csvs: list[SilverCsv] = field(default_factory=list)
    expected_df: pd.DataFrame = field(default_factory=pd.DataFrame)

    @property
    def touched_schemas(self) -> set[str]:
        """Schemas this fixture writes into — informs the per-test TRUNCATE set."""
        return {csv.schema for csv in self.bronze_csvs} | {csv.schema for csv in self.silver_csvs}

    @property
    def touched_tables(self) -> set[tuple[str, str]]:
        """(schema, table) pairs the fixture touches."""
        return (
            {(c.schema, c.table) for c in self.bronze_csvs}
            | {(c.schema, c.table) for c in self.silver_csvs}
        )


def load(fixture_root: Path) -> Fixture:
    """Load a single fixture folder. Raises FixtureError on any malformation."""
    if not fixture_root.is_dir():
        raise FixtureError(f"fixture directory not found: {fixture_root}")

    spec = _load_spec(fixture_root)
    bronze_csvs = _enumerate_bronze_csvs(fixture_root)
    silver_csvs = _enumerate_silver_csvs(fixture_root)
    expected_df = _load_expected(fixture_root, key_columns=spec.key_columns)

    return Fixture(
        name=fixture_root.name,
        root=fixture_root,
        spec=spec,
        bronze_csvs=bronze_csvs,
        silver_csvs=silver_csvs,
        expected_df=expected_df,
    )


def discover_all(fixtures_root: Path) -> list[Path]:
    """List candidate fixture folders under `fixtures/` (one level deep)."""
    if not fixtures_root.is_dir():
        return []
    candidates = []
    for child in sorted(fixtures_root.iterdir()):
        # Skip dotfiles, READMEs, hidden caches
        if child.name.startswith(".") or child.name.startswith("_"):
            continue
        if child.is_dir():
            candidates.append(child)
    return candidates


# ---------------------------------------------------------------------------
# internals
# ---------------------------------------------------------------------------


def _load_spec(root: Path) -> SpecYaml:
    spec_path = root / "spec.yaml"
    if not spec_path.is_file():
        raise FixtureError(f"missing spec.yaml in {root}")
    raw_text = spec_path.read_text(encoding="utf-8")
    try:
        raw = yaml.safe_load(raw_text)
    except yaml.YAMLError as e:
        raise FixtureError(f"{spec_path}: invalid YAML: {e}") from e
    if not isinstance(raw, dict):
        raise FixtureError(f"{spec_path}: top-level must be a mapping, got {type(raw).__name__}")
    try:
        jsonschema.validate(raw, SPEC_SCHEMA)
    except jsonschema.ValidationError as e:
        raise FixtureError(
            f"{spec_path}: spec.yaml does not match schema:\n  {e.message}\n  at path: {list(e.absolute_path)}"
        ) from e
    return SpecYaml.from_dict(raw)


def _enumerate_bronze_csvs(root: Path) -> list[BronzeCsv]:
    bronze_dir = root / "bronze"
    if not bronze_dir.is_dir():
        raise FixtureError(f"missing bronze/ directory in {root}")
    csvs = []
    for entry in sorted(bronze_dir.iterdir()):
        if not entry.is_file() or not entry.name.endswith(".csv"):
            continue
        m = _BRONZE_CSV_NAME.match(entry.name)
        if not m:
            raise FixtureError(
                f"{entry}: bronze CSV name must match `<bronze_schema>.<table>.csv`, "
                f"e.g. `bronze_bamboohr.employees.csv`"
            )
        csvs.append(BronzeCsv(path=entry, schema=m["schema"], table=m["table"]))
    if not csvs:
        raise FixtureError(f"no bronze CSV files found in {bronze_dir}")
    return csvs


def _enumerate_silver_csvs(root: Path) -> list[SilverCsv]:
    """Optional silver/ directory: files named `<table>.csv` (schema implicit)."""
    silver_dir = root / "silver"
    if not silver_dir.is_dir():
        return []
    csvs = []
    for entry in sorted(silver_dir.iterdir()):
        if not entry.is_file() or not entry.name.endswith(".csv"):
            continue
        m = _SILVER_CSV_NAME.match(entry.name)
        if not m:
            raise FixtureError(
                f"{entry}: silver CSV name must match `<table>.csv` "
                f"(no schema prefix; the schema is always `silver`)"
            )
        csvs.append(SilverCsv(path=entry, table=m["table"]))
    return csvs


def _load_expected(root: Path, *, key_columns: list[str]) -> pd.DataFrame:
    expected = root / "expected" / "response.csv"
    if not expected.is_file():
        raise FixtureError(f"missing expected/response.csv in {root}")
    try:
        df = pd.read_csv(expected)
    except Exception as e:
        raise FixtureError(f"{expected}: failed to read CSV: {e}") from e
    missing = [c for c in key_columns if c not in df.columns]
    if missing:
        raise FixtureError(
            f"{expected}: key_columns missing from expected CSV: {missing} "
            f"(have: {list(df.columns)})"
        )
    return df
