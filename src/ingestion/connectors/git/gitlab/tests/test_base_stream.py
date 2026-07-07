"""GitlabStream / ScopedGitlabStream / GitlabSubstream HTTP-level behaviour."""

from __future__ import annotations

import pytest
import requests

from source_gitlab.streams.base import trim_text
from source_gitlab.streams.branches import BranchesStream
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.errors import GitlabApiError, GitlabAuthError
from source_gitlab.streams.projects import ProjectsStream
from tests.conftest import BASE, FakeParent, FakeResponse

SCOPED = {**BASE, "groups": (), "projects": ()}


def _projects() -> ProjectsStream:
    return ProjectsStream(**SCOPED)


def _branches() -> BranchesStream:
    return BranchesStream(parent=FakeParent([]), gate=RequestGate(1), **BASE)


class TestTrimText:
    def test_none(self):
        assert trim_text(None, 5) == (None, False)

    def test_short(self):
        assert trim_text("abc", 5) == ("abc", False)

    def test_truncated(self):
        assert trim_text("abcdef", 5) == ("abcde", True)

    def test_non_string_coerced(self):
        assert trim_text(12345, 3) == ("123", True)


class TestUrlAndHeaders:
    def test_url_base(self):
        assert _projects().url_base == "https://gl.example/api/v4/"

    def test_private_token_header(self):
        assert _projects().request_headers() == {"PRIVATE-TOKEN": "tok"}

    def test_is_resumable_tracks_incremental_support(self):
        from source_gitlab.streams.issues import IssuesStream

        assert _projects().is_resumable is False
        issues = IssuesStream(
            parent=FakeParent([]), gate=RequestGate(1), groups=(), projects=(),
            start_date=None, **BASE,
        )
        assert issues.is_resumable is True


class TestPagination:
    def test_next_page_token_from_link_header(self):
        resp = FakeResponse(links={"next": {"url": "https://gl.example/api/v4/projects?page=2"}})
        assert _projects().next_page_token(resp) == {
            "next_url": "https://gl.example/api/v4/projects?page=2",
        }

    def test_no_next_link(self):
        assert _projects().next_page_token(FakeResponse(links={})) is None
        assert _projects().next_page_token(FakeResponse(links={"next": {}})) is None

    def test_path_prefers_next_url(self):
        path = _projects().path(
            next_page_token={"next_url": "https://gl.example/api/v4/projects?page=2"},
        )
        assert path == "projects?page=2"

    def test_relative_next_path_without_query(self):
        assert _projects()._relative_next_path("https://gl.example/api/v4/projects") == (
            "projects"
        )

    def test_relative_next_path_without_api_marker(self):
        assert _projects()._relative_next_path("https://gl.example/other/projects") == (
            "other/projects"
        )

    def test_path_falls_back_to_slice(self):
        assert _projects().path(stream_slice={"mode": "instance"}) == "projects"

    def test_request_params_empty_when_following_next(self):
        assert _projects().request_params(next_page_token={"next_url": "u"}) == {}

    def test_request_params_initial(self):
        params = _projects().request_params(stream_slice={"mode": "project", "project": "x"})
        assert params == {"statistics": "true"}


class TestRetry:
    def test_non_response_object_retried(self):
        assert _projects().should_retry("not-a-response") is True

    def test_response_delegates_to_concurrency(self):
        resp = requests.Response()
        resp.status_code = 429
        assert _projects().should_retry(resp) is True
        resp.status_code = 200
        assert _projects().should_retry(resp) is False

    def test_backoff_non_response_is_60s(self):
        assert _projects().backoff_time("not-a-response") == 60.0

    def test_backoff_429_uses_retry_after(self):
        resp = requests.Response()
        resp.status_code = 429
        resp.headers["Retry-After"] = "7"
        assert _projects().backoff_time(resp) == 7.0


class TestParseResponse:
    def test_list_payload_enveloped(self):
        records = list(_projects().parse_response(
            FakeResponse([{"id": 1}, {"id": 2}]), stream_slice=None,
        ))
        assert [r["unique_key"] for r in records] == ["T:S:1", "T:S:2"]
        assert all(r["tenant_id"] == "T" for r in records)

    def test_dict_payload_single_record(self):
        records = list(_projects().parse_response(
            FakeResponse({"id": 3}), stream_slice=None,
        ))
        assert [r["unique_key"] for r in records] == ["T:S:3"]

    def test_auth_error_raises(self):
        with pytest.raises(GitlabAuthError):
            list(_projects().parse_response(FakeResponse(status_code=401), stream_slice=None))
        with pytest.raises(GitlabAuthError):
            list(_projects().parse_response(FakeResponse(status_code=403), stream_slice=None))

    def test_unexpected_4xx_raises(self):
        with pytest.raises(GitlabApiError):
            list(_projects().parse_response(FakeResponse(status_code=404), stream_slice=None))

    def test_skippable_status_skips_silently(self):
        # branches (substream) treats 404 as deleted-entity: no records, no raise
        records = list(_branches().parse_response(
            FakeResponse(status_code=404), stream_slice={"parent": {"id": 1}},
        ))
        assert records == []

    def test_skip_disabled_via_slice_flag(self):
        with pytest.raises(GitlabApiError):
            list(_branches().parse_response(
                FakeResponse(status_code=404),
                stream_slice={"parent": {"id": 1}, "skip_404": False},
            ))

    def test_mid_pagination_404_not_skippable(self):
        with pytest.raises(GitlabApiError):
            list(_branches().parse_response(
                FakeResponse(status_code=404),
                stream_slice={"parent": {"id": 1}},
                next_page_token={"next_url": "u"},
            ))


class TestJsonSchema:
    def test_schema_loads_and_caches(self):
        stream = _projects()
        schema = stream.get_json_schema()
        assert "properties" in schema
        assert stream.get_json_schema() is schema  # @cache

    def test_schema_covers_envelope_fields(self):
        stream = _projects()
        props = set(stream.get_json_schema()["properties"])
        assert {"tenant_id", "source_id", "unique_key", "data_source"} <= props


class TestSubstreamSlices:
    def test_substream_slices_wrap_parent_records(self):
        parent = FakeParent([({"mode": "instance"}, [{"id": 1}, {"id": 2}])])
        stream = BranchesStream(parent=parent, gate=RequestGate(1), **BASE)
        # GitlabSubstream.stream_slices comes from the base; BranchesStream
        # overrides it — call the base implementation explicitly.
        from source_gitlab.streams.base import GitlabSubstream

        slices = list(GitlabSubstream.stream_slices(stream))
        assert slices == [{"parent": {"id": 1}}, {"parent": {"id": 2}}]

    def test_parent_value_present(self):
        assert _branches()._parent_value({"parent": {"id": 9}}, "id") == 9
