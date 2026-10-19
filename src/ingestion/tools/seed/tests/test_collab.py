"""The collaboration generator emits each vendor's own report shape."""

from __future__ import annotations

import datetime as _dt
from typing import Any

import pytest

from insight_seed.generators import base, collab
from insight_seed.profiles import TEAM_PROFILES, build_roster

DEV_EMAIL = "dev@company.nonpresent"
TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"
DAYS = 21
ANCHOR = _dt.date(2026, 10, 1)

Table = tuple[list[str], list[tuple[Any, ...]]]


@pytest.fixture(autouse=True)
def _pinned_anchor(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(base, "_anchor_cache", ANCHOR)


@pytest.fixture
def emitted(monkeypatch: pytest.MonkeyPatch) -> dict[str, Table]:
    captured: dict[str, Table] = {}

    def capture(
        client: Any, schema: str, table: str, cols: list[str], rows: list[tuple[Any, ...]]
    ) -> int:
        captured[f"{schema}.{table}"] = (list(cols), list(rows))
        return len(rows)

    monkeypatch.setattr(collab, "truncate", lambda client, schema, table: None)

    monkeypatch.setattr(collab, "prune_to_bronze", lambda client, snapshot, tenant: None)
    monkeypatch.setattr(collab, "bulk_insert", capture)
    return captured


def _column(table: Table, name: str) -> list[Any]:
    cols, rows = table
    return [row[cols.index(name)] for row in rows]


def test_gmail_is_not_emitted_because_no_connector_delivers_it(emitted: dict[str, Table]) -> None:
    """A gmail-sourced row is one no install could hold. The weight stays in
    TEAM_PROFILES for whenever that connector lands."""
    assert any(TEAM_PROFILES[team].weights.get("gmail", 0) > 0 for team in TEAM_PROFILES), (
        "no team carries a gmail weight, so this guard proves nothing"
    )
    roster = build_roster(DEV_EMAIL)

    collab.generate(None, roster, TENANT, DAYS)

    assert not [name for name in emitted if "gmail" in name], (
        f"a gmail stream was written: {sorted(emitted)}"
    )
    assert "bronze_m365.email_activity" in emitted, "no email was emitted at all"


def test_a_teams_report_row_carries_both_chat_and_meetings(emitted: dict[str, Table]) -> None:
    """Microsoft reports both on one row per licensed user per day, which is
    why a day with only one kind still reports the other as zero."""
    roster = build_roster(DEV_EMAIL)

    collab.seed_m365_teams_activity(None, roster, TENANT, DAYS)

    table = emitted["bronze_m365.teams_activity"]
    assert table[1], "no teams report row was emitted"
    chat = _column(table, "privateChatMessageCount")
    meetings = _column(table, "meetingsAttendedCount")
    assert any(c == 0 for c in chat) or any(m == 0 for m in meetings), (
        "no row carries a zero for one kind, so the combined-row shape is untested"
    )
    assert all(c + m > 0 for c, m in zip(chat, meetings, strict=True)), (
        "a row was emitted for a day with neither chat nor meetings"
    )


def test_a_reported_day_is_never_read_as_a_graph_copy(emitted: dict[str, Table]) -> None:
    """m365_reported_days drops a whole day when any row shows activity after
    its own lastActivityDate — Graph's signature for an uncomputed report."""
    roster = build_roster(DEV_EMAIL)

    collab.seed_m365_teams_activity(None, roster, TENANT, DAYS)
    collab.seed_m365_email_activity(None, roster, TENANT, DAYS)

    for stream in ("teams_activity", "email_activity"):
        table = emitted[f"bronze_m365.{stream}"]
        for report_day, last_activity in zip(
            _column(table, "reportRefreshDate"), _column(table, "lastActivityDate"), strict=True
        ):
            assert last_activity >= report_day, (
                f"{stream} reports activity on {report_day} last seen {last_activity}, "
                f"which drops the whole day"
            )


def test_only_the_person_whose_day_it_is_carries_an_email(emitted: dict[str, Table]) -> None:
    """A Zoom participant with an email becomes a person of their own. The
    other seats are company presence, not people — so they carry none."""
    roster = build_roster(DEV_EMAIL)

    collab.seed_zoom_meetings(None, roster, TENANT, DAYS)

    table = emitted["bronze_zoom.participants"]
    assert table[1], "no participant was emitted"
    known = {person.email for person in roster}
    for email in _column(table, "email"):
        assert email == "" or email in known, f"{email} is neither blank nor a roster person"


def test_every_meeting_has_someone_else_in_it(emitted: dict[str, Table]) -> None:
    """A participant alone throughout earns no company time, and a meeting with
    no company time is not attended."""
    roster = build_roster(DEV_EMAIL)

    collab.seed_zoom_meetings(None, roster, TENANT, DAYS)

    table = emitted["bronze_zoom.participants"]
    seats: dict[str, int] = {}
    for meeting in _column(table, "meeting_uuid"):
        seats[meeting] = seats.get(meeting, 0) + 1

    assert seats, "no meeting was emitted"
    assert min(seats.values()) >= 2, (
        f"a meeting has {min(seats.values())} participant(s), so it earns no company time"
    )
