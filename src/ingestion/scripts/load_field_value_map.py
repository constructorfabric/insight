"""Load operator-authored `config.field_value_map` rows from a TSV bundle.

The bundle is a per-environment directory (gitops environments/<env>/clickhouse/
field-value-map/) packed into a ConfigMap and mounted by the
clickhouse-field-value-map Hook Job.

Every `*.tsv` except `defaults.tsv` carries mapping decisions and lands in
`config.field_value_map`; the file mirrors the table, so a decision states which
`field` it standardizes. The optional `defaults.tsv` carries per-(tenant,
source, field) fallbacks for unmapped keys and lands in
`config.field_value_defaults`.

INSERT-ONLY. A key that already carries a decision is left exactly as it is.
Both tables are ReplacingMergeTrees whose `unique_key` includes `valid_from` and
`recorded_at`, so a plain re-INSERT would not collide with the stored row — it
would become a NEWER decision and win in every consumer's
`ORDER BY valid_from DESC, recorded_at DESC LIMIT 1 BY` read. The bundle is a
seed for keys nobody has decided yet, so every row is anti-joined against the
stored keys and only misses are written. The anti-join deliberately matches ANY
stored row for the key, including `is_deleted = 1`: a retraction is a decision
too, and re-seeding it would resurrect a value an operator removed.

Mapping file format — TSV, one decision per line, `#` comments and blank lines
ignored, optional header line:

    tenant_id  insight_source_id  data_source  field  source_key  display_name  target_value  [note]

`field` names what is standardized (e.g. `issue_type`); `target_value` must lie
in that field's canonical domain. `display_name` is required: it records the
name the decision was made against, which is what later makes a rename of the
vendor value detectable.

`defaults.tsv` format — same conventions, all columns required:

    tenant_id  insight_source_id  field  default_value

The operator states business columns only. `valid_from` / `recorded_at` /
`recorded_by` are the loader's, which is what makes a hand-written file unable
to supersede a stored decision. `valid_from` is epoch: a seeded decision is the
baseline and applies to all history; a later correction is an operator INSERT
with a real `valid_from`, and this loader never touches it.

Required env (set by the Hook Job):
    CLICKHOUSE_URL       e.g. http://ch-host:8123
    CLICKHOUSE_USER, CLICKHOUSE_PASSWORD
Options:
    FIELD_VALUE_MAP_DIR          directory of *.tsv (default /config/field-value-map)
    FIELD_VALUE_MAP_RECORDED_BY  `recorded_by` stamp (default gitops)
    FIELD_VALUE_MAP_TIMEOUT      per-request timeout, seconds (default 30)
"""

from __future__ import annotations

import csv
import io
import os
import urllib.error
import urllib.request
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import insight_logging

LOG = insight_logging.configure("field-value-map")

MAP_TABLE = "config.field_value_map"
DEFAULTS_TABLE = "config.field_value_defaults"
DEFAULTS_FILE = "defaults.tsv"
DEFAULT_DIR = "/config/field-value-map"
DEFAULT_RECORDED_BY = "gitops"
DEFAULT_TIMEOUT = 30.0

# INVARIANT: the bundle parser accepts a quoted field holding a tab or a newline,
# so every field must carry ClickHouse's documented TabSeparated escapes before
# it is joined — a raw join turns one such value into two columns or two rows.
TSV_ESCAPES = str.maketrans(
    {"\\": "\\\\", "\b": "\\b", "\f": "\\f", "\r": "\\r", "\n": "\\n", "\t": "\\t", "\0": "\\0", "'": "\\'"}
)

KEY_COLUMNS = ("tenant_id", "insight_source_id", "data_source", "field", "source_key")
VALUE_COLUMNS = ("display_name", "target_value", "note")
MIN_COLUMNS = 7
MAX_COLUMNS = 8
DEFAULTS_COLUMNS = ("tenant_id", "insight_source_id", "field", "default_value")

# Canonical domain per standardized field. A row for a field outside this set is
# rejected: no consumer resolves it yet, so it could only mislead.
FIELD_DOMAINS = {"issue_type": frozenset({"bug", "task", "unknown"})}

Key = tuple[str, str, str, str, str]
DefaultKey = tuple[str, str, str]


class MappingFileError(Exception):
    """One or more bundle lines are malformed; nothing is written."""


@dataclass(frozen=True)
class Mapping:
    """One operator mapping decision, business columns only."""

    tenant_id: str
    insight_source_id: str
    data_source: str
    field: str
    source_key: str
    display_name: str
    target_value: str
    note: str = ""

    @property
    def key(self) -> Key:
        return (self.tenant_id, self.insight_source_id, self.data_source, self.field, self.source_key)


@dataclass(frozen=True)
class Default:
    """One operator fallback: what an unmapped key of `field` resolves to."""

    tenant_id: str
    insight_source_id: str
    field: str
    default_value: str

    @property
    def key(self) -> DefaultKey:
        return (self.tenant_id, self.insight_source_id, self.field)


class Keyed(Protocol):
    @property
    def key(self) -> tuple[str, ...]: ...


def _parse_files[RowT: Keyed](
    files: Iterable[tuple[str, str]],
    *,
    rejection: Callable[[Sequence[str]], str | None],
    build: Callable[[Sequence[str]], RowT],
) -> list[RowT]:
    """Parse `(name, text)` pairs, reporting every bad line as `file:line`.

    Rejecting rows before they reach ClickHouse makes a typo name its own file
    and line instead of surfacing as a type error mid-INSERT.
    """
    rows_out: list[RowT] = []
    errors: list[str] = []

    for name, text in files:
        header_allowed = True
        reader = csv.reader(io.StringIO(text), delimiter="\t", quotechar='"')

        for row in reader:
            fields = [field.strip() for field in row]

            if not any(fields):
                continue

            if fields[0].startswith("#"):
                continue

            if header_allowed:
                header_allowed = False

                if fields[0].lower() == "tenant_id":
                    continue

            problem = rejection(fields)
            if problem:
                errors.append(f"{name}:{reader.line_num}: {problem}")
                continue
            rows_out.append(build(fields))

    if errors:
        raise MappingFileError("\n".join(errors))
    return rows_out


def parse_bundle(files: Iterable[tuple[str, str]]) -> list[Mapping]:
    return _parse_files(files, rejection=_rejection, build=lambda fields: Mapping(*(*fields, "")[:MAX_COLUMNS]))


def parse_defaults(files: Iterable[tuple[str, str]]) -> list[Default]:
    return _parse_files(files, rejection=_defaults_rejection, build=lambda fields: Default(*fields))


def _rejection(fields: Sequence[str]) -> str | None:
    if not MIN_COLUMNS <= len(fields) <= MAX_COLUMNS:
        return f"expected {MIN_COLUMNS}-{MAX_COLUMNS} tab-separated columns, got {len(fields)}"
    if any(value == "" for value in fields[:MIN_COLUMNS]):
        return f"the first {MIN_COLUMNS} columns are all required"

    # A typo passes every later gate silently: the consumer models filter on the
    # canonical domain, so an unknown spelling reads as an absent decision.
    return _domain_problem(field=fields[3], value=fields[6], value_column="target_value")


def _defaults_rejection(fields: Sequence[str]) -> str | None:
    if len(fields) != len(DEFAULTS_COLUMNS):
        return f"expected {len(DEFAULTS_COLUMNS)} tab-separated columns, got {len(fields)}"
    if any(value == "" for value in fields):
        return f"all {len(DEFAULTS_COLUMNS)} columns are required"

    return _domain_problem(field=fields[2], value=fields[3], value_column="default_value")


def _domain_problem(*, field: str, value: str, value_column: str) -> str | None:
    domain = FIELD_DOMAINS.get(field)
    if domain is None:
        return f"field {field!r} is not supported; supported fields: {', '.join(sorted(FIELD_DOMAINS))}"
    if value not in domain:
        return f"{value_column} {value!r} for field {field!r} is not one of {', '.join(sorted(domain))}"
    return None


def read_bundle(directory: Path) -> tuple[list[Mapping], list[Default]]:
    """Split the bundle: `defaults.tsv` feeds the defaults table, the rest the map."""
    paths = sorted(directory.glob("*.tsv"))
    map_files = [(path.name, path.read_text(encoding="utf-8")) for path in paths if path.name != DEFAULTS_FILE]
    defaults_files = [(path.name, path.read_text(encoding="utf-8")) for path in paths if path.name == DEFAULTS_FILE]
    LOG.info("bundle read", extra={"files": len(paths), "dir": str(directory)})

    errors: list[str] = []
    mappings: list[Mapping] = []
    defaults: list[Default] = []
    try:
        mappings = parse_bundle(map_files)
    except MappingFileError as error:
        errors.append(str(error))
    try:
        defaults = parse_defaults(defaults_files)
    except MappingFileError as error:
        errors.append(str(error))

    if errors:
        raise MappingFileError("\n".join(errors))
    return mappings, defaults


def deduplicate[RowT: Keyed](rows: Iterable[RowT]) -> list[RowT]:
    """Collapse a key declared twice across the bundle; the first wins."""
    seen: set[tuple[str, ...]] = set()
    unique: list[RowT] = []
    for row in rows:
        if row.key in seen:
            continue
        seen.add(row.key)
        unique.append(row)
    return unique


def select_missing[RowT: Keyed](rows: Iterable[RowT], stored: set[tuple[str, ...]]) -> list[RowT]:
    """The anti-join: rows whose key carries no stored decision at all."""
    return [row for row in deduplicate(rows) if row.key not in stored]


def insert_statement(mappings: Sequence[Mapping], *, recorded_by: str) -> str:
    """One INSERT streaming the business columns through `input()`.

    The loader-owned columns are computed server-side, so `valid_from` is epoch
    and `recorded_at` is the server's clock rather than this pod's. Every field
    is TSV-escaped, so a value that legitimately contains a tab, a newline or a
    backslash reaches ClickHouse as one field.
    """
    rows = "\n".join(
        "\t".join(
            field.translate(TSV_ESCAPES)
            for field in (*mapping.key, mapping.display_name, mapping.target_value, mapping.note)
        )
        for mapping in mappings
    )
    structure = ", ".join(f"{column} String" for column in (*KEY_COLUMNS, *VALUE_COLUMNS))
    return (
        f"INSERT INTO {MAP_TABLE} "
        f"(tenant_id, insight_source_id, data_source, field, source_key, valid_from, recorded_at, "
        f"target_value, display_name, note, recorded_by) "
        f"SELECT tenant_id, insight_source_id, data_source, field, source_key, "
        f"toDateTime64(0, 3), now64(3), target_value, display_name, note, {_lit(recorded_by)} "
        f"FROM input('{structure}') FORMAT TSV\n{rows}\n"
    )


def defaults_insert_statement(defaults: Sequence[Default], *, recorded_by: str) -> str:
    rows = "\n".join(
        "\t".join(field.translate(TSV_ESCAPES) for field in (*default.key, default.default_value))
        for default in defaults
    )
    structure = ", ".join(f"{column} String" for column in DEFAULTS_COLUMNS)
    return (
        f"INSERT INTO {DEFAULTS_TABLE} "
        f"(tenant_id, insight_source_id, field, valid_from, recorded_at, default_value, recorded_by) "
        f"SELECT tenant_id, insight_source_id, field, toDateTime64(0, 3), now64(3), "
        f"default_value, {_lit(recorded_by)} "
        f"FROM input('{structure}') FORMAT TSV\n{rows}\n"
    )


def _lit(value: str) -> str:
    return "'" + value.replace("\\", "\\\\").replace("'", "\\'") + "'"


def load(
    mappings: Sequence[Mapping],
    *,
    recorded_by: str,
    execute: Callable[[str], None],
    fetch_rows: Callable[[str], Sequence[Sequence[str]]],
) -> int:
    """Insert the declared map keys that hold no stored decision. Returns the count."""
    stored_sql = f"SELECT DISTINCT {', '.join(KEY_COLUMNS)} FROM {MAP_TABLE}"
    return _load(
        mappings,
        table=MAP_TABLE,
        stored_sql=stored_sql,
        statement=lambda missing: insert_statement(missing, recorded_by=recorded_by),
        execute=execute,
        fetch_rows=fetch_rows,
    )


def load_defaults(
    defaults: Sequence[Default],
    *,
    recorded_by: str,
    execute: Callable[[str], None],
    fetch_rows: Callable[[str], Sequence[Sequence[str]]],
) -> int:
    """Insert the declared default keys that hold no stored decision. Returns the count."""
    stored_sql = f"SELECT DISTINCT {', '.join(DEFAULTS_COLUMNS[:3])} FROM {DEFAULTS_TABLE}"
    return _load(
        defaults,
        table=DEFAULTS_TABLE,
        stored_sql=stored_sql,
        statement=lambda missing: defaults_insert_statement(missing, recorded_by=recorded_by),
        execute=execute,
        fetch_rows=fetch_rows,
    )


def _load[RowT: Keyed](
    rows: Sequence[RowT],
    *,
    table: str,
    stored_sql: str,
    statement: Callable[[Sequence[RowT]], str],
    execute: Callable[[str], None],
    fetch_rows: Callable[[str], Sequence[Sequence[str]]],
) -> int:
    if not rows:
        return 0

    if not _table_exists(fetch_rows, table):
        LOG.info("table absent — skipping", extra={"table": table})
        return 0

    # Every stored row counts, retracted (is_deleted = 1) and future-dated
    # alike: a decision exists for the key either way.
    stored = {tuple(row) for row in fetch_rows(stored_sql)}
    missing = select_missing(rows, stored)
    LOG.info("keys without a stored decision", extra={"table": table, "declared": len(rows), "missing": len(missing)})
    if not missing:
        return 0

    execute(statement(missing))
    return len(missing)


def _table_exists(fetch_rows: Callable[[str], Sequence[Sequence[str]]], table: str) -> bool:
    database, _, name = table.partition(".")
    rows = fetch_rows(f"SELECT count() FROM system.tables WHERE database = '{database}' AND name = '{name}'")
    return bool(rows) and rows[0][0] == "1"


def _http_client(
    *, urlopen: Callable[..., object] = urllib.request.urlopen
) -> tuple[Callable[[str], None], Callable[[str], Sequence[Sequence[str]]]]:
    """Executors over the ClickHouse HTTP interface, mirroring lib/ch-exec.sh.

    ClickHouse is always external to the release, so HTTP is the only path. The
    password travels in a header (not argv) exactly as ch-exec.sh does.

    Every request carries a finite timeout: this runs as a post-upgrade hook, and
    a stalled endpoint would otherwise hold the release open until Helm's own
    (much longer) timeout with nothing in the log to explain it.
    """
    url = os.environ.get("CLICKHOUSE_URL")
    user = os.environ.get("CLICKHOUSE_USER")
    password = os.environ.get("CLICKHOUSE_PASSWORD")
    missing = [
        name
        for name, value in (("CLICKHOUSE_URL", url), ("CLICKHOUSE_USER", user), ("CLICKHOUSE_PASSWORD", password))
        if not value
    ]
    if missing:
        raise SystemExit(f"{', '.join(missing)} must be set")

    endpoint = url.rstrip("/") + "/"
    timeout = _timeout()

    def _post(sql: str) -> str:
        request = urllib.request.Request(  # noqa: S310 — fixed http(s) endpoint from config
            endpoint,
            data=sql.encode("utf-8"),
            headers={"X-ClickHouse-User": user, "X-ClickHouse-Key": password},
            method="POST",
        )
        try:
            with urlopen(request, timeout=timeout) as response:  # type: ignore[attr-defined]  # noqa: S310
                return response.read().decode("utf-8")
        except TimeoutError as error:
            raise SystemExit(f"ClickHouse did not answer within {timeout}s ({endpoint})") from error
        except urllib.error.URLError as error:
            # WORKAROUND: urllib wraps a connect timeout instead of raising it.
            if isinstance(error.reason, TimeoutError):
                raise SystemExit(f"ClickHouse did not answer within {timeout}s ({endpoint})") from error
            raise

    def execute(sql: str) -> None:
        _post(sql)

    def fetch_rows(sql: str) -> list[list[str]]:
        return [line.split("\t") for line in _post(sql).splitlines() if line]

    return execute, fetch_rows


def _timeout() -> float:
    raw = os.environ.get("FIELD_VALUE_MAP_TIMEOUT")
    if not raw:
        return DEFAULT_TIMEOUT
    try:
        value = float(raw)
    except ValueError:
        raise SystemExit(f"FIELD_VALUE_MAP_TIMEOUT must be a number of seconds, got {raw!r}") from None
    if value <= 0:
        raise SystemExit(f"FIELD_VALUE_MAP_TIMEOUT must be positive, got {raw!r}")
    return value


def main() -> int:
    directory = Path(os.environ.get("FIELD_VALUE_MAP_DIR") or DEFAULT_DIR)
    recorded_by = os.environ.get("FIELD_VALUE_MAP_RECORDED_BY") or DEFAULT_RECORDED_BY

    # WARNING, not INFO: the Job runs only when the feature is enabled, so an
    # absent mount means the ConfigMap never reached the pod.
    if not directory.is_dir():
        LOG.warning("bundle directory absent — nothing to load", extra={"dir": str(directory)})
        return 0

    try:
        mappings, defaults = read_bundle(directory)
    except MappingFileError as error:
        LOG.error("rejected bundle", extra={"problems": str(error)})
        return 1

    if not mappings and not defaults:
        LOG.warning("no data rows — nothing to load", extra={"dir": str(directory)})
        return 0

    execute, fetch_rows = _http_client()
    inserted = load(mappings, recorded_by=recorded_by, execute=execute, fetch_rows=fetch_rows)
    defaults_inserted = load_defaults(defaults, recorded_by=recorded_by, execute=execute, fetch_rows=fetch_rows)

    LOG.info("field value config loaded", extra={"map": inserted, "defaults": defaults_inserted})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
