"""Journey — every screen a reader opens shows up on Manage / Platform usage.

The browser is the only writer this feature has: `session_start` comes from the
frontx SDK in the page and page views from `recordPageView` in `main.tsx`, so no
API test can prove the shipped SPA emits anything. Usage events are append-only
on a shared stand, so every assertion here is a delta around one sweep, never an
equality on what the tables hold.

Three promises share that one sweep (#2573 scenarios 1, 3 and 4). A browser
session is the most expensive thing this suite spends, so the fixture pays for
it once and each test states one promise against it.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

import pytest
from insight_stand import PersonaSession, wait_until
from playwright.sync_api import Browser, BrowserContext, expect

from .flows import collect_page_rows, collect_rows, revisit_usage, sign_in, sweep_portal
from .pages.platform_usage_page import PAGES_TABLE, PEOPLE_TABLE, PlatformUsagePage

PERSON, VISITS, PAGES = 0, 1, 2

#: The sweep must not walk unbuilt zones, whatever the app default is.
SHOW_PLANNED_OFF = "window.localStorage.setItem('insight.portal.showPlanned', 'false')"

#: The admin's own re-reading of this page records screens for the whole run, and
#: all of them are in the zone a lead is never offered.
ADMIN_ZONE = "/portal/manage"

#: A cold sign-in records these before the sweep begins.
COLD_START = ("/", "/portal")

#: Opening a zone can compose a deeper screen name than the sweep reports for
#: that navigation — a direction opens with a lens (#2648) — so a swept screen
#: stands for everything recorded beneath it.


@dataclass(frozen=True)
class Swept:
    screens: list[str]
    #: "What they opened", as (the name shown, the path recorded, the views) per row.
    opened: list[tuple[str, str, int]]
    #: The sweeper's own Visits and Pages figures, before and after.
    visits: tuple[int, int]
    pages: tuple[int, int]
    #: Views per recorded path, before and after.
    views_before: dict[str, int]
    views_after: dict[str, int]

    @property
    def paths(self) -> list[str]:
        return [path for _, path, _ in self.opened]


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
    # Baseline before the sweep: the tables are append-only and the stand is shared.
    before_visits, before_pages = _figures_for(usage, lead.person.display_name)
    views_before = _views_by_path(usage)

    lead_context = _portal_context(browser, base_url)
    lead_page = lead_context.new_page()
    sign_in(lead_page, base_url, lead)
    screens = sweep_portal(lead_page)
    assert screens, "the sweep opened nothing — the assertions below would be vacuous"

    # The SDK flushes on a 5s timer and nothing re-sends on unload, so the
    # sweeper's context stays open until its events land.
    wait_until(
        lambda: _pages_recorded_for(usage, lead.person.display_name) >= before_pages + len(screens),
        timeout_s=45,
        description=(
            f"the {len(screens)} screens {lead.person.display_name} opened to reach the summary"
        ),
    )
    lead_context.close()

    after_visits, after_pages = _figures_for(usage, lead.person.display_name)
    # One read, two uses: a second pass over this virtualized table starts from
    # wherever the first left it scrolled and comes back short.
    opened = collect_page_rows(usage)
    return Swept(
        screens=screens,
        opened=opened,
        visits=(before_visits, after_visits),
        pages=(before_pages, after_pages),
        views_before=views_before,
        views_after={path: views for _, path, views in opened},
    )


def _people_row(usage: PlatformUsagePage, display_name: str) -> list[str] | None:
    for row in collect_rows(usage.table(PEOPLE_TABLE)):
        if row[PERSON] == display_name:
            return row
    return None


def _figures_for(usage: PlatformUsagePage, display_name: str) -> tuple[int, int]:
    """That person's (Visits, Pages), as the page shows them now. (0, 0) when absent."""
    row = _people_row(usage, display_name)
    return (int(row[VISITS]), int(row[PAGES])) if row else (0, 0)


def _views_by_path(usage: PlatformUsagePage) -> dict[str, int]:
    return {path: views for _, path, views in collect_page_rows(usage)}


def _pages_recorded_for(usage: PlatformUsagePage, display_name: str) -> int:
    """That person's Pages figure, re-read from the page. 0 when absent yet.

    A remount, not a reload: reloading re-asks `/auth/me` and a poll trips the
    gateway's `auth_per_ip` limiter.
    """
    revisit_usage(usage.page)
    return _figures_for(usage, display_name)[1]


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.stand_smoke
@pytest.mark.reliability
def test_every_screen_the_sweep_opened_is_listed(swept: Swept) -> None:
    """#2573 scenario 3 — every screen a reader opened reaches the report by name.

    Marked `stand_smoke`: this is the post-deploy gate's "the shipped SPA still
    reports what it opened", the one claim no API test can make for it.
    """
    missing = [screen for screen in swept.screens if screen not in swept.paths]
    assert not missing, (
        f"{len(missing)} of the {len(swept.screens)} screens opened are absent from "
        f"'{PAGES_TABLE}': {missing}. Listed: {sorted(set(swept.paths))}"
    )


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.stand_smoke
@pytest.mark.reliability
def test_the_sweep_counts_as_one_sitting(swept: Swept) -> None:
    """#2573 scenario 1 — a sitting is one visit however many screens it opens.

    Read off the sweeper's own row, so the admin's polling cannot confound it.
    Marked `stand_smoke`: it shares the gate's sweep and costs it no browser.
    """
    before_visits, after_visits = swept.visits
    before_pages, after_pages = swept.pages
    assert after_visits - before_visits == 1, (
        f"one uninterrupted sitting over {len(swept.screens)} screens counted as "
        f"{after_visits - before_visits} visits, not 1"
    )
    assert after_pages - before_pages >= len(swept.screens), (
        f"the Pages figure rose by {after_pages - before_pages} over a sweep that "
        f"opened {len(swept.screens)} screens"
    )


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.stand_smoke
@pytest.mark.reliability
def test_each_screen_carries_its_own_count(swept: Swept) -> None:
    """#2573 scenario 4 — the number beside a screen counts that screen alone.

    Both halves are needed: that every screen opened moved says the column
    counts something, and that a screen nobody opened did not move says it is
    not a figure the whole table shares.
    Marked `stand_smoke`: it shares the gate's sweep and costs it no browser.
    """
    stalled = [
        screen
        for screen in swept.screens
        if swept.views_after.get(screen, 0) <= swept.views_before.get(screen, 0)
    ]
    assert not stalled, (
        f"{len(stalled)} of the {len(swept.screens)} screens opened kept the same Views "
        f"figure across the sweep: "
        f"{ {s: (swept.views_before.get(s, 0), swept.views_after.get(s, 0)) for s in stalled} }"
    )

    unopened = [
        path
        for path in swept.views_before
        if not any(path.startswith(screen) for screen in swept.screens)
        and not path.startswith(ADMIN_ZONE)
        and path not in COLD_START
    ]
    if not unopened:
        pytest.skip("the sweep opened every screen already listed — nothing to compare against")
    moved = {
        path: (swept.views_before[path], swept.views_after.get(path, 0))
        for path in unopened
        if swept.views_after.get(path, 0) != swept.views_before[path]
    }
    assert not moved, (
        f"{len(moved)} screens nobody opened during the sweep changed their Views figure, "
        f"so the column is not counting per screen: {moved}"
    )
