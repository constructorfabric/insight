"""The identity map heal drops a map view that predates its tenant column, keeps
one that has it and anything that is not a view, drops person_map before the
account_assignment it reads, and stops at a probe it cannot read.

The heal runs against a stub of `_ch_http_query` that answers from a scripted
catalogue and logs every statement, so no ClickHouse is needed.
"""

from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
MAPS = ("person_map", "account_assignment")

# Each catalogue entry is `<name> <state>`; a name not listed does not exist.
# The stub answers the probe the way ClickHouse would for that state, so a probe
# that forgets the view or the column condition counts the wrong relations.
STUB = r"""
set -euo pipefail
source "$SCRIPTS/lib/ch-exec.sh"
source "$SCRIPTS/lib/heal-identity-map-tenant.sh"
_state() {
  local name state
  while read -r name state; do
    [[ "$name" == "$1" ]] && { printf '%s' "$state"; return; }
  done <<< "$CATALOGUE"
  printf 'absent'
}
_ch_http_query() {
  local sql name lacks_tenant=0 is_view=0
  sql="$(cat)"
  printf '%s\n;;\n' "$sql" >> "$LOG"
  [[ "$sql" == SELECT* ]] || return 0
  name="$(sed -nE "s/.*t\.name = '([a-z_]+)'.*/\1/p" <<< "$sql")"
  [[ "$sql" == *"t.database = 'identity'"* ]] || { echo 0; return; }
  [[ "$sql" == *"NOT has("*"'tenant_id')"* ]] && lacks_tenant=1
  [[ "$sql" == *"t.engine = 'View'"* ]] && is_view=1
  case "$(_state "$name")" in
    stale_view) echo "$lacks_tenant" ;;
    fresh_view) echo "$((1 - lacks_tenant))" ;;
    stale_table) echo "$((1 - is_view))" ;;
    fail) return 1 ;;
    unreadable) echo "not a number" ;;
    *) echo 0 ;;
  esac
}
heal_identity_map_tenants
"""


@dataclass(frozen=True)
class Run:
    returncode: int
    statements: list[str]

    def dropped(self) -> list[str]:
        return [s for s in self.statements if s.startswith("DROP")]


def _heal(tmp_path: Path, catalogue: dict[str, str]) -> Run:
    log = tmp_path / "statements.log"
    log.touch()
    result = subprocess.run(
        ["bash", "-c", STUB],
        env={
            "PATH": "/usr/bin:/bin:/opt/homebrew/bin",
            "SCRIPTS": str(SCRIPTS),
            "LOG": str(log),
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


def test_maps_without_the_tenant_are_dropped_person_map_first(tmp_path: Path) -> None:
    run = _heal(tmp_path, dict.fromkeys(MAPS, "stale_view"))

    assert run.returncode == 0
    assert run.dropped() == [
        "DROP VIEW IF EXISTS identity.person_map",
        "DROP VIEW IF EXISTS identity.account_assignment",
    ]


@pytest.mark.parametrize("state", ["fresh_view", "absent", "stale_table"])
def test_a_map_that_is_not_a_view_without_the_tenant_is_left_alone(tmp_path: Path, state: str) -> None:
    catalogue = dict.fromkeys(MAPS, "stale_view")
    catalogue["person_map"] = state

    run = _heal(tmp_path, catalogue)

    assert run.returncode == 0
    assert run.dropped() == ["DROP VIEW IF EXISTS identity.account_assignment"], f"should keep a {state} person_map"


@pytest.mark.parametrize("state", ["fail", "unreadable"])
def test_a_probe_that_cannot_be_read_stops_the_heal_before_anything_after_it(tmp_path: Path, state: str) -> None:
    run = _heal(tmp_path, {"person_map": state, "account_assignment": "stale_view"})

    assert run.returncode != 0, f"should fail on a {state} probe"
    assert "'person_map'" in run.statements[-1], f"nothing is sent past the {state} probe"
    assert run.dropped() == []
