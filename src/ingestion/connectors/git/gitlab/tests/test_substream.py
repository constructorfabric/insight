from __future__ import annotations

from typing import Any

from airbyte_cdk.models import SyncMode

from source_gitlab.streams.branches import BranchesStream
from source_gitlab.streams.concurrency import RequestGate


class _FakeParent:
    def __init__(self, scopes: list[tuple[dict, list[Any]]]) -> None:
        self._scopes = scopes

    def stream_slices(self, **kwargs: Any) -> list[dict]:
        return [s for s, _ in self._scopes]

    def read_records(
        self, sync_mode: SyncMode, stream_slice: Any = None, **kwargs: Any
    ) -> list[Any]:
        for s, records in self._scopes:
            if s == stream_slice:
                return records
        return []


def _substream(parent: _FakeParent) -> BranchesStream:
    return BranchesStream(
        parent=parent,
        gate=RequestGate(1),
        base_url="https://x",
        token="t",
        tenant_id="tn",
        source_id="s",
    )


class TestUniqueProjectEnumeration:
    def test_dedupes_a_project_across_overlapping_scopes(self) -> None:
        parent = _FakeParent(
            [
                ({"mode": "group", "group": "A"}, [{"id": 1}, {"id": 2}]),
                ({"mode": "group", "group": "B"}, [{"id": 2}, {"id": 3}]),
                ({"mode": "project", "project": "3"}, [{"id": 3}]),
            ]
        )
        ids = [p["id"] for p in _substream(parent)._iter_unique_projects()]
        assert ids == [1, 2, 3]

    def test_skips_non_mapping_and_idless_records(self) -> None:
        parent = _FakeParent(
            [({"mode": "instance"}, [{"id": 1}, "garbage", {"no_id": True}, {"id": 1}])]
        )
        ids = [p["id"] for p in _substream(parent)._iter_unique_projects()]
        assert ids == [1]
