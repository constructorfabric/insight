from __future__ import annotations

from source_gitlab.streams.branches import BranchesStream
from source_gitlab.streams.commits import CommitsStream
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.file_changes import CommitFileChangesStream
from source_gitlab.streams.projects import ProjectsStream
from source_gitlab.streams.users import UsersStream

SH = dict(base_url="https://x", token="t", tenant_id="tn", source_id="s")


def _streams() -> dict[str, object]:
    p = ProjectsStream(groups=(), projects=(), **SH)
    b = BranchesStream(parent=p, gate=RequestGate(2), **SH)
    return {
        "projects": p,
        "users": UsersStream(groups=(), projects=(), **SH),
        "branches": b,
        "commits": CommitsStream(
            parent=p, branches=b, gate=RequestGate(2), start_date="2025-01-01", **SH
        ),
        "file_changes": CommitFileChangesStream(
            parent=p, branches=b, gate=RequestGate(2), start_date="2025-01-01", **SH
        ),
    }


class TestResumability:
    def test_full_refresh_streams_are_not_resumable(self) -> None:
        streams = _streams()
        for name in ("projects", "users", "branches"):
            assert streams[name].is_resumable is False, name

    def test_incremental_streams_are_resumable(self) -> None:
        streams = _streams()
        for name in ("commits", "file_changes"):
            assert streams[name].is_resumable is True, name
