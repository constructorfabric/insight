"""A day's roster must arrive whole, or the day must be recognisably short.

`claude_team_code_metrics` asks the vendor to sort by `total_lines_accepted`
descending and then walks the answer by offset. That sort is not a total order:
everyone who accepted no lines that day ties on the sort key, and the vendor
promises no tiebreak. Two requests are two separate answers, so a person inside
the tied block can sit past the page boundary in the first answer and before it
in the second — returned by neither, while both requests succeed.

A declarative stream cannot refuse its own read, so the defect is answered in
two places and these tests cover the connector's half of it:

* a roster that fits inside one page is one answer, and no window can shift
  under it — the happy path below;
* a roster past the page is still lossy, and what makes it *recoverable* is
  that the day's own headcount comes back in the envelope. The reproduction
  proves the loss and proves the headcount exposes it, which is the signal
  `claude_team__ai_dev_usage` gates on.

The mock serves whatever page size the manifest declares, so raising or lowering
that number cannot quietly turn either test into a different one.
"""

from __future__ import annotations

import json

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

# Five people accepted lines that day; everyone else is tied at zero and is
# therefore free to move between one answer and the next.
_WORKED = [f"worked-{i:02d}@example.com" for i in range(5)]


def _page_size() -> int:
    """The page size the manifest declares for the user stream."""
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    stream = next(s for s in manifest["streams"] if s.get("name") == _USER_STREAM)
    return int(stream["retriever"]["paginator"]["pagination_strategy"]["page_size"])


def _roster(size: int) -> list[str]:
    idle = [f"idle-{i:03d}@example.com" for i in range(size - len(_WORKED))]
    return _WORKED + idle


def _user(email: str, lines: int) -> dict:
    return load_fixture(
        __file__,
        "code_metrics_user.json",
        email=email,
        total_lines_accepted=lines,
        avg_lines_accepted_per_day=lines,
        last_active=f"{_DAY}T00:00:00",
    )


def _envelope(emails: list[str], roster_size: int, limit: int, offset: int) -> HttpResponse:
    body = load_fixture(__file__, "code_metrics_page.json")
    body["organization_id"] = ORG_ID
    body["start_date"] = _DAY
    body["end_date"] = _DAY
    body["total_users"] = roster_size
    body["users"] = [_user(e, 100 if e in _WORKED else 0) for e in emails]
    body["pagination"] = {
        "limit": limit,
        "offset": offset,
        "total": roster_size,
        "has_next": offset + len(emails) < roster_size,
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


def _answer(roster: list[str], rotate_ties_by: int) -> list[str]:
    """One request's answer: the people who accepted lines first, then the tied
    block in the order this particular request happened to choose."""
    ties = roster[len(_WORKED) :]
    return _WORKED + ties[rotate_ties_by:] + ties[:rotate_ties_by]


def _emails(output) -> list[str]:
    return [r.record.data["email"] for r in output.records]


def test_a_roster_that_fits_one_page_arrives_whole(http_mocker: HttpMocker) -> None:
    """Under the page size there is one request and one answer, so there is no
    boundary for anyone to fall through."""
    page_size = _page_size()
    roster = _roster(page_size - 5)
    config = ClaudeTeamConfigBuilder().with_start_date(_DAY).build()

    http_mocker.get(
        HttpRequest(METRICS_URL, query_params=_params(page_size)),
        _envelope(_answer(roster, rotate_ties_by=0), len(roster), page_size, 0),
    )

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _USER_STREAM, config)

    assert not output.errors, f"the read failed: {output.errors}"
    assert sorted(set(_emails(output))) == sorted(roster)
    assert len(_emails(output)) == len(roster), "a single page emitted a person twice"


def test_a_roster_past_the_page_loses_people_and_the_headcount_shows_it(http_mocker: HttpMocker) -> None:
    """Past the page the tied block can be laid out differently in each answer,
    and then the walk both drops people and repeats others — in equal number,
    because every person who moves one way across the boundary displaces one
    moving the other. Neither request fails, so the only thing that betrays the
    short day is the headcount the envelope carries."""
    page_size = _page_size()
    roster = _roster(page_size + 5)
    config = ClaudeTeamConfigBuilder().with_start_date(_DAY).build()

    first = _answer(roster, rotate_ties_by=0)[:page_size]
    second = _answer(roster, rotate_ties_by=len(roster) // 2)[page_size:]
    http_mocker.get(
        HttpRequest(METRICS_URL, query_params=_params(page_size)), _envelope(first, len(roster), page_size, 0)
    )
    http_mocker.get(
        HttpRequest(METRICS_URL, query_params=_params(page_size, offset=page_size)),
        _envelope(second, len(roster), page_size, page_size),
    )

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _USER_STREAM, config)

    emitted = _emails(output)
    distinct = set(emitted)
    missing = sorted(set(roster) - distinct)

    assert not output.errors, (
        "the read reported an error — if the stream has learnt to refuse a short "
        "read, this test is describing behaviour that no longer exists"
    )
    assert missing, "the mock did not reproduce the cross-page shift"
    assert len(emitted) - len(distinct) == len(missing), (
        f"gaps and duplicates should balance: {len(emitted)} emitted, {len(distinct)} distinct, {len(missing)} missing"
    )
    # The gate downstream compares exactly these two numbers.
    assert len(distinct) < len(roster)


def test_the_day_carries_the_vendor_s_own_headcount(http_mocker: HttpMocker) -> None:
    """The org-level row stores how many people the vendor has for the day, so a
    day stored short can be told apart from a quiet one.

    The declared schema is asserted alongside the record: the envelope always
    carries the count, but only a declared field becomes a Bronze column, so a
    record-only check would pass over the gap that matters.
    """
    roster = _roster(_page_size() + 5)
    config = ClaudeTeamConfigBuilder().with_start_date(_DAY).build()
    http_mocker.get(HttpRequest(METRICS_URL, query_params=_org_params(1)), _envelope([_WORKED[0]], len(roster), 1, 0))

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _ORG_STREAM, config)

    assert not output.errors, f"the read failed: {output.errors}"
    assert len(output.records) == 1
    assert output.records[0].record.data.get("total_users") == len(roster)

    declared = stream_schema(_CONNECTOR, _ORG_STREAM)["properties"]
    assert "total_users" in declared, (
        "the org stream does not declare total_users, so the vendor's headcount for "
        "the day never becomes a Bronze column and nothing downstream can tell a "
        f"short read from a quiet day. Declared: {sorted(declared)}"
    )


def test_each_read_of_a_day_keeps_its_own_headcount(http_mocker: HttpMocker) -> None:
    """The org row's key carries the read, not just the day. A headcount is only
    true as of the read that returned it — a day still in progress reports fewer
    people than it ends with — so a later read must not overwrite the reference
    an earlier one left behind."""
    roster = _roster(_page_size() + 5)
    config = ClaudeTeamConfigBuilder().with_start_date(_DAY).build()
    http_mocker.get(HttpRequest(METRICS_URL, query_params=_org_params(1)), _envelope([_WORKED[0]], len(roster), 1, 0))

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _ORG_STREAM, config)

    key = output.records[0].record.data["unique_key"]
    assert key.endswith("T11:30:00Z"), (
        f"the org row's key does not name the read: {key!r}. Without it a later "
        "read of the same day replaces the reference the gate needs."
    )
    assert _DAY in key, f"the org row's key no longer names the day: {key!r}"
