"""The CRM generator emits HubSpot's streams, keyed the way staging reads them."""

from __future__ import annotations

import datetime as _dt
from typing import Any

import pytest

from insight_seed.generators import base, crm
from insight_seed.profiles import build_roster

DEV_EMAIL = "dev@company.nonpresent"
TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"
DAYS = 30
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

    monkeypatch.setattr(crm, "truncate", lambda client, schema, table: None)
    monkeypatch.setattr(crm, "bulk_insert", capture)
    return captured


def _column(table: Table, name: str) -> list[Any]:
    cols, rows = table
    return [row[cols.index(name)] for row in rows]


def test_every_engagement_lands_on_a_stream_the_staging_model_reads(
    emitted: dict[str, Table],
) -> None:
    """`hubspot__crm_activities` unions three streams and names the kind after
    the one a row arrived on — a fourth would simply never be counted."""
    roster = build_roster(DEV_EMAIL)

    crm.seed_engagements(None, roster, TENANT, DAYS)

    assert set(emitted) == {f"bronze_hubspot.{stream}" for stream in crm._CRM_ACTIVITY_STREAMS}
    assert sum(len(table[1]) for table in emitted.values()) > 0, "no engagement was emitted"


def test_an_owner_is_the_same_person_in_every_stream(emitted: dict[str, Table]) -> None:
    """Deals and engagements are attributed by hubspot_owner_id, so an owner
    that disagreed with the directory would strand them."""
    roster = build_roster(DEV_EMAIL)

    crm.seed_owners(None, roster, TENANT)
    crm.seed_deals(None, roster, TENANT, DAYS)
    crm.seed_engagements(None, roster, TENANT, DAYS)

    directory = set(_column(emitted["bronze_hubspot.owners"], "id"))
    assert directory, "the owner directory is empty"

    attributed = set(_column(emitted["bronze_hubspot.deals"], "properties_hubspot_owner_id"))
    for stream in crm._CRM_ACTIVITY_STREAMS:
        attributed |= set(
            _column(emitted[f"bronze_hubspot.{stream}"], "properties_hubspot_owner_id")
        )

    assert attributed <= directory, (
        f"these owners are in no directory row: {sorted(attributed - directory)}"
    )


def test_a_deal_is_closed_only_once_its_close_date_has_passed(emitted: dict[str, Table]) -> None:
    """The flags are strings in bronze because HubSpot sends them so; the
    staging model compares them to 'true'."""
    roster = build_roster(DEV_EMAIL)

    crm.seed_deals(None, roster, TENANT, DAYS)

    table = emitted["bronze_hubspot.deals"]
    assert table[1], "the pinned window drew no deals, so this proves nothing"
    for close, is_closed, is_won in zip(
        _column(table, "properties_closedate"),
        _column(table, "properties_hs_is_closed"),
        _column(table, "properties_hs_is_closed_won"),
        strict=True,
    ):
        expected = "true" if _dt.date.fromisoformat(close) < ANCHOR else "false"
        assert is_closed == expected, f"a deal closing {close} reports is_closed={is_closed}"
        if is_won == "true":
            assert is_closed == "true", "a deal is won without being closed"
