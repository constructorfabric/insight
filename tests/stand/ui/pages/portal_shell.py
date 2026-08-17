"""The portal shell — the lens rail, the contextual pane, and the screens they reach.

Layout, measured on a mock-mode build of `src/frontend` at this commit:

    [ lens rail ] [ zone-contextual pane ] [ content ]

Both halves are shadcn `Sidebar`s, so their entries are indistinguishable by
role: every rail zone and every pane item is a `button` whose accessible name is
its label, and both sit inside `[data-slot="sidebar-menu-button"]`. What tells
them apart is the container. Measured ancestor chains:

    rail zone  menu-button < menu-item < menu < sidebar-content < sidebar < [data-testid="lens-rail"]
    pane item  menu-button < menu-item < menu < sidebar-group-content < SIDEBAR-GROUP < sidebar-content < sidebar

So the rail is addressed by its own testid, and a pane item by the
`sidebar-group` no rail entry has. `data-slot` is the permitted structural
attribute here (the component library emits it and it survives restyles); the
rule this suite bans is hashed classes and Tailwind utilities.

**The rail widens on hover, and the words are the hit area.** A rail entry is
inside the hover target, so `hover_rail()` comes before a real click — a click
dispatched at a collapsed rail lands on the icon strip, which is what the
component's own vitest tests hover before clicking too.

**A screen is a URL, and the URL shape is not one path per screen.** Portal
screens differ only by the `zone` and `item` search params, which is why
`main.tsx` subscribes to history to record them. Measured, with the mock
viewer's uuid abbreviated:

    zone Overview      /portal?zone=overview
    zone Directions    /portal?zone=directions
    zone Person        /ic/<uuid>/personal
    zone People        /ic/<uuid>/team?scope=<uuid>
    zone AI & Cost     /portal?scope=<uuid>&zone=aicost
    zone Reports       /portal?scope=<uuid>&zone=reports
    zone Manage        /portal?zone=manage
    item Trend         /portal?scope=<uuid>&zone=overview&item=trend
    item Git output    /ic/<uuid>/personal?scope=<uuid>&item=git_output
    item Platform usage  /portal?scope=<uuid>&zone=manage&item=platform-usage

`screen_of()` reduces one of those to the string the product records, and
`recorded_path_of()` strips it the way the SPA does before it leaves the browser.
Both live here because both are properties of the portal's URLs; a journey that
sweeps navigation reads them off `page.url` rather than predicting them.
"""

from __future__ import annotations

import re
from urllib.parse import parse_qs, urlsplit

from playwright.sync_api import Locator, Page

RAIL = '[data-testid="lens-rail"]'

MENU_BUTTON = '[data-slot="sidebar-menu-button"]'

PANE_ITEM = f'[data-slot="sidebar-group"] {MENU_BUTTON}'

#: The person key in a `/ic/<key>/…` path, whatever shape it arrives in — the SPA
#: strips the segment after `/ic` by position, so matching on shape alone here
#: would let an email through and call it anonymous.
_UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", re.I)

_LONG_NUMBER = re.compile(r"^\d{6,}$")


class LensRail:
    """The zone rail. One entry per zone the viewer's shape and role allow."""

    def __init__(self, page: Page) -> None:
        self.page = page
        self.rail = page.locator(RAIL)

    def hover(self) -> None:
        self.rail.hover()

    def zone(self, label: str) -> Locator:
        return self.rail.locator(MENU_BUTTON).filter(has_text=re.compile(rf"^{re.escape(label)}$"))

    def zones(self) -> Locator:
        return self.rail.locator(MENU_BUTTON)

    def settings(self) -> Locator:
        return self.rail.get_by_role("button", name="Settings")

    def open_zone(self, label: str) -> None:
        """Hover, then click — the two steps a person performs to reach a zone."""
        self.hover()
        self.zone(label).click()


class ContextPane:
    """The zone-contextual pane. Its entries are the screens inside a zone.

    With `insight.portal.showPlanned` pinned off (see `ui/conftest.py`) the
    entries it renders are exactly the built ones, so `item_labels()` is a
    denominator a journey can trust rather than a list to maintain.
    """

    def __init__(self, page: Page) -> None:
        self.page = page

    def items(self) -> Locator:
        return self.page.locator(PANE_ITEM)

    def item(self, label: str) -> Locator:
        return self.items().filter(has_text=re.compile(rf"^{re.escape(label)}$"))

    def item_labels(self) -> list[str]:
        return [text.strip() for text in self.items().all_inner_texts()]

    def open_item(self, label: str) -> None:
        self.item(label).click()


class PortalShell:
    """The portal as a whole: where it lands, and the two navigations into it."""

    PATH = "/portal"

    def __init__(self, page: Page) -> None:
        self.page = page
        self.rail = LensRail(page)
        self.pane = ContextPane(page)

    def go(self) -> None:
        self.page.goto(self.PATH, wait_until="domcontentloaded")

    def screen(self) -> str:
        return screen_of(self.page.url)

    def recorded_path(self) -> str:
        return recorded_path_of(self.page.url)


def screen_of(url: str) -> str:
    """The string the product records for a URL — `currentScreen()` in `main.tsx`.

    Only `zone` and `item` take part: every other search param (`scope`,
    `period`, `filter`) is absent from the recorded screen, so two readers with
    different scopes on the same view count as one screen rather than two.
    """
    parts = urlsplit(url)
    query = parse_qs(parts.query)
    named = [values[0] for key in ("zone", "item") if (values := query.get(key)) and values[0]]
    return f"{parts.path}/{'/'.join(named)}" if named else parts.path


def recorded_path_of(url: str) -> str:
    """`screen_of()` with the person stripped — `screenPath()` in `telemetry.ts`.

    Adoption counting must not become a record of who read whose profile, so the
    segment after `/ic` and any identifier-shaped segment become `:id` before the
    beacon leaves the browser. A journey asserts against this, never against the
    raw url, because the raw url is what must NOT be recorded.
    """
    segments = screen_of(url).split("/")
    return "/".join(
        ":id" if _is_person_key(segments, index) else segment
        for index, segment in enumerate(segments)
    )


def _is_person_key(segments: list[str], index: int) -> bool:
    if index > 0 and segments[index - 1] == "ic":
        return True
    segment = segments[index]
    return bool(_UUID.match(segment) or _LONG_NUMBER.match(segment))
