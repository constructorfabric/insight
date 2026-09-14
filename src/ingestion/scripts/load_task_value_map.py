"""Load operator-authored `config.task_value_map` rows from a TSV bundle.

The bundle is a per-environment directory (gitops environments/<env>/clickhouse/
task-value-map/) packed into a ConfigMap and mounted by the
clickhouse-task-value-map Hook Job.

INSERT-ONLY. A key that already carries a decision is left exactly as it is.
`config.task_value_map` is a ReplacingMergeTree whose `unique_key` includes
`valid_from` and `recorded_at`, so a plain re-INSERT would not collide with the
stored row — it would become a NEWER decision and win in every consumer's
`ORDER BY valid_from DESC, recorded_at DESC LIMIT 1 BY` read. The bundle is a
seed for keys nobody has decided yet, so every row is anti-joined against the
stored keys and only misses are written. The anti-join deliberately matches ANY
stored row for the key, including `is_deleted = 1`: a retraction is a decision
too, and re-seeding it would resurrect a mapping an operator removed.

File format — TSV, one decision per line, `#` comments and blank lines ignored,
optional header line:

    tenant_id  insight_source_id  data_source  field_id  value_id  canonical_value  [value_display]  [note]

The operator states business columns only. `valid_from` / `recorded_at` /
`recorded_by` are the loader's, which is what makes a hand-written file unable
to supersede a stored decision. `valid_from` is epoch: a seeded mapping is the
baseline and applies to all history; a later correction is an operator INSERT
with a real `valid_from`, and this loader never touches it.

Required env (set by the Hook Job):
    CLICKHOUSE_URL       e.g. http://ch-host:8123
    CLICKHOUSE_USER, CLICKHOUSE_PASSWORD
Options:
    TASK_VALUE_MAP_DIR          directory of *.tsv (default /config/task-value-map)
    TASK_VALUE_MAP_RECORDED_BY  `recorded_by` stamp (default gitops)
"""

from __future__ import annotations

import csv
import io
import os
import urllib.request
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path

import insight_logging

LOG = insight_logging.configure("task-value-map")

TABLE = "config.task_value_map"
DEFAULT_DIR = "/config/task-value-map"
DEFAULT_RECORDED_BY = "gitops"

KEY_COLUMNS = ("tenant_id", "insight_source_id", "data_source", "field_id", "value_id")
VALUE_COLUMNS = ("canonical_value", "value_display", "note")
MIN_COLUMNS = 6
MAX_COLUMNS = 8

ISSUE_KINDS = frozenset({"bug", "other", "unknown"})
STATUS_CATEGORIES = frozenset({"new", "in_progress", "done", "undefined"})
CANONICAL_VALUES = ISSUE_KINDS | STATUS_CATEGORIES

Key = tuple[str, str, str, str, str]


class MappingFileError(Exception):
    """One or more bundle lines are malformed; nothing is written."""


@dataclass(frozen=True)
class Mapping:
    """One operator decision, business columns only."""

    tenant_id: str
    insight_source_id: str
    data_source: str
    field_id: str
    value_id: str
    canonical_value: str
    value_display: str = ""
    note: str = ""

    @property
    def key(self) -> Key:
        return (self.tenant_id, self.insight_source_id, self.data_source, self.field_id, self.value_id)


def parse_bundle(files: Iterable[tuple[str, str]]) -> list[Mapping]:
    """Parse `(name, text)` pairs, reporting every bad line as `file:line`.

    Rejecting rows before they reach ClickHouse makes a typo name its own file
    and line instead of surfacing as a type error mid-INSERT.
    """
    mappings: list[Mapping] = []
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

            problem = _rejection(fields)
            if problem:
                errors.append(f"{name}:{reader.line_num}: {problem}")
                continue
            padded = (*fields, "", "")[:MAX_COLUMNS]
            mappings.append(Mapping(*padded))

    if errors:
        raise MappingFileError("\n".join(errors))
    return mappings


def _rejection(fields: Sequence[str]) -> str | None:
    if not MIN_COLUMNS <= len(fields) <= MAX_COLUMNS:
        return f"expected {MIN_COLUMNS}-{MAX_COLUMNS} tab-separated columns, got {len(fields)}"
    if any(value == "" for value in fields[:MIN_COLUMNS]):
        return f"the first {MIN_COLUMNS} columns are all required"

    # A typo passes every later gate silently: the consumer models filter on the
    # canonical domain, so an unknown spelling reads as an absent decision.
    field_id, canonical_value = fields[3], fields[5]
    permitted = ISSUE_KINDS if field_id == "type" else CANONICAL_VALUES
    if canonical_value not in permitted:
        return f"canonical_value {canonical_value!r} is not one of {', '.join(sorted(permitted))}"
    return None


def read_bundle(directory: Path) -> list[Mapping]:
    files = [(path.name, path.read_text(encoding="utf-8")) for path in sorted(directory.glob("*.tsv"))]
    LOG.info("bundle read", extra={"files": len(files), "dir": str(directory)})
    return parse_bundle(files)


def deduplicate(mappings: Iterable[Mapping]) -> list[Mapping]:
    """Collapse a key declared twice across the bundle; the first wins."""
    seen: set[Key] = set()
    unique: list[Mapping] = []
    for mapping in mappings:
        if mapping.key in seen:
            continue
        seen.add(mapping.key)
        unique.append(mapping)
    return unique


def select_missing(mappings: Iterable[Mapping], stored: set[Key]) -> list[Mapping]:
    """The anti-join: rows whose key carries no stored decision at all."""
    return [mapping for mapping in deduplicate(mappings) if mapping.key not in stored]


def insert_statement(mappings: Sequence[Mapping], *, recorded_by: str) -> str:
    """One INSERT streaming the business columns through `input()`.

    The loader-owned columns are computed server-side, so `valid_from` is epoch
    and `recorded_at` is the server's clock rather than this pod's. Field text
    goes out verbatim, so ClickHouse's TSV escapes (`\\t`, `\\n`, `\\\\`) mean in
    the wire payload what they mean in the file.
    """
    rows = "\n".join(
        "\t".join((*mapping.key, mapping.canonical_value, mapping.value_display, mapping.note)) for mapping in mappings
    )
    structure = ", ".join(f"{column} String" for column in (*KEY_COLUMNS, *VALUE_COLUMNS))
    return (
        f"INSERT INTO {TABLE} "
        f"({', '.join(KEY_COLUMNS)}, valid_from, recorded_at, {', '.join(VALUE_COLUMNS)}, recorded_by) "
        f"SELECT {', '.join(KEY_COLUMNS)}, toDateTime64(0, 3), now64(3), "
        f"{', '.join(VALUE_COLUMNS)}, {_lit(recorded_by)} "
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
    """Insert the declared keys that hold no stored decision. Returns the count."""
    if not _table_exists(fetch_rows):
        LOG.info("table absent — skipping", extra={"table": TABLE})
        return 0

    stored = _stored_keys(fetch_rows)
    missing = select_missing(mappings, stored)
    LOG.info("keys without a stored decision", extra={"declared": len(mappings), "missing": len(missing)})
    if not missing:
        return 0

    execute(insert_statement(missing, recorded_by=recorded_by))
    return len(missing)


def _table_exists(fetch_rows: Callable[[str], Sequence[Sequence[str]]]) -> bool:
    database, _, table = TABLE.partition(".")
    rows = fetch_rows(f"SELECT count() FROM system.tables WHERE database = '{database}' AND name = '{table}'")
    return bool(rows) and rows[0][0] == "1"


def _stored_keys(fetch_rows: Callable[[str], Sequence[Sequence[str]]]) -> set[Key]:
    # Every stored row counts, retracted (is_deleted = 1) and future-dated
    # alike: a decision exists for the key either way.
    rows = fetch_rows(f"SELECT DISTINCT {', '.join(KEY_COLUMNS)} FROM {TABLE}")
    return {tuple(row[: len(KEY_COLUMNS)]) for row in rows}  # type: ignore[misc]


def _http_client() -> tuple[Callable[[str], None], Callable[[str], Sequence[Sequence[str]]]]:
    """Executors over the ClickHouse HTTP interface, mirroring lib/ch-exec.sh.

    ClickHouse is always external to the release, so HTTP is the only path. The
    password travels in a header (not argv) exactly as ch-exec.sh does.
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

    def _post(sql: str) -> str:
        request = urllib.request.Request(  # noqa: S310 — fixed http(s) endpoint from config
            endpoint,
            data=sql.encode("utf-8"),
            headers={"X-ClickHouse-User": user, "X-ClickHouse-Key": password},
            method="POST",
        )
        with urllib.request.urlopen(request) as response:  # noqa: S310
            return response.read().decode("utf-8")

    def execute(sql: str) -> None:
        _post(sql)

    def fetch_rows(sql: str) -> list[list[str]]:
        return [line.split("\t") for line in _post(sql).splitlines() if line]

    return execute, fetch_rows


def main() -> int:
    directory = Path(os.environ.get("TASK_VALUE_MAP_DIR") or DEFAULT_DIR)
    recorded_by = os.environ.get("TASK_VALUE_MAP_RECORDED_BY") or DEFAULT_RECORDED_BY

    if not directory.is_dir():
        LOG.info("bundle directory absent — nothing to load", extra={"dir": str(directory)})
        return 0

    try:
        mappings = read_bundle(directory)
    except MappingFileError as error:
        LOG.error("rejected bundle", extra={"problems": str(error)})
        return 1

    if not mappings:
        LOG.info("no data rows — nothing to load")
        return 0

    execute, fetch_rows = _http_client()
    inserted = load(mappings, recorded_by=recorded_by, execute=execute, fetch_rows=fetch_rows)

    LOG.info("task value mappings loaded", extra={"inserted": inserted})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
