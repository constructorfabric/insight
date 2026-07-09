"""copilot_seats — full-refresh, offset-paginated stream.

Two divergent pagination code paths (Link-header vs count-heuristic fallback)
and identity extraction from the nested `assignee` object are the risk here.
"""

from __future__ import annotations

import pytest

from source_github_copilot.streams.base import CopilotAuthError
from source_github_copilot.streams.seats import CopilotSeatsStream
from tests.conftest import FakeResponse, SHARED_STREAM_KWARGS, real_response


def _seats() -> CopilotSeatsStream:
    return CopilotSeatsStream(**SHARED_STREAM_KWARGS)


def _seats_page(n: int) -> dict:
    return {
        "total_seats": n,
        "seats": [
            {
                "assignee": {"login": f"user-{i}", "email": f"user-{i}@example.com"},
                "plan_type": "business",
            }
            for i in range(n)
        ],
    }


class TestPath:
    def test_path_includes_org(self):
        assert _seats().path() == "orgs/acme/copilot/billing/seats"


class TestRequestParams:
    def test_first_page_defaults_to_page_one(self):
        params = _seats().request_params()
        assert params == {"per_page": "100", "page": "1"}

    def test_next_page_token_page_is_used(self):
        params = _seats().request_params(next_page_token={"page": 3})
        assert params == {"per_page": "100", "page": "3"}


class TestNextPageToken:
    def test_non_response_object_returns_none(self):
        # isinstance(response, requests.Response) guard — a duck-typed stand-in fails it.
        assert _seats().next_page_token(FakeResponse(status_code=200, payload=_seats_page(100))) is None

    def test_non_200_status_returns_none(self):
        assert _seats().next_page_token(real_response(status_code=404)) is None

    def test_link_header_next_is_authoritative(self):
        link = (
            '<https://api.github.com/orgs/acme/copilot/billing/seats?page=3&per_page=100>; rel="next"'
        )
        resp = real_response(status_code=200, headers={"Link": link}, content=b'{"seats": []}')
        assert _seats().next_page_token(resp) == {"page": 3}

    def test_no_link_header_full_page_falls_back_to_count_heuristic(self):
        import json

        resp = real_response(
            status_code=200,
            content=json.dumps(_seats_page(100)).encode(),
            url="https://api.github.com/orgs/acme/copilot/billing/seats?page=1&per_page=100",
        )
        assert _seats().next_page_token(resp) == {"page": 2}

    def test_no_link_header_under_full_page_returns_none(self):
        import json

        resp = real_response(status_code=200, content=json.dumps(_seats_page(50)).encode())
        assert _seats().next_page_token(resp) is None

    def test_no_link_header_exact_hundred_does_not_truncate(self):
        """Regression the Link-header rewrite exists to fix: an exact 100-seat org
        must still be asked for page 2 via the fallback, not stopped early."""
        import json

        resp = real_response(status_code=200, content=json.dumps(_seats_page(100)).encode())
        assert _seats().next_page_token(resp) is not None

    def test_non_json_body_with_no_link_header_returns_none(self):
        resp = real_response(status_code=200, content=b"not json at all")
        assert _seats().next_page_token(resp) is None


class TestParseResponse:
    def test_extracts_flat_identity_from_nested_assignee(self):
        resp = FakeResponse(status_code=200, payload=_seats_page(1))
        records = list(_seats().parse_response(resp))
        assert records[0]["user_login"] == "user-0"
        assert records[0]["user_email"] == "user-0@example.com"

    def test_seat_without_assignee_login_is_skipped(self):
        payload = {"seats": [{"assignee": {}, "plan_type": "business"}]}
        resp = FakeResponse(status_code=200, payload=payload)
        assert list(_seats().parse_response(resp)) == []

    def test_seat_with_no_assignee_at_all_is_skipped(self):
        payload = {"seats": [{"plan_type": "business"}]}
        resp = FakeResponse(status_code=200, payload=payload)
        assert list(_seats().parse_response(resp)) == []

    def test_null_email_is_tolerated_not_dropped(self):
        payload = {"seats": [{"assignee": {"login": "user-x", "email": None}}]}
        resp = FakeResponse(status_code=200, payload=payload)
        records = list(_seats().parse_response(resp))
        assert len(records) == 1
        assert records[0]["user_email"] is None

    def test_non_json_body_yields_nothing_not_raises(self):
        resp = FakeResponse(status_code=200, json_error=ValueError("bad"))
        assert list(_seats().parse_response(resp)) == []

    def test_401_propagates_auth_error(self):
        with pytest.raises(CopilotAuthError):
            list(_seats().parse_response(FakeResponse(status_code=401)))

    def test_204_yields_nothing_no_raise(self):
        assert list(_seats().parse_response(FakeResponse(status_code=204))) == []

    def test_unique_key_keyed_on_login(self):
        resp = FakeResponse(status_code=200, payload=_seats_page(1))
        record = next(iter(_seats().parse_response(resp)))
        assert record["unique_key"] == "T-S-user-0"

    def test_raw_assignee_retained_for_passthrough(self):
        resp = FakeResponse(status_code=200, payload=_seats_page(1))
        record = next(iter(_seats().parse_response(resp)))
        assert record["assignee"]["login"] == "user-0"

    def test_no_seats_key_yields_nothing(self):
        assert list(_seats().parse_response(FakeResponse(status_code=200, payload={}))) == []


class TestGetJsonSchema:
    def test_identity_and_framework_fields_present(self):
        schema = _seats().get_json_schema()
        props = schema["properties"]
        for field in ("user_login", "user_email", "tenant_id", "unique_key", "data_source"):
            assert field in props
        assert schema["additionalProperties"] is True
