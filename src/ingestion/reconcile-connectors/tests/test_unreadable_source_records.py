"""A source record nobody can read is left alone rather than acted on.

Both readers of the `sources/list` payload emit TSV their caller turns straight
into deletions, and both callers suppress a non-zero exit — so an unreadable
record has two ways to do damage, and `python/airbyte_sources.py` names them.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path

import pytest
from reconcile_inputs import plan_row

ROOT = Path(__file__).resolve().parents[1]
SELECTOR = ROOT / "python" / "select_connector_sources.py"
FINDER = ROOT / "python" / "find_removed_instances.py"

CONNECTOR = "example-tracker"
TENANT = "example-tenant"
#: An instance of this connector that the plan below does not carry, so both
#: readers have something to say about it when the record is readable.
GONE_NAME = f"{CONNECTOR}-{CONNECTOR}-gone-{TENANT}"

Reader = Callable[[list[object], Path], subprocess.CompletedProcess[str]]


def _run(argv: list[str], sources: list[object]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        argv, input=json.dumps(sources), capture_output=True, text=True, check=False
    )


def _written(tmp_path: Path, name: str, content: str = "[]") -> Path:
    """A file the reader is pointed at. The default is an empty JSON listing."""
    path = tmp_path / name
    path.write_text(content, encoding="utf-8")
    return path


def select(sources: list[object], tmp_path: Path) -> subprocess.CompletedProcess[str]:
    known = _written(tmp_path, "known.json", json.dumps([CONNECTOR]))
    definitions = _written(tmp_path, "definitions.json")
    return _run(
        [sys.executable, str(SELECTOR), CONNECTOR, TENANT, str(known), str(definitions)],
        sources,
    )


def find(sources: list[object], tmp_path: Path) -> subprocess.CompletedProcess[str]:
    plan = _written(
        tmp_path, "plan.tsv", plan_row(CONNECTOR, f"{CONNECTOR}-main", "secret") + "\n"
    )
    definitions = _written(tmp_path, "definitions.json")
    return _run([sys.executable, str(FINDER), str(plan), TENANT, str(definitions)], sources)


class TestARecordWithNoUsableId:
    @pytest.mark.parametrize("reader", [select, find], ids=["selector", "finder"])
    def test_neither_reader_emits_it(self, reader: Reader, tmp_path: Path) -> None:
        """`None` is what a null id becomes on the way to the shell, and the
        shell deletes whatever it is handed."""
        result = reader([{"name": GONE_NAME, "sourceId": None}], tmp_path)

        assert result.returncode == 0, result.stderr
        assert result.stdout == "", "a null id was emitted as a value to delete"


class TestARecordThatIsNotTwoStrings:
    @pytest.mark.parametrize(
        "unreadable",
        [{"name": 7, "sourceId": "src-bad"}, "not-a-source"],
        ids=["a name that is not a string", "a record that is not an object"],
    )
    def test_the_records_after_it_are_still_read(
        self, unreadable: object, tmp_path: Path
    ) -> None:
        """It must not end the listing: the caller cannot tell a crash from an
        empty listing, and an empty listing means "nothing to remove"."""
        result = select([unreadable, {"name": GONE_NAME, "sourceId": "src-good"}], tmp_path)

        assert result.returncode == 0, result.stderr
        assert result.stdout.splitlines() == [f"src-good\t{CONNECTOR}-gone"]
        assert "unreadable" in result.stderr


class TestAPayloadThatIsNotAListing:
    def test_the_selector_refuses_it(self, tmp_path: Path) -> None:
        """Not a listing at all is different from a listing with a bad record in
        it: nothing in it can be trusted, so nothing is emitted and the exit code
        says so."""
        result = select({"sources": []}, tmp_path)  # type: ignore[arg-type]

        assert result.returncode == 1
        assert result.stdout == ""
