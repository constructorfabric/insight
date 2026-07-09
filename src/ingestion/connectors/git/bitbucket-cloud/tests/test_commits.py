from __future__ import annotations

from source_bitbucket_cloud.streams.commits import CommitsStream
from tests.conftest import SHARED, FakeParent, FakeResponse


def _commit(sha="a" * 40, date="2026-06-01T00:00:00+00:00", **extra):
    base = {
        "hash": sha,
        "date": date,
        "message": "msg",
        "author": {
            "raw": "Ann Author <ann@example.com>",
            "user": {"display_name": "Ann", "uuid": "{a-1}"},
        },
        "parents": [{"hash": "b" * 40}],
    }
    base.update(extra)
    return base


def _branch_record(name="main", target_hash="aa11", target_date="2026-06-15T00:00:00+00:00",
                   is_default=None, workspace="ws", slug="repo"):
    return {
        "workspace": workspace,
        "repo_slug": slug,
        "name": name,
        "target_hash": target_hash,
        "target_date": target_date,
        "is_default": (name == "main") if is_default is None else is_default,
        "default_branch_name": "main",
    }


def _slice(branch=None, cursor_value="", head_sha="aa11"):
    branch = branch or _branch_record()
    return {
        "parent": branch,
        "cursor_value": cursor_value,
        "head_sha": head_sha,
        "partition_key": f"{branch['workspace']}/{branch['repo_slug']}/{branch['name']}",
    }


def _fresh(parent_records=None) -> CommitsStream:
    stream = CommitsStream(parent=FakeParent(records=parent_records or []), **SHARED)
    stream._open_dedup_db()
    return stream


class TestDedupStorage:
    def test_seen_and_mark_first_time_false_then_true(self):
        stream = _fresh()
        assert stream._seen_and_mark("a" * 40) is False
        assert stream._seen_and_mark("a" * 40) is True

    def test_non_hex_hash_falls_back_to_utf8(self):
        stream = _fresh()
        assert stream._seen_and_mark("not-hex!") is False
        assert stream._seen_and_mark("not-hex!") is True

    def test_reset_clears_seen(self):
        stream = _fresh()
        stream._seen_and_mark("a" * 40)
        stream._reset_dedup_for_repo()
        assert stream._seen_and_mark("a" * 40) is False

    def test_seen_without_db_is_noop(self):
        stream = CommitsStream(parent=FakeParent(records=[]), **SHARED)
        assert stream._seen_and_mark("a" * 40) is False

    def test_del_cleans_up(self):
        import os
        stream = _fresh()
        path = stream._dedup_path
        assert os.path.exists(path)
        stream.__del__()
        assert not os.path.exists(path)


class TestPath:
    def test_path(self):
        stream = _fresh()
        assert stream._path(_slice()) == "repositories/ws/repo/commits/main"


class TestParseResponse:
    def test_emits_commit_with_parsed_author(self):
        stream = _fresh()
        records = list(stream.parse_response(
            FakeResponse({"values": [_commit()]}), stream_slice=_slice(),
        ))
        assert len(records) == 1
        rec = records[0]
        assert rec["unique_key"] == f"T:S:ws:repo:{'a' * 40}"
        assert rec["author_name"] == "Ann Author"
        assert rec["author_email"] == "ann@example.com"
        assert rec["author_display_name"] == "Ann"
        assert rec["parent_hashes"] == ["b" * 40]
        assert rec["branch_name"] == "main"
        assert rec["head_sha"] == "aa11"

    def test_author_raw_without_email(self):
        stream = _fresh()
        commit = _commit(author={"raw": "buildbot", "user": None})
        records = list(stream.parse_response(
            FakeResponse({"values": [commit]}), stream_slice=_slice(),
        ))
        rec = records[0]
        assert rec["author_name"] == "buildbot"
        assert rec["author_email"] is None
        assert rec["author_display_name"] is None

    def test_message_truncated(self):
        stream = _fresh()
        commit = _commit(message="x" * 5000)
        records = list(stream.parse_response(
            FakeResponse({"values": [commit]}), stream_slice=_slice(),
        ))
        assert len(records[0]["message"].encode()) <= 1024

    def test_cursor_early_exit(self):
        stream = _fresh()
        payload = {"values": [
            _commit(sha="1" * 40, date="2026-06-20T00:00:00+00:00"),
            _commit(sha="2" * 40, date="2026-06-01T00:00:00+00:00"),
        ]}
        slice_ = _slice(cursor_value="2026-06-10T00:00:00+00:00")
        records = list(stream.parse_response(FakeResponse(payload), stream_slice=slice_))
        assert [r["hash"] for r in records] == ["1" * 40]
        assert stream.next_page_token(FakeResponse({"next": "u"})) is None

    def test_start_date_cutoff(self):
        stream = CommitsStream(
            parent=FakeParent(records=[]), start_date="2026-06-10", **SHARED,
        )
        stream._open_dedup_db()
        payload = {"values": [_commit(date="2026-06-01T00:00:00+00:00")]}
        records = list(stream.parse_response(FakeResponse(payload), stream_slice=_slice()))
        assert records == []
        assert stream.next_page_token(FakeResponse({"next": "u"})) is None

    def test_cross_branch_dedup_stops_pagination(self):
        stream = _fresh()
        main_slice = _slice()
        # main emits the shared commit first
        shared = _commit(sha="c" * 40)
        list(stream.parse_response(FakeResponse({"values": [shared]}), stream_slice=main_slice))
        # feature branch page: one new + the shared commit → new emitted,
        # shared skipped, pagination stopped (merged into seen history)
        feature_slice = _slice(branch=_branch_record(name="feature", is_default=False))
        payload = {"values": [_commit(sha="d" * 40), shared]}
        records = list(stream.parse_response(FakeResponse(payload), stream_slice=feature_slice))
        assert [r["hash"] for r in records] == ["d" * 40]
        assert stream.next_page_token(FakeResponse({"next": "u"})) is None

    def test_new_repo_resets_dedup(self):
        stream = _fresh()
        shared = _commit(sha="e" * 40)
        list(stream.parse_response(FakeResponse({"values": [shared]}), stream_slice=_slice()))
        other_repo = _branch_record(slug="other")
        other_slice = _slice(branch=other_repo)
        records = list(stream.parse_response(FakeResponse({"values": [shared]}), stream_slice=other_slice))
        assert len(records) == 1  # same sha emitted again — different repo


class TestEmitRepo:
    def test_head_unchanged_skip(self):
        stream = _fresh()
        branch = _branch_record(target_hash="aa11", target_date="2026-06-15T00:00:00+00:00")
        state = {"ws/repo/main": {"date": "2026-06-15T00:00:00+00:00", "head_sha": "aa11"}}
        slices = list(stream._emit_repo([{"parent": branch}], state))
        assert slices == []

    def test_force_push_resets_cursor(self):
        stream = _fresh()
        branch = _branch_record(target_hash="NEW1", target_date="2026-06-15T00:00:00+00:00")
        state = {"ws/repo/main": {"date": "2026-06-10T00:00:00+00:00", "head_sha": "OLD1"}}
        slices = list(stream._emit_repo([{"parent": branch}], state))
        assert len(slices) == 1
        assert slices[0]["cursor_value"] == ""  # reset → re-walk ancestry
        assert slices[0]["head_sha"] == "NEW1"

    def test_default_branch_sorted_first(self):
        stream = _fresh()
        feature = _branch_record(name="feature", is_default=False)
        main = _branch_record(name="main")
        slices = list(stream._emit_repo([{"parent": feature}, {"parent": main}], {}))
        assert [s["parent"]["name"] for s in slices] == ["main", "feature"]

    def test_cursor_kept_when_head_matches_but_not_synced(self):
        stream = _fresh()
        # HEAD matches but stored cursor is behind head date → re-slice with cursor
        branch = _branch_record(target_hash="aa11", target_date="2026-06-15T00:00:00+00:00")
        state = {"ws/repo/main": {"date": "2026-06-10T00:00:00+00:00", "head_sha": "aa11"}}
        slices = list(stream._emit_repo([{"parent": branch}], state))
        assert len(slices) == 1
        assert slices[0]["cursor_value"] == "2026-06-10T00:00:00+00:00"


class TestSlicesEndToEnd:
    def test_groups_by_repo_and_filters_invalid(self):
        records = [
            _branch_record(name="main", slug="r1"),
            _branch_record(name="dev", slug="r1", is_default=False),
            _branch_record(name="main", slug="r2"),
            {"workspace": "ws"},  # no slug → skipped
            "junk",               # non-mapping → skipped
        ]
        stream = _fresh(parent_records=records)
        slices = list(stream.stream_slices(sync_mode=None, stream_state={}))
        keys = [s["partition_key"] for s in slices]
        assert keys == ["ws/r1/main", "ws/r1/dev", "ws/r2/main"]


class TestState:
    def test_advances_date_and_head(self):
        stream = _fresh()
        record = {
            "workspace": "ws", "repo_slug": "repo", "branch_name": "main",
            "date": "2026-06-20T00:00:00+00:00", "head_sha": "aa11",
        }
        state = stream.get_updated_state({}, record)
        assert state["ws/repo/main"] == {
            "date": "2026-06-20T00:00:00+00:00", "head_sha": "aa11",
        }

    def test_keeps_max_date(self):
        stream = _fresh()
        state = {"ws/repo/main": {"date": "2026-06-25T00:00:00+00:00"}}
        out = stream.get_updated_state(state, {
            "workspace": "ws", "repo_slug": "repo", "branch_name": "main",
            "date": "2026-06-20T00:00:00+00:00", "head_sha": "bb22",
        })
        assert out["ws/repo/main"]["date"] == "2026-06-25T00:00:00+00:00"
        assert out["ws/repo/main"]["head_sha"] == "bb22"

    def test_schema_covers_all_record_fields(self):
        stream = _fresh()
        record = next(iter(stream.parse_response(
            FakeResponse({"values": [_commit()]}), stream_slice=_slice(),
        )))
        schema_props = set(stream.get_json_schema()["properties"])
        assert set(record) <= schema_props
