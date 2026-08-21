"""Journey — every screen a reader opens shows up on Manage / Platform usage.

The browser is the only writer this feature has: `session_start` comes from the
frontx SDK in the page and page views from `recordPageView` in `main.tsx`, so no
API test can prove the shipped SPA emits anything. Usage events are append-only
on a shared stand, so the assertion is a subset, not an equality.
"""

from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime

import pytest
from insight_stand import PersonaSession, analytics_path, wait_until
from playwright.sync_api import Browser, BrowserContext, expect

from .flows import (
    _scrolled,
    collect_page_rows,
    collect_rows,
    revisit_usage,
    sign_in,
    sweep_portal,
)
from .pages.platform_usage_page import PAGES_TABLE, PEOPLE_TABLE, PlatformUsagePage
from .pages.portal_shell import PortalShell

PERSON, VISITS, PAGES = 0, 1, 2

#: `readBoolPref` treats anything but `"false"` as true, so unbuilt zones render.
SHOW_PLANNED_OFF = "window.localStorage.setItem('insight.portal.showPlanned', 'false')"


@dataclass(frozen=True)
class Swept:
    screens: list[str]
    #: "What they opened", as (the name shown, the path recorded) per row.
    opened: list[tuple[str, str]]

    @property
    def paths(self) -> list[str]:
        return [path for _, path in self.opened]


def _portal_context(browser: Browser, base_url: str) -> BrowserContext:
    context = browser.new_context(base_url=base_url)
    context.add_init_script(SHOW_PLANNED_OFF)
    return context


@pytest.fixture(scope="module")
def swept(
    browser: Browser,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> Swept:
    """Sweep as a lead, then read the page as the admin operator.

    Two contexts: one browser profile holds one session, and only the admin is
    served the summary. The server insert is batched (`async_insert`), so the read
    is polled.
    """
    lead = session_for("dev_lead")
    operator = session_for("admin_operator")

    admin_context = _portal_context(browser, base_url)
    admin_page = admin_context.new_page()
    sign_in(admin_page, base_url, operator)
    usage = PlatformUsagePage(admin_page)
    usage.go()
    expect(usage.chart_heading()).to_be_visible()
    # Baseline before the sweep: the table is append-only and the stand is shared.
    before = _pages_recorded_for(usage, lead.person.display_name)

    lead_context = _portal_context(browser, base_url)
    lead_page = lead_context.new_page()
    sign_in(lead_page, base_url, lead)
    screens = sweep_portal(lead_page)
    assert screens, "the sweep opened nothing — the assertion below would be vacuous"

    # The SDK flushes on a 5s timer and nothing re-sends on unload, so the
    # sweeper's context stays open until its events land.
    wait_until(
        lambda: _pages_recorded_for(usage, lead.person.display_name) >= before + len(screens),
        timeout_s=45,
        description=(
            f"the {len(screens)} screens {lead.person.display_name} opened to reach the summary"
        ),
    )
    lead_context.close()

    return Swept(screens=screens, opened=collect_page_rows(usage))


def _pages_recorded_for(usage: PlatformUsagePage, display_name: str) -> int:
    """That person's Pages figure, re-read from the page. 0 when absent yet.

    A remount, not a reload: reloading re-asks `/auth/me` and a poll trips the
    gateway's `auth_per_ip` limiter.
    """
    revisit_usage(usage.page)
    for row in collect_rows(usage.table(PEOPLE_TABLE)):
        if row[PERSON] == display_name:
            return int(row[PAGES])
    return 0


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.reliability
def test_every_screen_the_sweep_opened_is_listed(swept: Swept) -> None:
    missing = [screen for screen in swept.screens if screen not in swept.paths]
    assert not missing, (
        f"{len(missing)} of the {len(swept.screens)} screens opened are absent from "
        f"'{PAGES_TABLE}': {missing}. Listed: {sorted(set(swept.paths))}"
    )


#: A person, in any shape the SPA could let through: a uuid, an email, or the
#: long numeric key a source system uses. `screenPath()` reduces all three to
#: `:id` before the beacon leaves the browser, so none may appear in a report.
PERSON_SHAPED = re.compile(
    r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|@|%40|/\d{6,}(?:/|$)",
    re.IGNORECASE,
)


def _views_by_path(usage: PlatformUsagePage) -> dict[str, int]:
    """Every row of "What they opened" as path → views, freshly re-read.

    `collect_page_rows` reads the name and the path; this needs the count beside
    them, and reading the table twice would let it re-render between the walks.

    Remounts first, like `_pages_recorded_for`: without it every poll re-reads
    the DOM the first fetch produced and a wait for a rising count never ends.
    """
    revisit_usage(usage.page)

    def read(index: int) -> tuple[str, int]:
        usage.header(PAGES_TABLE).hover()
        expect(usage.tooltips()).to_have_count(0)
        row = usage.row_at(PAGES_TABLE, index)
        cells = row.locator('[data-slot="table-cell"]').all_inner_texts()
        usage.page_label(row).hover()
        expect(usage.tooltips()).to_have_count(1)
        return usage.tooltips().inner_text().strip(), int(cells[1].strip())

    return dict(_scrolled(usage.table(PAGES_TABLE), read))


def _views_reported(operator: PersonaSession) -> dict[str, int]:
    """Today's Views per screen as the SERVICE reports them, in one request.

    Only a readiness signal — the batched insert makes "accepted" and "visible"
    two moments, and the assertions this waits for are the rendered ones.
    """
    day = datetime.now(UTC).date().isoformat()
    response = operator.client.get(
        analytics_path("/v1/usage/summary"), params={"since": day, "until": day}
    )
    assert response.status_code == 200, response.text[:300]
    return {page["path"]: int(page["views"]) for page in response.json().get("by_page", [])}


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.security
def test_no_recorded_screen_names_the_person_it_was_about(swept: Swept) -> None:
    """Nothing in the report may say WHO a screen was about.

    The journey above proves the person screens the sweep opened arrive masked.
    This is the other half, and the one a masking bug would actually show up in:
    that no row anywhere in the report carries a person — an extra unmasked row
    riding alongside the masked one passes every assertion that only checks the
    masked one is present.

    Read over the whole table, not just this run's rows, because the table is
    append-only: a build that recorded a person once leaves the evidence behind
    for every later reader, which is exactly the harm.
    """
    named = sorted({path for path in swept.paths if PERSON_SHAPED.search(path)})
    assert not named, f"{len(named)} recorded screens name the person they were about: {named[:10]}"


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.reliability
def test_the_count_beside_a_screen_rises_once_per_opening(
    browser: Browser,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """ "How many times" is half of what the pages table answers, and untested.

    The journey above reads each row's name and recorded path, so a screen
    listed at the wrong number passes it. The count is what makes the table a
    usage report rather than a list of screens that exist.

    Two zones opened alternately rather than one opened repeatedly: the recorder
    drops a screen identical to the one before it, on purpose, so a straight
    reload loop would record one opening and prove nothing. Alternating gives
    two independent deltas from one walk — 3 and 2 — and a recorder that counted
    page loads instead of screen changes would produce neither.
    """
    lead = session_for("dev_lead")
    operator = session_for("admin_operator")

    admin_context = _portal_context(browser, base_url)
    admin_page = admin_context.new_page()
    sign_in(admin_page, base_url, operator)
    usage = PlatformUsagePage(admin_page)
    usage.go()
    expect(usage.chart_heading()).to_be_visible()
    before = _views_by_path(usage)
    reported_before = _views_reported(operator)

    lead_context = _portal_context(browser, base_url)
    lead_page = lead_context.new_page()
    sign_in(lead_page, base_url, lead)
    portal = PortalShell(lead_page)
    portal.go()
    portal.wait_url_settled()
    expect(portal.rail.zones().first).to_be_visible()

    zones = [label.strip() for label in portal.rail.zones().all_inner_texts()]
    assert len(zones) >= 2, f"the rail offers {zones}, so there is nothing to alternate"
    first, second = zones[0], zones[1]

    opened: list[str] = []
    for zone in (first, second, first, second, first):
        portal.rail.open_zone(zone)
        portal.wait_url_settled()
        opened.append(portal.recorded_path())

    expected = {path: opened.count(path) for path in set(opened)}
    assert sorted(expected.values()) == [2, 3], (
        f"the walk did not open two distinct screens 3 and 2 times: {expected}"
    )
    busiest = max(expected, key=lambda path: expected[path])

    # Waited for over the API, not the table: re-reading a virtualized table
    # costs a remount and a hover per row, so polling it would spend the whole
    # window on three samples. The assertion below is still the rendered one.
    wait_until(
        lambda: _views_reported(operator).get(busiest, 0) >= reported_before.get(busiest, 0) + 3,
        timeout_s=60,
        description=f"the three openings of {busiest} to reach the summary",
    )
    lead_context.close()

    after = _views_by_path(usage)
    for path, openings in expected.items():
        rose = after.get(path, 0) - before.get(path, 0)
        assert rose == openings, (
            f"'{path}' was opened {openings} times and its Views figure rose by {rose}"
        )
