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
from .pages.portal_shell import ContextPane, PortalShell


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

    A zone click lands on its default item; person zones leave `/portal` for
    `/ic/<uuid>/…`.
    """
    portal = PortalShell(page)
    portal.go()
    portal.wait_url_settled()
    # `all_inner_texts()` reads what is mounted now, so gate set reads on visibility.
    expect(portal.rail.zones().first).to_be_visible()
    seen: list[str] = []
    for zone in [label.strip() for label in portal.rail.zones().all_inner_texts()]:
        portal.rail.open_zone(zone)
        portal.wait_url_settled()
        _record(portal, seen)
        expect(portal.pane.views().first).to_be_visible()
        _walk_pane(portal, seen)
    return seen


#: Shorter than the 30s default so an entry re-rendered away fails in seconds.
_ITEM_CLICK_MS = 20_000


def _walk_pane(portal: PortalShell, seen: list[str]) -> None:
    """Open each of this zone's views, re-reading the pane between clicks.

    A pane re-render detaches the button a held label points at.
    """
    opened: set[str] = set()
    while True:
        portal.pane.wait_settled()
        remaining = [label for label in portal.pane.view_labels() if label not in opened]
        if not remaining:
            return
        label = remaining[0]
        opened.add(label)
        if portal.pane.item(label).count() == 0:
            continue
        portal.pane.open_item(label, timeout_ms=_ITEM_CLICK_MS)
        _record(portal, seen)


def _record(portal: PortalShell, seen: list[str]) -> None:
    expect(portal.content()).to_be_visible()
    recorded = portal.recorded_path()
    if recorded not in seen:
        seen.append(recorded)


def revisit_usage(page: Page) -> None:
    """Leave the usage screen and come back, the way a reader does.

    A client-side remount is what fires `refetchOnMount: "always"` in
    queries/usage.ts.
    """
    pane = ContextPane(page)
    usage = PlatformUsagePage(page)
    expect(pane.views().first).to_be_visible()
    labels = pane.view_labels()
    elsewhere = next((label for label in labels if label != PlatformUsagePage.ITEM), None)
    assert elsewhere, (
        f"the pane offers nothing beside {PlatformUsagePage.ITEM!r} to leave for: {labels}"
    )
    pane.open_item(elsewhere)
    expect(usage.chart_heading()).not_to_be_visible()
    pane.open_item(PlatformUsagePage.ITEM)
    expect(usage.chart_heading()).to_be_visible()


_INDICES_AT_THIS_SCROLL = "rows => rows.map(row => row.dataset.index)"


def collect_rows(table: Locator) -> list[list[str]]:
    def read(index: int) -> list[str]:
        cells = table.locator(f'{TABLE_ROW}[data-index="{index}"] [data-slot="table-cell"]')
        return [text.strip() for text in cells.all_inner_texts()]

    return _scrolled(table, read)


def collect_page_rows(usage: PlatformUsagePage) -> list[tuple[str, str]]:
    """Every row of "What they opened" as (the name shown, the path recorded).

    The path exists only in a hover tooltip.
    """

    def read(index: int) -> tuple[str, str]:
        # Two tooltips can be mounted at once; wait for zero before the next hover.
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

    `max-h-90` with `overscan: 8` keeps about nine rows in the DOM. Two quiet
    passes end the walk because scrolling and re-rendering are separate ticks.
    """
    seen: dict[int, T] = {}
    body = table.locator(f"{TABLE_ROW}[data-index]")
    # INVARIANT: `evaluate_all` never waits, so an ungated pass ends the walk empty.
    expect(body.first).to_be_visible()
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
