"""Mock-server tests for the `users` stream.

Plain paginated stream: GET /v2/users behind a Server-to-Server OAuth token
exchange (SessionTokenAuthenticator), CursorPagination on `next_page_token`,
`users` extract field, AddFields stamping tenant_id / source_id /
unique_key = "{tenant}-{source}-{id}".

Coverage matrix rows: full_refresh_single_page, schema_conformance,
tenant_source_stamping, empty_page, pagination_multi_page, error_retry (429).
The exact query matcher on page_size=100 also pins the PR #1746 review fix
(the users requester previously sent no page_size at all).
"""

from __future__ import annotations

import json

from config import API_URL, ZoomConfigBuilder, mock_token
from connector_tests import HttpMocker, HttpRequest, HttpResponse, assert_records_conform, load_fixture, read_stream

_STREAM = "users"
_CONNECTOR = "collaboration/zoom"
_USERS_URL = f"{API_URL}/users"


def _user(uid: str, email: str) -> dict:
    return load_fixture(__file__, "user.json", id=uid, email=email)


def _page(users: list[dict], next_token: str | None = None) -> HttpResponse:
    body: dict = {"users": users}
    if next_token:
        body["next_page_token"] = next_token
    return HttpResponse(body=json.dumps(body), status_code=200)


def _params(page_token: str | None = None) -> dict:
    params = {"page_size": "100"}
    if page_token:
        params["next_page_token"] = page_token
    return params


def test_full_refresh_single_page(http_mocker: HttpMocker) -> None:
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    http_mocker.get(
        HttpRequest(_USERS_URL, query_params=_params()),
        _page([_user("usr-1", "alice@example.com"), _user("usr-2", "bob@example.com")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors
    assert sorted(r.record.data["email"] for r in output.records) == ["alice@example.com", "bob@example.com"]


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    http_mocker.get(HttpRequest(_USERS_URL, query_params=_params()), _page([_user("usr-1", "alice@example.com")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-usr-1")


def test_schema_conformance(http_mocker: HttpMocker) -> None:
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    http_mocker.get(HttpRequest(_USERS_URL, query_params=_params()), _page([_user("usr-1", "alice@example.com")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_pagination_multi_page(http_mocker: HttpMocker) -> None:
    """CursorPagination: a next_page_token in the response drives a second
    request carrying it; a response without the token stops."""
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    http_mocker.get(
        HttpRequest(_USERS_URL, query_params=_params()),
        _page([_user("usr-1", "alice@example.com")], next_token="tok-2"),
    )
    http_mocker.get(
        HttpRequest(_USERS_URL, query_params=_params(page_token="tok-2")), _page([_user("usr-2", "bob@example.com")])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert output.records[-1].record.data["id"] == "usr-2"


def test_empty_page(http_mocker: HttpMocker) -> None:
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    http_mocker.get(HttpRequest(_USERS_URL, query_params=_params()), _page([]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 0
    assert not output.errors


def test_error_retry_429(http_mocker: HttpMocker) -> None:
    """The manifest error handler RETRIES 429 (WaitTimeFromHeader Retry-After);
    the read must succeed once the source recovers, without losing records."""
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    http_mocker.get(
        HttpRequest(_USERS_URL, query_params=_params()),
        [
            HttpResponse(
                body=json.dumps({"code": 429, "message": "rate limited"}), status_code=429, headers={"Retry-After": "0"}
            ),
            _page([_user("usr-1", "alice@example.com")]),
        ],
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    assert not output.errors
