"""Load `<name>.test.yaml` into a typed TestYaml value.

A test file (discovered by the `*.test.yaml` suffix) has:

    bronze:                         # what to seed, keyed by table name
      <db>.<table>:
        - $ref: templates/<g>.yaml#/templates/<rec>   # inherit a record
          <field>: <override>                          # + fields under test
    cases:                          # batch request → expect rules
      - name: ...
        request: { url, method, body: { queries: [...] } }
        expect: [ {in?, find?, equal?|assert?}, ... ]

Bronze rows are `$ref`-resolved (`ref_resolver`), padded and validated against
`schemas/<table>.yaml` (`schema_validator`) at load time, so a bad ref or a
misspelled column fails at pytest collection — before the stack comes up.

Shared `schemas/` and `templates/` files are NOT tests (no `cases`).
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from pathlib import Path

import yaml

from lib import ref_resolver, schema_validator

LOG = logging.getLogger("e2e.fixture")


class FixtureError(ValueError):
    """Malformed test file — message is shown to pytest at collect time."""


@dataclass(frozen=True)
class TestYaml:
    __test__ = False  # not a pytest test class despite the `Test` prefix
    name: str
    path: Path
    # table fqn ("bronze_m365.email_activity") -> list of resolved+padded rows
    bronze: dict[str, list[dict]] = field(default_factory=dict)
    cases: list[dict] = field(default_factory=list)

    @property
    def touched_tables(self) -> set[tuple[str, str]]:
        out = set()
        for fqn in self.bronze:
            schema, _, table = fqn.partition(".")
            out.add((schema, table))
        return out


def discover_tests(specs_root: Path) -> list[Path]:
    """Every `**/*.test.yaml` under metrics/. Shared schemas/templates are excluded
    by the suffix; nothing else is collected as a test."""
    if not specs_root.is_dir():
        return []
    return sorted(specs_root.rglob("*.test.yaml"))


def load(path: Path, *, schemas_dir: Path | None = None) -> TestYaml:
    """Load and fully resolve one test file. Raises FixtureError on any problem."""
    if not path.is_file():
        raise FixtureError(f"test file not found: {path}")
    if schemas_dir is None:
        schemas_dir = _find_schemas_dir(path)

    try:
        doc = yaml.safe_load(path.read_text(encoding="utf-8"))
    except yaml.YAMLError as e:
        raise FixtureError(f"{path}: invalid YAML: {e}") from e
    if not isinstance(doc, dict):
        raise FixtureError(f"{path}: top-level must be a mapping")
    if "cases" not in doc:
        raise FixtureError(f"{path}: a test must define `cases`")

    bronze_doc = doc.get("bronze") or {}
    if not isinstance(bronze_doc, dict):
        raise FixtureError(f"{path}: `bronze` must be a mapping of table → records")
    bronze: dict[str, list[dict]] = {}
    for table, rows in bronze_doc.items():
        if not isinstance(rows, list):
            raise FixtureError(f"{path}: bronze.{table} must be a list of records")
        try:
            schema = schema_validator.load_schema(schemas_dir, table)
        except schema_validator.SchemaError as e:
            raise FixtureError(str(e)) from e
        resolved: list[dict] = []
        for idx, row in enumerate(rows):
            try:
                merged = ref_resolver.resolve(row, path)
            except ref_resolver.RefError as e:
                raise FixtureError(f"{path}: bronze.{table}[{idx}]: {e}") from e
            if not isinstance(merged, dict):
                raise FixtureError(f"{path}: bronze.{table}[{idx}] did not resolve to a record")
            try:
                resolved.append(schema_validator.pad_and_validate(merged, schema, table=table))
            except schema_validator.SchemaError as e:
                raise FixtureError(f"{path}: bronze.{table}[{idx}]: {e}") from e
        bronze[table] = resolved

    cases = doc["cases"]
    if not isinstance(cases, list) or not cases:
        raise FixtureError(f"{path}: `cases` must be a non-empty list")
    for i, case in enumerate(cases):
        if not isinstance(case, dict) or "request" not in case or "expect" not in case:
            raise FixtureError(f"{path}: cases[{i}] must be a mapping with `request` and `expect`")

    return TestYaml(name=path.name[: -len(".test.yaml")], path=path, bronze=bronze, cases=cases)


def _find_schemas_dir(test_path: Path) -> Path:
    """Walk up to the `metrics/` dir and use its `schemas/` subdir."""
    for parent in test_path.parents:
        if parent.name == "metrics":
            return parent / "schemas"
        if (parent / "schemas").is_dir():
            return parent / "schemas"
    raise FixtureError(f"{test_path}: could not locate a sibling `schemas/` directory")
