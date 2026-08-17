"""Journey — browsing the product becomes the numbers on Manage / Platform usage.

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

Measured on the compose stand while writing this: two real sign-ins, one lead and
one admin operator, showed up unprompted on the page as `9 visits / 67 pages` and
`5 / 27`. That is the loop under test.

**What the sweep covers.** Every screen the rail and the pane offer the persona,
walked by clicking, with `showPlanned` pinned off so what renders is the built
set (`ui/conftest.py`). It is deliberately not every URL that exists: scaffolded
entries are excluded because they render "Not built yet", the pre-portal routes
(`/metrics`, `/whats-new`, `/queries`) are outside the shell, and other people's
views reduce to the same recorded `/ic/:id/…` path as the first.

**The assertions are subsets, not equalities.** Usage events are append-only, the
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
from playwright.sync_api import Browser, Page, expect

from .conftest import apply_portal_prefs
from .flows import collect_page_rows, collect_rows, sign_in, sweep_portal
from .pages.platform_usage_page import PAGES_TABLE, PEOPLE_TABLE, PlatformUsagePage
from .pages.portal_shell import PortalShell

#: Column order of "Who opened it", as the page renders it.
PERSON, VISITS, PAGES = 0, 1, 2


@dataclass(frozen=True)
class Swept:
    """One sweep, plus what the admin's page said about it afterwards."""

    screens: list[str]
    sweeper: str
    admin: str
    people: list[list[str]]
    #: "What they opened", as (the name shown, the path recorded) per row.
    opened: list[tuple[str, str]]
    page: Page

    @property
    def paths(self) -> list[str]:
        return [path for _, path in self.opened]


@pytest.fixture(scope="module")
def swept(
    browser: Browser,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> Swept:
    """Sweep as a lead, then read the page as the admin operator.

    Module-scoped because the sweep is the expensive half and every assertion
    below is about the same one: two sign-ins and one walk, not two per test.
    Both halves need their own context — one browser profile cannot hold two
    sessions, and the SDK keys its session storage by person id, which is the
    thing `test_two_readers_do_not_merge_into_one_visit` checks.

    Polled before it reads: the insert is batched server-side (`async_insert`),
    so "swept" and "visible" are two moments and a bare read would be a flake
    waiting for a slow flush.
    """
    lead = session_for("dev_lead")
    operator = session_for("admin_operator")

    admin_context = browser.new_context(base_url=base_url)
    apply_portal_prefs(admin_context)
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

    lead_context = browser.new_context(base_url=base_url)
    apply_portal_prefs(lead_context)
    lead_page = lead_context.new_page()
    sign_in(lead_page, base_url, lead)
    screens = sweep_portal(lead_page)
    assert screens, "the sweep opened nothing — every assertion below would be vacuous"

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
    return Swept(
        screens=screens,
        sweeper=lead.person.display_name,
        admin=operator.person.display_name,
        people=collect_rows(usage.table(PEOPLE_TABLE)),
        opened=collect_page_rows(usage),
        page=admin_page,
    )


def _pages_recorded_for(usage: PlatformUsagePage, display_name: str) -> int:
    """That person's Pages figure, re-read from the page. 0 when absent yet."""
    usage.page.reload(wait_until="domcontentloaded")
    expect(usage.chart_heading()).to_be_visible()
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


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.reliability
def test_every_screen_this_run_opened_is_named(swept: Swept) -> None:
    """A reader sees screen names, not paths.

    Scoped to this run's screens on purpose: an older row from a retired route
    may legitimately have no name left, and failing on that would blame this
    build for someone else's history.
    """
    unnamed = [
        (label, path)
        for label, path in swept.opened
        if path in swept.screens and label.startswith("/")
    ]
    assert not unnamed, f"screens listed by raw path rather than name: {unnamed}"


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.security
def test_a_person_page_is_recorded_without_the_person(swept: Swept, base_url: str) -> None:
    """Adoption counting must not become a record of who read whose profile.

    The sweep opens the person zones, so the run always has person screens to
    check — asserted rather than assumed, because a rail that stopped offering
    them would make the rest of this test vacuous.
    """
    person_screens = [screen for screen in swept.screens if "/ic/" in screen]
    assert person_screens, "the sweep opened no person screen, so nothing here is proven"
    assert all(screen.startswith("/ic/:id/") for screen in person_screens), (
        f"a person key survived into the recorded path: {person_screens}"
    )
    assert swept.sweeper not in [label for label, _ in swept.opened]


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.reliability
def test_the_sweeper_is_named_with_their_visit_counted(swept: Swept) -> None:
    """The visitor is a name from the identity rows, not a bare uuid.

    This is the only test of the `identity.identity_persons` join the summary
    does; a broken join leaves the column showing person ids, which reads as
    "somebody" to the admin the feature exists for.
    """
    rows = {row[PERSON]: row for row in swept.people}
    assert swept.sweeper in rows, f"the sweeper is absent from '{PEOPLE_TABLE}': {sorted(rows)}"
    assert int(rows[swept.sweeper][VISITS]) >= 1
    assert int(rows[swept.sweeper][PAGES]) >= len(swept.screens)


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.reliability
def test_two_readers_do_not_merge_into_one_visit(swept: Swept) -> None:
    """Two people, two rows — the SDK keys its session storage by person id.

    A shared session id would fold both into one visit and one visitor, which is
    only observable in a browser: the storage the key protects is the browser's.
    """
    names = [row[PERSON] for row in swept.people]
    assert swept.sweeper in names and swept.admin in names, (
        f"expected both readers in '{PEOPLE_TABLE}', got {names}"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.security
def test_the_page_is_absent_from_a_non_admin_shell(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """Hiding the entry is a courtesy; refusing the deep link is the boundary.

    Both halves live here because they are different code — the pane filters on
    `adminOnly`, the view renders behind `AdminGate` — and a build can regress
    either one alone. The API suite owns the 403; what a browser adds is that a
    non-admin who addresses the url anyway is told no instead of shown numbers.
    """
    sign_in(page, base_url, session_for("dev_lead"))
    portal = PortalShell(page)
    portal.go()
    portal.rail.open_zone("Manage")
    expect(portal.pane.items().first).to_be_visible()
    assert PlatformUsagePage.ITEM not in portal.pane.item_labels()

    usage = PlatformUsagePage(page)
    usage.go()
    expect(page.get_by_text("admin surface")).to_be_visible()
    expect(usage.table(PEOPLE_TABLE)).not_to_be_visible()
    expect(usage.kpi_figure("visits")).not_to_be_visible()
