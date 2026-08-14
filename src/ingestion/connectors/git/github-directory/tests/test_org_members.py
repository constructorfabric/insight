"""Mock-server tests for the `org_members` stream.

GraphQL POST against api.github.com/graphql, paginated by a cursor injected
into the request body at `variables.cursor`. The pagination test matches on the
exact request body, so a page-2 response is only served when the cursor really
was injected — that is the assertion, not the record count.

The `login_normalized` assertions guard the join key: it becomes the
`value_type='id'` binding the authenticator matches byte-exact against a
case-sensitive column.

Coverage matrix rows: full_refresh_single_page, pagination_multi_page,
empty_page, tenant_source_stamping, schema_conformance, transformations,
error_retry (GraphQL 200 + errors[]).
"""

from __future__ import annotations

import json

from config import CONNECTOR, GRAPHQL_URL, ORG, GitHubDirectoryConfigBuilder
from connector_tests import HttpMocker, HttpRequest, HttpResponse, assert_records_conform, read_stream
from connector_tests.source import load_manifest

_STREAM = "org_members"


def _query() -> str:
    """The stream's GraphQL document, read from the manifest so these tests
    assert the cursor plumbing rather than restating the query text.

    Jinja renders the manifest's block scalar without its single trailing
    newline (`keep_trailing_newline` is off), so the value on the wire differs
    from the raw YAML by exactly that character.
    """
    for s in load_manifest(CONNECTOR)["streams"]:
        if s.get("name") == _STREAM:
            query = s["retriever"]["requester"]["request_body_json"]["query"]
            return query.removesuffix("\n")
    raise AssertionError(f"stream {_STREAM} not found")


def _body(cursor: str | None = None) -> dict:
    variables: dict = {"org": ORG}
    if cursor is not None:
        variables["cursor"] = cursor
    return {"query": _query(), "variables": variables}


def _page(
    edges: list[dict], *, has_next: bool = False, end_cursor: str | None = None
) -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            {
                "data": {
                    "organization": {
                        "membersWithRole": {
                            "pageInfo": {"hasNextPage": has_next, "endCursor": end_cursor},
                            "edges": edges,
                        }
                    }
                }
            }
        ),
        status_code=200,
    )


def _member(login: str, database_id: int, role: str = "MEMBER", **node) -> dict:
    return {
        "role": role,
        "node": {
            "login": login,
            "databaseId": database_id,
            "name": f"Dev {login}",
            "email": None,
            "company": "Example Corp",
            "createdAt": "2026-01-05T08:00:00Z",
            "updatedAt": "2026-02-05T08:00:00Z",
            **node,
        },
    }


def test_full_refresh_single_page(http_mocker: HttpMocker) -> None:
    config = GitHubDirectoryConfigBuilder().build()
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_body()),
        _page([_member("dev-one", 7001, role="ADMIN", email="dev-one@example.com")]),
    )

    output = read_stream(CONNECTOR, _STREAM, config)

    assert not output.errors
    record = output.records[0].record.data
    assert record["login"] == "dev-one"
    assert record["member_id"] == 7001
    assert record["role"] == "ADMIN"
    assert record["name"] == "Dev dev-one"
    assert record["email"] == "dev-one@example.com"
    assert record["company"] == "Example Corp"
    assert record["org"] == ORG
    # the GraphQL edge wrapper is flattened away
    assert "node" not in record


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = GitHubDirectoryConfigBuilder().with_tenant_id("t-9").with_source_id("s-9").build()
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_body()), _page([_member("dev-one", 7001)])
    )

    record = read_stream(CONNECTOR, _STREAM, config).records[0].record.data

    assert record["tenant_id"] == "t-9"
    assert record["source_id"] == "s-9"
    assert record["data_source"] == "insight_github"
    assert record["collected_at"].endswith("Z")
    assert record["unique_key"] == f"t-9:s-9:{ORG}:dev-one"


def test_login_normalized_is_lowercased(http_mocker: HttpMocker) -> None:
    """The identity binding is matched byte-exact against a case-sensitive
    column, and Keycloak lowercases the username it brokers from GitHub."""
    config = GitHubDirectoryConfigBuilder().build()
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_body()), _page([_member("Dev-One", 7001)])
    )

    record = read_stream(CONNECTOR, _STREAM, config).records[0].record.data

    assert record["login"] == "Dev-One"
    assert record["login_normalized"] == "dev-one"
    # the entity key follows the normalized login, so a letter-case change on
    # GitHub does not fork the member into a second bronze row
    assert record["unique_key"] == f"test-tenant:test-source:{ORG}:dev-one"


def test_absent_profile_fields_are_empty_not_the_text_none(http_mocker: HttpMocker) -> None:
    """A field GitHub does not expose must arrive as `''`, never as `"None"`.

    GitHub returns `null` for a member's e-mail unless it is verified and
    visible to the token's scopes, so this is the COMMON case, not an edge one.
    These fields carry `value_type: string`, which renders a Jinja `none` as the
    four-character text `None` — a value, not an absence. Identity anchors on
    e-mail, so every such member would share one "address" and be grouped into a
    single person: one member's sign-in would then load another's profile.

    Asserted as `''` rather than as a real NULL deliberately.
    `fields_history` detects changes with `curr != prev`, which yields NULL
    across a NULL boundary in ClickHouse and emits NO history row in either
    direction — so a field that later gained a value would never be observed at
    all. `''` compares normally and the downstream filters already read it as
    absence.
    """
    config = GitHubDirectoryConfigBuilder().build()
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_body()),
        _page([_member("dev-one", 7001, email=None, name=None, company=None)]),
    )

    record = read_stream(CONNECTOR, _STREAM, config).records[0].record.data

    for field in ("email", "name", "company"):
        assert record[field] == "", (
            f"{field} came through as {record[field]!r}; anything other than '' "
            f"is stored as a value and groups unrelated members together"
        )


def test_pagination_injects_cursor(http_mocker: HttpMocker) -> None:
    config = GitHubDirectoryConfigBuilder().build()
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_body()),
        _page([_member("dev-one", 7001)], has_next=True, end_cursor="CUR1"),
    )
    # Only served if `variables.cursor` was injected into the request body.
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_body(cursor="CUR1")),
        _page([_member("dev-two", 7002)]),
    )

    output = read_stream(CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["login"] for r in output.records] == ["dev-one", "dev-two"]


def test_empty_page(http_mocker: HttpMocker) -> None:
    config = GitHubDirectoryConfigBuilder().build()
    http_mocker.post(HttpRequest(GRAPHQL_URL, body=_body()), _page([]))

    output = read_stream(CONNECTOR, _STREAM, config)

    assert not output.records
    assert not output.errors


def test_schema_conformance(http_mocker: HttpMocker) -> None:
    config = GitHubDirectoryConfigBuilder().build()
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_body()),
        _page([_member("dev-one", 7001), _member("dev-two", 7002, name=None)]),
    )

    output = read_stream(CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, CONNECTOR, _STREAM)


def test_fails_loudly_on_a_graphql_query_error(http_mocker: HttpMocker) -> None:
    """A query-level GraphQL error arrives as HTTP 200 and omits `data`
    entirely — GitHub validates the document before executing it, so an
    insufficient scope rejects the whole query rather than nulling a field.

    Without an explicit filter the extractor just finds nothing: the sync
    reports zero members and the first visible symptom is an interpolation
    error from the paginator, several layers from the cause.
    """
    config = GitHubDirectoryConfigBuilder().build()
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "errors": [
                        {
                            "type": "INSUFFICIENT_SCOPES",
                            "message": "Your token has not been granted the required scopes.",
                        }
                    ]
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(CONNECTOR, _STREAM, config, expecting_exception=True)

    assert not output.records
    assert output.errors
    assert "required scopes" in str(output.errors), (
        "GitHub's own message must reach the operator — a generic failure sends "
        f"them looking in the wrong place: {output.errors}"
    )


def test_retries_graphql_rate_limit(http_mocker: HttpMocker) -> None:
    """GraphQL reports throttling as HTTP 200 with errors[].type=RATE_LIMITED,
    which no status-code filter can see."""
    config = GitHubDirectoryConfigBuilder().build()
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_body()),
        [
            HttpResponse(
                body=json.dumps({"errors": [{"type": "RATE_LIMITED", "message": "throttled"}]}),
                status_code=200,
                headers={"Retry-After": "0"},
            ),
            _page([_member("dev-one", 7001)]),
        ],
    )

    output = read_stream(CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 1
