"""An instance created under the former tenant is renamed, not created again.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
FINDER = ROOT / "python" / "find_former_tenant_source.py"

DEFINITION = "def-jira"
TENANT = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
FORMER = "example-legacy"


def source(name: str, source_id: str, tenant: str, *, ident: str = "src-1", definition: str = DEFINITION) -> dict:
    return {
        "sourceId": ident,
        "name": name,
        "sourceDefinitionId": definition,
        "connectionConfiguration": {"insight_source_id": source_id, "insight_tenant_id": tenant, "token": "**********"},
    }


def find(sources: list[dict], source_id: str = "main") -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(FINDER), DEFINITION, "jira", source_id, TENANT],
        input=json.dumps(sources),
        capture_output=True,
        text=True,
        check=False,
    )


def test_the_source_under_the_former_tenant_is_found_with_its_tenant() -> None:
    result = find([source(f"jira-main-{FORMER}", "main", FORMER)])

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip().split("\t") == ["src-1", FORMER, f"jira-main-{FORMER}"]


@pytest.mark.parametrize(
    "sources",
    [
        [],
        [source(f"jira-main-{TENANT}", "main", TENANT)],
        [source(f"jira-main-eu-{FORMER}", "main-eu", FORMER)],
        [source(f"jira-main-{FORMER}", "main", FORMER, definition="def-other")],
        [source("jira-renamed-by-hand", "main", FORMER)],
    ],
    ids=[
        "nothing installed",
        "already on the tenant",
        "a sibling instance",
        "another connector",
        "a name it did not give",
    ],
)
def test_nothing_is_taken_that_is_not_this_instance_under_its_former_tenant(sources: list[dict]) -> None:
    result = find(sources)

    assert result.returncode == 0, result.stderr
    assert result.stdout == "", f"should find nothing in {sources}"


def test_a_sibling_whose_id_extends_this_one_is_told_apart_by_its_configuration() -> None:
    sibling = source(f"jira-main-eu-{FORMER}", "main-eu", FORMER, ident="src-2")
    mine = source(f"jira-main-{FORMER}", "main", FORMER)

    result = find([sibling, mine])

    assert result.stdout.split("\t")[0] == "src-1", result.stdout


@pytest.mark.parametrize(
    ("sources", "reason"),
    [
        (
            [source(f"jira-main-{FORMER}", "main", FORMER), source("jira-main-older", "main", "older", ident="src-2")],
            "several former sources",
        ),
        (
            [
                source(f"jira-main-{FORMER}", "main", FORMER),
                source(f"jira-main-{TENANT}", "main", TENANT, ident="src-2"),
            ],
            "keep one by hand",
        ),
    ],
    ids=["two former sources", "the new name is taken"],
)
def test_a_rename_that_would_have_to_choose_is_refused(sources: list[dict], reason: str) -> None:
    result = find(sources)

    assert result.returncode == 1
    assert reason in result.stderr, result.stderr
    assert result.stdout == ""


#: Airbyte answered from files the test writes; every write is recorded. An
#: update whose reply is lost has still renamed the source, as Airbyte would.
RENAME_STUBS = """
ab_workspace_id()     { printf 'ws-1'; }
ab_list_sources()     { cat "$SOURCES"; }
ab_update_source()    {
  printf 'UPDATE_SOURCE %s %s\\n' "$1" "$3" >> "$CALLS"
  [[ -z "${LOSE_REPLY:-}" ]] && return 0
  python3 - "$SOURCES" "$3" "$2" <<'PY'
import json, sys
path, name, config = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
sources = json.load(open(path))
sources[0]["name"] = name
sources[0]["connectionConfiguration"].update(config)
json.dump(sources, open(path, "w"))
PY
  return 1
}
"""


def rename(
    tmp_path: Path, sources: list[dict] | None = None, *, lose_reply: bool = False
) -> tuple[int, str, list[str]]:
    if sources is not None:
        (tmp_path / "sources.json").write_text(json.dumps(sources), encoding="utf-8")
    calls = tmp_path / "calls"
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight CONNECTORS_DIR="{ROOT}/../connectors" AIRBYTE_URL=http://127.0.0.1:1
    export SOURCES="{tmp_path / "sources.json"}" CALLS="{calls}" LOSE_REPLY="{"1" if lose_reply else ""}"
    source "{ROOT}/lib/reconcile.sh"
    {RENAME_STUBS}
    reconcile_rename_former_tenant jira {DEFINITION} main {TENANT} jira-main-{TENANT} '{{"insight_tenant_id": "{TENANT}"}}'
    """
    result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, check=False)
    recorded = calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []
    return result.returncode, result.stdout, recorded


def test_reconcile_renames_the_source_in_one_update(tmp_path: Path) -> None:
    code, _, calls = rename(tmp_path, [source(f"jira-main-{FORMER}", "main", FORMER)])

    assert code == 0
    assert calls == [f"UPDATE_SOURCE src-1 jira-main-{TENANT}"]


def test_a_rename_whose_reply_was_lost_is_not_repeated(tmp_path: Path) -> None:
    first, _, _ = rename(tmp_path, [source(f"jira-main-{FORMER}", "main", FORMER)], lose_reply=True)
    (tmp_path / "calls").unlink()

    second, _, calls = rename(tmp_path)

    assert first != 0, "the lost reply is reported"
    assert (second, calls) == (0, []), "the next pass sees the source already renamed"


def test_reconcile_writes_nothing_when_it_would_have_to_choose(tmp_path: Path) -> None:
    taken = [
        source(f"jira-main-{FORMER}", "main", FORMER),
        source(f"jira-main-{TENANT}", "main", TENANT, ident="src-2"),
    ]

    code, _, calls = rename(tmp_path, taken)

    assert code != 0
    assert calls == []


def test_reconcile_writes_nothing_for_an_instance_already_on_the_tenant(tmp_path: Path) -> None:
    code, stdout, calls = rename(tmp_path, [source(f"jira-main-{TENANT}", "main", TENANT)])

    assert (code, stdout, calls) == (0, "", [])
