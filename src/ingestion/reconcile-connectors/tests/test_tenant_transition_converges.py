"""A tenant transition cut off at any step is finished by a later pass.

The connection's name and the former tenant's schedules are derived from the
objects themselves on every pass, never from a record that the source rename
already consumed.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]

TENANT = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
FORMER = "example-legacy"
SOURCE_NAME = f"jira-main-{TENANT}"
CFG_HASH = "abcdefabcdef0123"
TAGS = [{"name": "insight"}, {"name": f"cfg-hash:{CFG_HASH[:12]}"}]

CURRENT_SCHEDULE = "jira-main-0f6b3c2e-sync"
FORMER_SCHEDULE = "jira-main-example-sync"
FORMER_FULL_TENANT_SCHEDULE = f"jira-{FORMER}-sync"
SIBLING_FORMER_SCHEDULE = "jira-main-eu-example-sync"

#: Airbyte and kubectl answered from files the test writes; every write is
#: recorded and applied, so a second pass sees what the first one left. A
#: name listed in FAIL fails; one listed in LOSE succeeds and then fails.
STUBS = r"""
ab_workspace_id()      { printf 'ws-1'; }
ab_list_connections()  { cat "$CONNECTIONS"; [[ -z "${LIST_FAILS:-}" ]]; }
reconcile_resolve_destination_id() { printf 'BOOTSTRAP_CONNECTION\n' >> "$CALLS"; return 1; }
ab_rename_connection() {
  printf 'RENAME_CONNECTION %s %s\n' "$1" "$2" >> "$CALLS"
  [[ " ${FAIL:-} " == *" $1 "* ]] && return 1
  python3 - "$CONNECTIONS" "$1" "$2" <<'PY'
import json, sys
path, ident, name = sys.argv[1:]
rows = json.load(open(path))
for row in rows:
    if row["connectionId"] == ident:
        row["name"] = name
json.dump(rows, open(path, "w"))
PY
  [[ " ${LOSE:-} " != *" $1 "* ]]
}
kubectl() {
  case " $* " in
    *" get "*)
      [[ -z "${GET_FAILS:-}" ]] || { echo 'connection refused'; return 1; }
      python3 - "$CRONS" <<'PY'
import json, sys
for name, tenant in json.load(open(sys.argv[1])):
    print(f"{name}\t{tenant}", end="\n")
PY
      ;;
    *" delete "*)
      local args="$*" name
      name="${args##*cronworkflow.argoproj.io/}"
      name="${name%% *}"
      printf 'DELETE %s\n' "${name}" >> "$CALLS"
      [[ " ${FAIL:-} " == *" ${name} "* ]] && return 1
      python3 - "$CRONS" "${name}" <<'PY'
import json, sys
path, gone = sys.argv[1:]
json.dump([c for c in json.load(open(path)) if c[0] != gone], open(path, "w"))
PY
      ;;
  esac
}
"""


@dataclass(frozen=True)
class Pass:
    code: int
    calls: list[str]


def run(tmp_path: Path, command: str, **env: str) -> Pass:
    calls = tmp_path / "calls"
    calls.unlink(missing_ok=True)
    exports = " ".join(f'{key}="{value}"' for key, value in env.items())
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight CONNECTORS_DIR="{ROOT}/../connectors" AIRBYTE_URL=http://127.0.0.1:1
    export CONNECTIONS="{tmp_path / "connections.json"}" CRONS="{tmp_path / "crons.json"}" CALLS="{calls}" {exports}
    source "{ROOT}/lib/reconcile.sh"
    {STUBS}
    {command}
    """
    result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, check=False)
    recorded = calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []
    return Pass(result.returncode, recorded)


def connections(tmp_path: Path, name: str) -> None:
    rows = [{"connectionId": "conn-1", "sourceId": "src-1", "name": name, "tags": TAGS}]
    (tmp_path / "connections.json").write_text(json.dumps(rows), encoding="utf-8")


def connection_name(tmp_path: Path) -> str:
    return json.loads((tmp_path / "connections.json").read_text(encoding="utf-8"))[0]["name"]


def reconcile_connection(tmp_path: Path, **env: str) -> Pass:
    return run(tmp_path, f"reconcile_connections jira src-1 {CFG_HASH} bronze_jira {SOURCE_NAME}", **env)


def test_a_connection_still_named_for_the_former_tenant_is_renamed_in_place(tmp_path: Path) -> None:
    connections(tmp_path, f"jira-main-{FORMER}-conn")

    result = reconcile_connection(tmp_path)

    assert result.code == 0
    assert result.calls == [f"RENAME_CONNECTION conn-1 {SOURCE_NAME}-conn"]


def test_a_connection_already_on_the_tenant_is_left_alone(tmp_path: Path) -> None:
    connections(tmp_path, f"{SOURCE_NAME}-conn")

    assert reconcile_connection(tmp_path) == Pass(0, [])


@pytest.mark.parametrize("failure", [{"FAIL": "conn-1"}, {"LOSE": "conn-1"}], ids=["fails", "reply lost"])
def test_a_connection_rename_cut_off_is_finished_by_the_next_pass(tmp_path: Path, failure: dict[str, str]) -> None:
    connections(tmp_path, f"jira-main-{FORMER}-conn")

    first = reconcile_connection(tmp_path, **failure)
    second = reconcile_connection(tmp_path)

    assert first.code != 0, "a failed rename is reported, not logged as done"
    assert second.code == 0
    assert connection_name(tmp_path) == f"{SOURCE_NAME}-conn"


@pytest.mark.parametrize(
    ("listing", "env"),
    [("[]", {"LIST_FAILS": "1"}), ("not json", {})],
    ids=["the request fails", "the payload is unreadable"],
)
def test_an_unread_connection_listing_stops_the_pass_without_writing(
    tmp_path: Path, listing: str, env: dict[str, str]
) -> None:
    (tmp_path / "connections.json").write_text(listing, encoding="utf-8")

    result = reconcile_connection(tmp_path, **env)

    assert result.code != 0, f"should refuse: {env or listing}"
    assert result.calls == [], "nothing is created or renamed from an unread listing"


def crons(tmp_path: Path, *named: tuple[str, str]) -> None:
    (tmp_path / "crons.json").write_text(json.dumps(list(named)), encoding="utf-8")


def remaining(tmp_path: Path) -> set[str]:
    return {name for name, _ in json.loads((tmp_path / "crons.json").read_text(encoding="utf-8"))}


def sweep(tmp_path: Path, **env: str) -> Pass:
    return run(tmp_path, f"argo_delete_former_tenant_cronworkflows jira {TENANT} main", **env)


def every_schedule(tmp_path: Path) -> None:
    crons(
        tmp_path,
        (CURRENT_SCHEDULE, TENANT),
        (FORMER_SCHEDULE, FORMER),
        (FORMER_FULL_TENANT_SCHEDULE, FORMER),
        (SIBLING_FORMER_SCHEDULE, FORMER),
        ("jira-hand-made-sync", FORMER),
    )


def test_only_this_instances_schedules_under_a_former_tenant_are_removed(tmp_path: Path) -> None:
    every_schedule(tmp_path)

    result = sweep(tmp_path)

    assert result.code == 0
    assert remaining(tmp_path) == {CURRENT_SCHEDULE, SIBLING_FORMER_SCHEDULE, "jira-hand-made-sync"}


def test_a_schedule_removal_that_failed_is_finished_by_the_next_pass(tmp_path: Path) -> None:
    every_schedule(tmp_path)

    first = sweep(tmp_path, FAIL=FORMER_SCHEDULE)
    second = sweep(tmp_path)

    assert first.code != 0
    assert FORMER_SCHEDULE not in remaining(tmp_path)
    assert second.code == 0


def test_an_unread_schedule_listing_removes_nothing(tmp_path: Path) -> None:
    every_schedule(tmp_path)

    result = sweep(tmp_path, GET_FAILS="1")

    assert result.code != 0
    assert result.calls == []
