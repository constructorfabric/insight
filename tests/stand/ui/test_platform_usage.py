"""Journey — every screen a reader opens shows up on Manage / Platform usage.

Why this is a browser test and not an API test, measured rather than asserted:
**the browser is the only writer this feature has in production.** `session_start`
comes from the frontx SDK inside the page (`grep session_start src/backend` finds
only the analytics reader's constant; the string lives in
`@gears-frontx/telemetry/dist/index.js`), and page views come from
`recordPageView` on a history subscription in `main.tsx`. Nothing server-side
records a visit when a session is minted. So `tests/stand/api`'s coverage — which
POSTs a hand-built SDK body and reads it back — proves the endpoint and the read
model, and cannot prove that the shipped SPA emits anything at all. A build whose
collector never starts passes every API test in this repository.

**What the sweep covers.** Every screen the rail and the pane offer the persona,
walked by clicking, with `showPlanned` pinned off so what renders is the built
set. It is deliberately not every URL that exists: scaffolded entries are
excluded because they render "Not built yet", the pre-portal routes (`/metrics`,
`/whats-new`, `/queries`) are outside the shell, and other people's views reduce
to the same recorded `/ic/:id/…` path as the first.

**The assertion is a subset, not an equality.** Usage events are append-only, the
stand is shared, and no operation deletes them, so the page legitimately lists
screens from earlier runs and other people. What this journey may claim is that
everything IT opened is there — which is the regression that matters: a screen
that stops emitting, or a route table that drifts.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

import pytest
from insight_stand import PersonaSession, wait_until
from playwright.sync_api import Browser, BrowserContext, expect

from .flows import collect_page_rows, collect_rows, revisit_usage, sign_in, sweep_portal
from .pages.platform_usage_page import PAGES_TABLE, PEOPLE_TABLE, PlatformUsagePage

#: Column order of "Who opened it", as the page renders it.
PERSON, VISITS, PAGES = 0, 1, 2

#: The pane must list only what renders, or the sweep would click scaffolds. The
#: preference defaults to ON (`readBoolPref` returns true for anything that is not
#: literally `"false"`), which lists the not-yet-built zones and entries beside the
#: real ones — measured: the rail offers Scorecard, whose zone carries
#: `readiness: "unbuilt"`, until this is set.
SHOW_PLANNED_OFF = "window.localStorage.setItem('insight.portal.showPlanned', 'false')"


@dataclass(frozen=True)
class Swept:
    """One sweep, and what the admin's page said about it afterwards."""

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

    Two contexts, because one browser profile cannot hold two sessions and the
    admin is the only persona the summary is served to.

    Polled before it reads: the insert is batched server-side (`async_insert`), so
    "swept" and "visible" are two moments and a bare read would be a flake waiting
    for a slow flush.
    """
    lead = session_for("dev_lead")
    operator = session_for("admin_operator")

    admin_context = _portal_context(browser, base_url)
    admin_page = admin_context.new_page()
    sign_in(admin_page, base_url, operator)
    usage = PlatformUsagePage(admin_page)
    usage.go()
    expect(usage.chart_heading()).to_be_visible()
    # Read the sweeper's figure BEFORE the sweep. The table is append-only and the
    # stand is shared, so "pages >= what this run opened" is satisfied by any
    # earlier run's rows and would let the wait through before a single beacon of
    # this one had landed.
    before = _pages_recorded_for(usage, lead.person.display_name)

    lead_context = _portal_context(browser, base_url)
    lead_page = lead_context.new_page()
    sign_in(lead_page, base_url, lead)
    screens = sweep_portal(lead_page)
    assert screens, "the sweep opened nothing — the assertion below would be vacuous"

    # The sweeper's tab stays open until its events are in, and that is not
    # politeness. The SDK flushes on a `FLUSH_DELAY_MS = 5e3` timer — five
    # seconds — and nothing re-sends on unload, so a context closed on the last
    # click loses everything queued since the previous flush. Measured that way
    # first: a sweep of 27 screens landed 22 of them, and the five missing were
    # the ones clicked in the final seconds.
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

    Re-read by LEAVING the screen and coming back, not by reloading the document.
    Two reasons, and the second one bit: a client-side round trip is what a reader
    actually does, and it exercises the query's `refetchOnMount: "always"`; a
    reload re-boots the SPA and re-asks `/auth/me`, and a poll that does that
    every 500ms walks straight into the gateway's `auth_per_ip` limiter — which
    answers 503 and reads as a broken backend (measured: `excess: 120` in the
    gateway log, and every sign-in on the stand refused for minutes afterwards).
    """
    revisit_usage(usage.page)
    for row in collect_rows(usage.table(PEOPLE_TABLE)):
        if row[PERSON] == display_name:
            return int(row[PAGES])
    return 0


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.reliability
def test_every_screen_the_sweep_opened_is_listed(swept: Swept) -> None:
    """The loop, end to end: what a real browser opened is what the page lists.

    Fails when a screen stops emitting, when the collector never starts, and when
    the route table grows an entry the recorder does not see — none of which any
    cheaper suite can observe, because the emitter is the browser.
    """
    missing = [screen for screen in swept.screens if screen not in swept.paths]
    assert not missing, (
        f"{len(missing)} of the {len(swept.screens)} screens opened are absent from "
        f"'{PAGES_TABLE}': {missing}. Listed: {sorted(set(swept.paths))}"
    )
