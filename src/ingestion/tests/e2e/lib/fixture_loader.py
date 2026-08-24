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
class IdentityAccount:
    """One source-account binding from a fixture's `identity_accounts`.

    `person` is a persona email, or the literal 'excluded' for the reserved
    bot person. `source_id` is the RAW connector source id; the rig hashes it
    the way the connectors mint insight_source_id.
    """

    source_type: str
    source_id: str
    account_id: str
    person: str


@dataclass(frozen=True)
class TestYaml:
    __test__ = False  # not a pytest test class despite the `Test` prefix
    name: str
    path: Path
    # table fqn ("bronze_m365.email_activity") -> list of resolved+padded rows
    bronze: dict[str, list[dict]] = field(default_factory=dict)
    schemas: dict[str, dict] = field(default_factory=dict)
    cases: list[dict] = field(default_factory=list)
    # Optional top-level `skip: <reason>` in the .test.yaml. When set, the runner
    # skips the test (pytest.skip) instead of executing it — used for metrics
    # blocked on an external fix (e.g. git metrics until bitbucket-cloud #1877).
    skip: str | None = None
    # Optional `identity_aliases: {canonical_email: [other, …]}`. Every listed
    # email binds to the CANONICAL email's person id, so a fixture can express
    # one human with several source accounts — the shape that makes a metric
    # double-count if gold does not collapse aliases. Without it every email is
    # its own person and no fixture can reach that path.
    identity_aliases: dict[str, list[str]] = field(default_factory=dict)
    # Optional `identity_accounts: [{source_type, source_id, account_id, person}]`.
    # Each entry is a source-account binding (`value_type='id'`) the rig writes
    # into identity_persons beside the synthetic email bindings — the shape the
    # account-first resolution map reads.
    identity_accounts: list[IdentityAccount] = field(default_factory=list)

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

    # Resolve `skip` BEFORE validating cases or resolving bronze schemas/data.
    # A skipped fixture (a metric blocked on an external fix) may legitimately
    # carry stale or invalid schemas/data, and must still skip cleanly instead
    # of failing pytest collection. Reject non-string `skip` at the boundary:
    # the contract is `str | None`, and e.g. `skip: false` would otherwise be
    # silently swallowed by the runner's truthiness check in conftest.
    skip = doc.get("skip")
    if skip is not None and not isinstance(skip, str):
        raise FixtureError(f"{path}: `skip` must be a string reason (got {type(skip).__name__})")
    if skip:
        return TestYaml(name=path.name[: -len(".test.yaml")], path=path, skip=skip)

    aliases_doc = doc.get("identity_aliases") or {}
    if not isinstance(aliases_doc, dict):
        raise FixtureError(f"{path}: `identity_aliases` must be a mapping of canonical email → aliases")
    identity_aliases: dict[str, list[str]] = {}
    for canonical, aliases in aliases_doc.items():
        if not isinstance(aliases, list) or not all(isinstance(a, str) for a in aliases):
            raise FixtureError(f"{path}: identity_aliases.{canonical} must be a list of emails")
        identity_aliases[str(canonical)] = list(aliases)

    accounts_doc = doc.get("identity_accounts") or []
    if not isinstance(accounts_doc, list):
        raise FixtureError(f"{path}: `identity_accounts` must be a list of bindings")
    identity_accounts: list[IdentityAccount] = []
    for idx, entry in enumerate(accounts_doc):
        if not isinstance(entry, dict) or set(entry) != {"source_type", "source_id", "account_id", "person"}:
            raise FixtureError(
                f"{path}: identity_accounts[{idx}] must be a mapping with exactly "
                "source_type, source_id, account_id, person"
            )
        if not all(isinstance(v, str) and v for v in entry.values()):
            raise FixtureError(f"{path}: identity_accounts[{idx}] values must be non-empty strings")
        identity_accounts.append(IdentityAccount(**entry))

    if "cases" not in doc:
        raise FixtureError(f"{path}: a test must define `cases`")

    bronze_doc = doc.get("bronze") or {}
    if not isinstance(bronze_doc, dict):
        raise FixtureError(f"{path}: `bronze` must be a mapping of table → records")
    bronze: dict[str, list[dict]] = {}
    schemas: dict[str, dict] = {}
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
        schemas[table] = schema

    cases = doc["cases"]
    if not isinstance(cases, list) or not cases:
        raise FixtureError(f"{path}: `cases` must be a non-empty list")
    for i, case in enumerate(cases):
        if not isinstance(case, dict) or "request" not in case or "expect" not in case:
            raise FixtureError(f"{path}: cases[{i}] must be a mapping with `request` and `expect`")

    return TestYaml(
        name=path.name[: -len(".test.yaml")],
        path=path,
        bronze=bronze,
        schemas=schemas,
        cases=cases,
        skip=skip,
        identity_aliases=identity_aliases,
        identity_accounts=identity_accounts,
    )


def _find_schemas_dir(test_path: Path) -> Path:
    """Walk up to the `metrics/` dir and use its `schemas/` subdir."""
    for parent in test_path.parents:
        if parent.name == "metrics":
            return parent / "schemas"
        if (parent / "schemas").is_dir():
            return parent / "schemas"
    raise FixtureError(f"{test_path}: could not locate a sibling `schemas/` directory")
