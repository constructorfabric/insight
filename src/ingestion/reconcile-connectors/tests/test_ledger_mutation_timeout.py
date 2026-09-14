"""A synchronous mutation is not given a request's deadline.

`adopt_identity` runs `ALTER TABLE … UPDATE … SETTINGS mutations_sync = 1`, so
the call returns when the rewrite is finished rather than when it is accepted —
over however much unidentified history the install is upgrading with. Past the
deadline the mutation carries on server-side while the tick reports a failure,
and the next tick issues the same statement again.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from sweep import ledger as ledger_module
from sweep.ledger import Ledger
from sweep.plan import Instance

INSTANCE = Instance("example-tracker", "example-tenant", "example-tracker-main")


class _Response:
    def __enter__(self) -> _Response:
        return self

    def __exit__(self, *exc: object) -> bool:
        return False

    def read(self) -> bytes:
        return b""


@pytest.fixture
def deadlines(monkeypatch: pytest.MonkeyPatch) -> list[int]:
    """Every timeout the ledger hands urllib, in call order."""
    seen: list[int] = []

    def fake_urlopen(request: object, timeout: int) -> _Response:
        seen.append(timeout)
        return _Response()

    monkeypatch.setattr(ledger_module.urllib.request, "urlopen", fake_urlopen)
    return seen


def _ledger() -> Ledger:
    return Ledger("http://ledger.invalid:8123", "user", "password")


class TestTheDeadlineMatchesTheStatement:
    def test_a_mutation_waits_longer_than_a_request(self, deadlines: list[int]) -> None:
        _ledger().adopt_identity(INSTANCE)

        assert deadlines == [ledger_module._MUTATION_TIMEOUT_SECS]
        assert ledger_module._MUTATION_TIMEOUT_SECS > ledger_module._TIMEOUT_SECS

    def test_an_ordinary_write_keeps_the_short_one(self, deadlines: list[int]) -> None:
        """The longer deadline belongs to the statement that earns it. An insert
        that hangs must still fail inside one tick."""
        _ledger().insert([{"tick_id": "tick-a", "connector": "example-tracker"}])

        assert deadlines == [ledger_module._TIMEOUT_SECS]
