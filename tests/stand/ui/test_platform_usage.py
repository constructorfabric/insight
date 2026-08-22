"""Journey — every screen a reader opens shows up on Manage / Platform usage.

The browser is the only writer this feature has: `session_start` comes from the
frontx SDK in the page and page views from `recordPageView` in `main.tsx`, so no
API test can prove the shipped SPA emits anything. Usage events are append-only
on a shared stand, so the assertion is a subset, not an equality.
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
