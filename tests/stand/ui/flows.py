"""Multi-step browser actions that compose page objects.

Page objects answer "where is it" and nothing else — no assertions, no test data,
no branching. A sign-in is three of them in sequence, which belongs neither in a
page object nor in a test that is about something else.

Kept here rather than in a fixture on purpose. A `signed_in_page` fixture would
share one authenticated page across journeys, and each journey is a statement
about a complete round trip from a cold browser; sharing would make the later
ones depend on the earlier ones having run.


"""

from __future__ import annotations

from collections.abc import Callable

from insight_stand import PersonaSession
from playwright.sync_api import Locator, Page, expect

from .pages.keycloak_login_page import KeycloakLoginPage
from .pages.login_page import LoginPage
from .pages.platform_usage_page import PAGES_TABLE, TABLE_ROW, PlatformUsagePage
from .pages.portal_shell import PortalShell


def sign_in(page: Page, base_url: str, persona: PersonaSession) -> None:
    """Drive the deployed OIDC chain until the app renders for that persona.

    No shortcut at any step: an unauthenticated visit to `/` starts
    authorization-code+PKCE by itself, Keycloak serves its real form, and the
    authenticator sets `__Host-sid` at the callback. Nothing is minted.
    """
    LoginPage(page).go()
    KeycloakLoginPage(page).fill_and_submit(persona.email, persona.password)
    page.wait_for_url(f"{base_url}/**")


def sweep_portal(page: Page) -> list[str]:
    """Open every screen the rail and pane offer, and report what was recorded.

    The expectation is DERIVED, not declared: each hop reads `page.url` back and
    reduces it the way the SPA does, so the returned list is what this run
    actually opened. Two things that makes right, both measured on the compose
    stand — clicking a zone lands on its default item and records a screen nobody
    clicked, and the person zones navigate to `/ic/<uuid>/...` rather than staying
    under `/portal`.

    Only what renders is walked, and only the zone's own views: the caller's
    context pins `showPlanned` off, so a scaffolded entry is not in the DOM to
    click, and the org chart's people are anchors that replace the pane rather
    than screens of it (see `ContextPane.views`). Those people reduce to the same
    recorded `/ic/:id/...` path as the persona's own view, so skipping them costs
    no coverage.
    """
    portal = PortalShell(page)
    portal.go()
    # `all_inner_texts()` reports what is there NOW rather than waiting for it, so
    # a sweep that read the rail straight after `goto` enumerated an empty shell
    # and passed by finding nothing. Every read of a set is gated on its first
    # member being on screen.
    expect(portal.rail.zones().first).to_be_visible()
    seen: list[str] = []
    for zone in [label.strip() for label in portal.rail.zones().all_inner_texts()]:
        portal.rail.open_zone(zone)
        _record(portal, seen)
        expect(portal.pane.views().first).to_be_visible()
        for item in portal.pane.view_labels():
            portal.pane.open_item(item)
            _record(portal, seen)
    return seen


def _record(portal: PortalShell, seen: list[str]) -> None:
    """Wait for the screen to be on show, then keep what the SPA would record."""
    expect(portal.content()).to_be_visible()
    recorded = portal.recorded_path()
    if recorded not in seen:
        seen.append(recorded)


#: Which rows exist at the current scroll position, read in one evaluation.
#: Per-locator enumeration cannot do this: the virtualizer unmounts a row while
#: the previous read is still resolving, and a positional `nth(17).get_attribute`
#: then waits its whole timeout for an element that no longer exists — measured
#: exactly that way before this became a single call.
_INDICES_AT_THIS_SCROLL = "rows => rows.map(row => row.dataset.index)"


def collect_rows(table: Locator) -> list[list[str]]:
    """Every row of a virtualized table as its cell texts, in the table's order."""

    def read(index: int) -> list[str]:
        cells = table.locator(f'{TABLE_ROW}[data-index="{index}"] [data-slot="table-cell"]')
        return [text.strip() for text in cells.all_inner_texts()]

    return _scrolled(table, read)


def collect_page_rows(usage: PlatformUsagePage) -> list[tuple[str, str]]:
    """Every row of "What they opened" as (the name shown, the path recorded).

    The column shows `screenLabel()`'s name and keeps the path in a tooltip, so
    the pair only exists after a hover — which is also the only way a reader can
    see the path at all.
    """

    def read(index: int) -> tuple[str, str]:
        # Park the pointer on the header and let the previous tooltip go before
        # opening the next: two stay mounted otherwise, and the pair read back
        # would be one row's name beside another row's path.
        usage.header(PAGES_TABLE).hover()
        expect(usage.tooltips()).to_have_count(0)
        row = usage.row_at(PAGES_TABLE, index)
        label = usage.page_cell(row).inner_text().strip()
        usage.page_label(row).hover()
        expect(usage.tooltips()).to_have_count(1)
        return label, usage.tooltips().inner_text().strip()

    return _scrolled(usage.table(PAGES_TABLE), read)


def _scrolled[T](table: Locator, read: Callable[[int], T]) -> list[T]:
    """Read every row of a virtualized table, scrolling it into existence.

    Measured: the container is `max-h-90` with `overscan: 8`, so about nine rows
    exist at a time and the rest are not in the DOM at all. A journey that read
    the table as it stands would assert against the first screenful and call a
    truncated set complete.

    Rows are keyed by `data-index` — the virtualizer's own number, which is
    identity where position is not — so a row met at two scroll positions is read
    once. Two quiet passes end the walk rather than one: scrolling and
    re-rendering are separate ticks, so the pass straight after a scroll can
    still be looking at the rows from before it.
    """
    seen: dict[int, T] = {}
    body = table.locator(f"{TABLE_ROW}[data-index]")
    viewport = table.locator("xpath=..")
    stalled = 0
    while stalled < 2:
        fresh = [
            int(index)
            for index in body.evaluate_all(_INDICES_AT_THIS_SCROLL)
            if index is not None and int(index) not in seen
        ]
        for index in fresh:
            seen[index] = read(index)
        before = viewport.evaluate("el => el.scrollTop")
        viewport.evaluate("el => el.scrollBy(0, el.clientHeight)")
        moved = viewport.evaluate("el => el.scrollTop") != before
        stalled = 0 if (fresh or moved) else stalled + 1
    return [seen[key] for key in sorted(seen)]
