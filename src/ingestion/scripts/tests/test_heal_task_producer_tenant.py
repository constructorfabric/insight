"""The task tenant heals place tenant_id after unique_key — on every task class,
and on every producer with the type its model emits — leave what ClickHouse says
is absent alone, and stop at a probe they cannot read.

The heal runs against a stub of `_ch_http_query` that answers from a scripted
catalogue and logs every statement, so no ClickHouse is needed.
"""

from __future__ import annotations

import re
import subprocess
from dataclasses import dataclass
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]

STRING = "String"
NULLABLE = "Nullable(String)"
PRODUCER_TYPES = {
    "jira__field_history_derived": STRING,
    "jira__comment_lifecycle_events": STRING,
    "jira__worklog_lifecycle_events": STRING,
    "jira__availability_events": NULLABLE,
    "jira__task_comments": NULLABLE,
    "jira__task_worklogs": NULLABLE,
    "jira__task_field_metadata": NULLABLE,
    "jira__task_links": NULLABLE,
    "youtrack__task_field_history": STRING,
    "youtrack__task_field_metadata": STRING,
    "youtrack__task_issuetypes": STRING,
    "youtrack__task_links": STRING,
    "youtrack__task_comments": NULLABLE,
    "youtrack__task_projects": NULLABLE,
    "youtrack__task_sprints": NULLABLE,
    "youtrack__task_statuses": NULLABLE,
    "youtrack__task_worklogs": NULLABLE,
    "github__task_field_history": NULLABLE,
    "github__task_links": NULLABLE,
}
CLASSES = [
    "class_task_comments",
    "class_task_field_history",
    "class_task_field_metadata",
    "class_task_issuetypes",
    "class_task_links",
    "class_task_projects",
    "class_task_sprints",
    "class_task_statuses",
    "class_task_worklogs",
]
PRODUCER_VIEWS = [
    "jira__task_issuetypes",
    "jira__task_projects",
    "jira__task_sprints",
    "jira__task_statuses",
    "github__task_field_metadata",
    "github__task_issuetypes",
    "github__task_projects",
    "github__task_statuses",
]

# Each catalogue entry is `<name> <state>`; a name not listed does not exist.
# The stub answers a probe the way ClickHouse would for that state, so a probe
# that forgets to exclude views counts them.
STUB = r"""
set -euo pipefail
source "$SCRIPTS/lib/ch-exec.sh"
source "$SCRIPTS/lib/heal-task-producer-tenant.sh"
_state() {
  local name state
  while read -r name state; do
    [[ "$name" == "$1" ]] && { printf '%s' "$state"; return; }
  done <<< "$CATALOGUE"
  printf 'absent'
}
_table_probe() {
  case "$2" in
    table) echo 1 ;;
    stale_view | fresh_view) [[ "$1" == *"engine != 'View'"* ]] && echo 0 || echo 1 ;;
    fail) return 1 ;;
    unreadable) echo "not a number" ;;
    *) echo 0 ;;
  esac
}
_view_probe() {
  local lacks_tenant=0
  [[ "$1" == *"NOT has("*"'tenant_id')"* ]] && lacks_tenant=1
  case "$2" in
    stale_view) echo "$lacks_tenant" ;;
    fresh_view) echo "$((1 - lacks_tenant))" ;;
    fail) return 1 ;;
    unreadable) echo "not a number" ;;
    *) echo 0 ;;
  esac
}
_ch_http_query() {
  local sql name state
  sql="$(cat)"
  printf '%s\n;;\n' "$sql" >> "$LOG"
  [[ "$sql" == SELECT* ]] || return 0
  name="$(sed -nE "s/.*name = '([a-z_]+)'.*/\1/p" <<< "$sql")"
  state="$(_state "$name")"
  if [[ "$sql" == *"engine = 'View'"* ]]; then _view_probe "$sql" "$state"; else _table_probe "$sql" "$state"; fi
}
"$ENTRY"
"""


@dataclass(frozen=True)
class Run:
    returncode: int
    statements: list[str]

    def altered(self) -> dict[str, list[str]]:
        changes: dict[str, list[str]] = {}
        for statement in self.statements:
            relation = re.search(r"(?:staging|silver)\.\w+", statement)
            if relation and statement.startswith(("ALTER", "DROP")):
                changes.setdefault(relation[0], []).append(statement)
        return changes


def _heal(tmp_path: Path, catalogue: dict[str, str], entry: str = "heal_task_producer_tenants") -> Run:
    log = tmp_path / "statements.log"
    log.touch()
    result = subprocess.run(
        ["bash", "-c", STUB],
        env={
            "PATH": "/usr/bin:/bin:/opt/homebrew/bin",
            "SCRIPTS": str(SCRIPTS),
            "LOG": str(log),
            "ENTRY": entry,
            "CATALOGUE": "\n".join(f"{name} {state}" for name, state in catalogue.items()),
            "CLICKHOUSE_URL": "http://clickhouse.invalid:8123",
            "CLICKHOUSE_USER": "test",
            "CLICKHOUSE_PASSWORD": "test",
        },
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    statements = [s.strip() for s in log.read_text().split("\n;;\n") if s.strip()]
    return Run(result.returncode, statements)


def test_every_producer_table_gains_the_tenant_after_unique_key_with_its_model_type(tmp_path: Path) -> None:
    run = _heal(tmp_path, dict.fromkeys(PRODUCER_TYPES, "table"))

    assert run.returncode == 0
    altered = run.altered()
    assert set(altered) == {f"staging.{table}" for table in PRODUCER_TYPES}
    for table, kind in PRODUCER_TYPES.items():
        assert altered[f"staging.{table}"] == [
            f"ALTER TABLE staging.{table} ADD COLUMN IF NOT EXISTS tenant_id {kind} AFTER unique_key",
            f"ALTER TABLE staging.{table} MODIFY COLUMN tenant_id {kind} AFTER unique_key",
        ], f"should add and place tenant_id as {kind}: {table}"


@pytest.mark.parametrize("state", ["absent", "fresh_view"])
def test_a_table_that_is_not_a_real_producer_table_is_left_alone(tmp_path: Path, state: str) -> None:
    catalogue = dict.fromkeys(PRODUCER_TYPES, "table")
    catalogue["jira__task_comments"] = state

    run = _heal(tmp_path, catalogue)

    assert run.returncode == 0
    altered = run.altered()
    assert "staging.jira__task_comments" not in altered, f"should skip a {state} table: {altered}"
    assert len(altered) == len(PRODUCER_TYPES) - 1, "every other producer is still healed"


def test_only_a_producer_view_without_the_tenant_is_dropped(tmp_path: Path) -> None:
    catalogue = dict.fromkeys(PRODUCER_VIEWS, "fresh_view")
    catalogue["jira__task_statuses"] = "stale_view"
    catalogue["github__task_projects"] = "stale_view"
    catalogue["jira__task_sprints"] = "table"

    run = _heal(tmp_path, catalogue)

    assert run.returncode == 0
    assert run.altered() == {
        "staging.jira__task_statuses": ["DROP VIEW IF EXISTS staging.jira__task_statuses"],
        "staging.github__task_projects": ["DROP VIEW IF EXISTS staging.github__task_projects"],
    }


@pytest.mark.parametrize("state", ["fail", "unreadable"])
@pytest.mark.parametrize("failing", ["youtrack__task_issuetypes", "github__task_links", "jira__task_projects"])
def test_a_probe_that_cannot_be_read_stops_the_heal_before_anything_after_it(
    tmp_path: Path, failing: str, state: str
) -> None:
    catalogue = dict.fromkeys(PRODUCER_TYPES, "table") | dict.fromkeys(PRODUCER_VIEWS, "stale_view")
    catalogue[failing] = state

    run = _heal(tmp_path, catalogue)

    assert run.returncode != 0, f"should fail on a {state} probe of {failing}"
    assert f"'{failing}'" in run.statements[-1], f"nothing is sent past the {state} probe of {failing}"
    assert f"staging.{failing}" not in run.altered()


def test_every_task_class_gains_a_nullable_tenant_after_unique_key(tmp_path: Path) -> None:
    run = _heal(tmp_path, dict.fromkeys(CLASSES, "table"), entry="heal_task_class_tenants")

    assert run.returncode == 0
    altered = run.altered()
    assert set(altered) == {f"silver.{name}" for name in CLASSES}
    for name in CLASSES:
        assert altered[f"silver.{name}"] == [
            f"ALTER TABLE silver.{name} ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key",
            f"ALTER TABLE silver.{name} MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key",
        ], f"should add and place tenant_id: {name}"


def test_a_class_no_sync_has_built_is_left_alone(tmp_path: Path) -> None:
    catalogue = dict.fromkeys(CLASSES, "table")
    catalogue["class_task_links"] = "absent"

    run = _heal(tmp_path, catalogue, entry="heal_task_class_tenants")

    assert run.returncode == 0
    altered = run.altered()
    assert "silver.class_task_links" not in altered, f"should skip an absent class: {altered}"
    assert len(altered) == len(CLASSES) - 1, "every other class is still healed"


@pytest.mark.parametrize("state", ["fail", "unreadable"])
@pytest.mark.parametrize("failing", ["class_task_comments", "class_task_statuses"])
def test_a_class_probe_that_cannot_be_read_stops_the_heal_before_anything_after_it(
    tmp_path: Path, failing: str, state: str
) -> None:
    catalogue = dict.fromkeys(CLASSES, "table")
    catalogue[failing] = state

    run = _heal(tmp_path, catalogue, entry="heal_task_class_tenants")

    assert run.returncode != 0, f"should fail on a {state} probe of {failing}"
    assert f"'{failing}'" in run.statements[-1], f"nothing is sent past the {state} probe of {failing}"
    assert f"silver.{failing}" not in run.altered()
