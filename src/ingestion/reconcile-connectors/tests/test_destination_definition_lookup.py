"""Which destination definition reconcile picks out of a workspace listing.

The id alone is not enough any more: the version Airbyte carries decides
whether the destination may be used at all, so both travel together.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

SELECTOR = Path(__file__).resolve().parents[1] / "python" / "select_destination_definition.py"

LISTING = {
    "destinationDefinitions": [
        {
            "destinationDefinitionId": "11111111-1111-1111-1111-111111111111",
            "name": "Postgres",
            "dockerImageTag": "2.0.0",
        },
        {
            "destinationDefinitionId": "22222222-2222-2222-2222-222222222222",
            "name": "ClickHouse",
            "dockerImageTag": "2.1.29",
        },
    ]
}


def _select(listing: object, name: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SELECTOR), name],
        input=json.dumps(listing),
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )


@pytest.mark.parametrize("name", ["ClickHouse", "Clickhouse", "clickhouse"])
def test_the_name_matches_whatever_case_airbyte_reports(name: str) -> None:
    """reconcile asks for `Clickhouse`; Airbyte has spelled it both ways."""
    done = _select(LISTING, name)

    assert done.returncode == 0, done.stderr
    assert done.stdout.rstrip("\n") == "22222222-2222-2222-2222-222222222222\t2.1.29"


def test_a_name_no_definition_carries_is_a_refusal_not_an_empty_answer() -> None:
    done = _select(LISTING, "Snowflake")

    assert done.returncode == 1
    assert done.stdout == ""


def test_an_entry_without_an_id_is_not_a_match() -> None:
    """A half-formed entry would otherwise be reported as the definition and
    create a destination against an empty id."""
    done = _select({"destinationDefinitions": [{"name": "ClickHouse", "dockerImageTag": "2.1.29"}]}, "Clickhouse")

    assert done.returncode == 1
    assert done.stdout == ""


def test_a_definition_reporting_no_tag_still_resolves_so_the_version_gate_sees_it() -> None:
    """An empty tag is refused by the version check, which says so; dropping
    the definition here would blame a missing connector instead."""
    done = _select({"destinationDefinitions": [{"destinationDefinitionId": "abc", "name": "ClickHouse"}]}, "Clickhouse")

    assert done.returncode == 0, done.stderr
    assert done.stdout.rstrip("\n") == "abc\t"


def test_a_listing_without_the_key_yields_no_definition() -> None:
    done = _select({}, "Clickhouse")

    assert done.returncode == 1


def test_a_call_without_a_name_is_a_usage_error() -> None:
    done = subprocess.run(
        [sys.executable, str(SELECTOR)], input="{}", capture_output=True, text=True, encoding="utf-8", check=False
    )

    assert done.returncode == 2
