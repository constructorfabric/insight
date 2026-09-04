"""A day's roster must arrive whole, however the vendor orders it.

`claude_team_code_metrics` asks the vendor to sort by `total_lines_accepted`
descending and then walks the answer by offset. That sort is not a total order:
everyone who accepted no lines that day ties on the sort key, and the vendor
promises no tiebreak. Two requests are two separate answers, so a person inside
the tied block can sit past the page boundary in the first answer and before it
in the second — returned by neither, while both requests succeed.

The rule these tests state is the one that matters downstream: every person the
vendor lists for a day reaches the stream. How many requests it takes is the
manifest's business, so the mock serves whatever page size the manifest declares
rather than pinning a number.

The companion rule is that the day carries its own total. The envelope reports
how many people the vendor has for the day; storing it is what lets a short read
be detected instead of passing for a quiet day.
"""

from __future__ import annotations

import json

import pytest
import yaml
from config import METRICS_URL, ORG_ID, ClaudeTeamConfigBuilder
from connector_tests import (
    HttpMocker,
    HttpRequest,
    HttpResponse,
    connector_dir,
    load_fixture,
    read_stream,
    stream_schema,
)
from freezegun import freeze_time

_CONNECTOR = "ai/claude-team"
_USER_STREAM = "claude_team_code_metrics"
_ORG_STREAM = "claude_team_code_metrics_org"
_DAY = "2026-08-19"
_NOW = f"{_DAY}T11:30:00Z"

# Five people accepted lines that day and 100 did not, so the tied block is
# wider than any page the manifest is likely to ask for.
_WORKED = [f"worked-{i:02d}@example.com" for i in range(5)]
_IDLE = [f"idle-{i:03d}@example.com" for i in range(100)]
_ROSTER = _WORKED + _IDLE


def _page_size() -> int:
    """The page size the manifest declares for the user stream."""
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    stream = next(s for s in manifest["streams"] if s.get("name") == _USER_STREAM)
    return int(stream["retriever"]["paginator"]["pagination_strategy"]["page_size"])


def _user(email: str, lines: int) -> dict:
    return load_fixture(
        __file__,
        "code_metrics_user.json",
        email=email,
        total_lines_accepted=lines,
        avg_lines_accepted_per_day=lines,
        last_active=f"{_DAY}T00:00:00",
    )


def _envelope(users: list[dict], limit: int, offset: int) -> HttpResponse:
    body = load_fixture(__file__, "code_metrics_page.json")
    body["organization_id"] = ORG_ID
    body["start_date"] = _DAY
    body["end_date"] = _DAY
    body["total_users"] = len(_ROSTER)
    body["users"] = users
    body["pagination"] = {
        "limit": limit,
        "offset": offset,
        "total": len(_ROSTER),
        "has_next": offset + len(users) < len(_ROSTER),
    }
    return HttpResponse(body=json.dumps(body), status_code=200)


def _org_params(limit: int) -> dict[str, str]:
    """The org stream asks for the envelope only — no sort, no paging."""
    return {
        "organization_uuid": ORG_ID,
        "customer_type": "claude_ai",
        "subscription_type": "team",
        "start_date": _DAY,
        "end_date": _DAY,
        "limit": str(limit),
    }


def _params(limit: int, offset: int | None = None) -> dict[str, str]:
    params = _org_params(limit) | {"sort_by": "total_lines_accepted", "sort_order": "desc"}
    if offset is not None:
        params["offset"] = str(offset)
    return params


def _ordering(rotate_ties_by: int) -> list[str]:
    """The vendor's answer for one request: the people who accepted lines first,
    then the tied block in an order this request happens to have chosen."""
    ties = _IDLE[rotate_ties_by:] + _IDLE[:rotate_ties_by]
    return _WORKED + ties


def _emails(output) -> set[str]:
    return {r.record.data["email"] for r in output.records}


@pytest.mark.xfail(
    strict=True,
    reason="#3172 is open: the stream still pages by offset over a sort key that "
    "is not unique. Strict, so this flips to a failure the moment the ordering "
    "becomes total and stops being a reproduction.",
)
def test_no_one_is_lost_when_the_vendor_reorders_the_tied_block(http_mocker: HttpMocker) -> None:
    """Every person the vendor lists for the day reaches the stream, even when
    the order of the people tied on the sort key differs between requests."""
    page_size = _page_size()
    config = ClaudeTeamConfigBuilder().with_start_date(_DAY).build()

    first = _ordering(rotate_ties_by=0)[:page_size]
    http_mocker.get(
        HttpRequest(METRICS_URL, query_params=_params(page_size)),
        _envelope([_user(e, 100 if e in _WORKED else 0) for e in first], page_size, 0),
    )
    if page_size < len(_ROSTER):
        # The second request is a second answer, and the vendor may lay the tied
        # block out differently in it.
        second = _ordering(rotate_ties_by=50)[page_size:]
        http_mocker.get(
            HttpRequest(METRICS_URL, query_params=_params(page_size, offset=page_size)),
            _envelope([_user(e, 100 if e in _WORKED else 0) for e in second], page_size, page_size),
        )

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _USER_STREAM, config)

    assert not output.errors, f"the read failed: {output.errors}"
    missing = sorted(set(_ROSTER) - _emails(output))
    assert not missing, (
        f"{len(missing)} of {len(_ROSTER)} people the vendor listed for {_DAY} never reached "
        f"the stream, and the read reported no error. First few: {missing[:5]}"
    )


def test_the_day_carries_the_vendor_s_own_headcount(http_mocker: HttpMocker) -> None:
    """The org-level row stores how many people the vendor has for the day, so a
    day stored short can be told apart from a quiet one.

    The declared schema is asserted alongside the record: the envelope always
    carries the count, but only a declared field becomes a Bronze column, so a
    record-only check would pass over the gap that matters.
    """
    config = ClaudeTeamConfigBuilder().with_start_date(_DAY).build()
    http_mocker.get(HttpRequest(METRICS_URL, query_params=_org_params(1)), _envelope([_user(_WORKED[0], 100)], 1, 0))

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _ORG_STREAM, config)

    assert not output.errors, f"the read failed: {output.errors}"
    assert len(output.records) == 1
    assert output.records[0].record.data.get("total_users") == len(_ROSTER)

    declared = stream_schema(_CONNECTOR, _ORG_STREAM)["properties"]
    assert "total_users" in declared, (
        "the org stream does not declare total_users, so the vendor's headcount for "
        "the day never becomes a Bronze column and nothing downstream can tell a "
        f"short read from a quiet day. Declared: {sorted(declared)}"
    )
