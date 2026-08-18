"""The portal shell — the lens rail, the contextual pane, and the screens they reach.

Layout, measured on a stand serving the SPA from `src/frontend` at this commit:

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
inside the hover target, so `hover()` comes before a real click — a click
dispatched at a collapsed rail lands on the icon strip, which is what the
component's own vitest tests hover before clicking too.

**A screen is a URL, and the URL shape is not one path per screen.** Portal
screens differ only by the `zone` and `item` search params, which is why
`main.tsx` subscribes to history to record them. Measured, with the person's uuid
abbreviated:

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

`recorded_path_of()` reduces one of those to the string the product records and
strips the person out of it, the way the SPA does before the beacon leaves. It
lives here because it is a property of the portal's URLs; a journey that sweeps
navigation reads it off `page.url` rather than predicting it.
"""

from __future__ import annotations

import re
from urllib.parse import parse_qs, urlsplit

from playwright.sync_api import Locator, Page

RAIL = '[data-testid="lens-rail"]'

MENU_BUTTON = '[data-slot="sidebar-menu-button"]'

PANE_GROUP = '[data-slot="sidebar-group"]'

PANE_ITEM = f"{PANE_GROUP} {MENU_BUTTON}"

PANE_VIEW = f"{PANE_GROUP} button{MENU_BUTTON}"

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

    def open_zone(self, label: str) -> None:
        """Hover, then click — the two steps a person performs to reach a zone."""
        self.hover()
        self.zone(label).click()


class ContextPane:
    """The zone-contextual pane. Its entries are the screens inside a zone.

    With `insight.portal.showPlanned` pinned off by the caller's context, the
    entries it renders are exactly the built ones, so `view_labels()` is a
    denominator a journey can trust rather than a list to maintain.
    """

    def __init__(self, page: Page) -> None:
        self.page = page

    def items(self) -> Locator:
        return self.page.locator(PANE_ITEM)

    def views(self) -> Locator:
        """The zone's own screens, without the org chart's people.

        Measured in the People pane: its "Views" group renders `<button>` entries
        ("People (roster)", "Employees") while the "WorkChart" group renders `<a>`
        entries per person, which navigate to that person's own route and so
        replace the pane they were clicked from. Both carry `sidebar-menu-button`,
        and the element is what separates them.
        """
        return self.page.locator(PANE_VIEW)

    def item(self, label: str) -> Locator:
        return self.items().filter(has_text=re.compile(rf"^{re.escape(label)}$"))

    def view_labels(self) -> list[str]:
        return [text.strip() for text in self.views().all_inner_texts()]

    def open_item(self, label: str, timeout_ms: float | None = None) -> None:
        self.item(label).click(timeout=timeout_ms)

    def wait_settled(self, timeout_ms: float = 15_000) -> None:
        """Wait until the pane stops re-rendering, then let the caller click.

        Measured on a compose stand whose SPA is served by a dev server: a zone
        re-renders its pane as its data arrives, and each re-render detaches the
        button a click is aiming at — Playwright reports "element was detached
        from the DOM, retrying" until the whole timeout is gone, on an entry that
        is plainly on screen the entire time.

        A condition, not a sleep: the entries' text is sampled on `window` and
        settled means unchanged for a beat.
        """
        self.page.wait_for_function(
            """(selector) => {
                const signature = [...document.querySelectorAll(selector)]
                    .map((entry) => entry.textContent.trim())
                    .join('|');
                const now = performance.now();
                if (window.__paneSignature !== signature) {
                    window.__paneSignature = signature;
                    window.__paneSettledAt = now;
                    return false;
                }
                return now - window.__paneSettledAt > 250;
            }""",
            arg=PANE_ITEM,
            timeout=timeout_ms,
        )


class PortalShell:
    """The portal as a whole: where it lands, and the two navigations into it."""

    PATH = "/portal"

    def __init__(self, page: Page) -> None:
        self.page = page
        self.rail = LensRail(page)
        self.pane = ContextPane(page)

    def go(self) -> None:
        self.page.goto(self.PATH, wait_until="domcontentloaded")

    def wait_url_settled(self, timeout_ms: float = 20_000) -> None:
        """Wait until the app stops changing its own URL.

        `PortalLayout` pins a landing zone once the viewer's shape resolves and
        `replaceZone` rewrites the search params to do it. On a slow stand that
        resolution can land AFTER a zone has been opened by hand, so the zone
        switches under the pointer and every pane entry detaches mid-click. This
        waits for the app to stop steering before a caller clicks anything.
        """
        self.page.wait_for_function(
            """() => {
                const here = location.href;
                const now = performance.now();
                if (window.__urlSeen !== here) {
                    window.__urlSeen = here;
                    window.__urlSeenAt = now;
                    return false;
                }
                return now - window.__urlSeenAt > 300;
            }""",
            timeout=timeout_ms,
        )

    def content(self) -> Locator:
        """The screen's own content area.

        Measured: the shell renders TWO `main` elements — the sidebar inset is one
        and the zone content is nested inside it — so a bare `main` locator is a
        strict-mode violation rather than a wait.
        """
        return self.page.locator("main").last

    def recorded_path(self) -> str:
        return recorded_path_of(self.page.url)


def recorded_path_of(url: str) -> str:
    """The string the product records for a URL, with the person stripped out.

    Two rules, both the product's. `currentScreen()` in `main.tsx` builds the
    screen from the pathname plus only the `zone` and `item` search params — every
    other param (`scope`, `period`, `dir`, `lens`) is absent, so two readers with
    different scopes on one view count as one screen. `screenPath()` in
    `telemetry.ts` then replaces the segment after `/ic` and any
    identifier-shaped segment with `:id`, so adoption counting cannot become a
    record of who read whose profile. A journey asserts against this, never
    against the raw url, because the raw url is what must NOT be recorded.
    """
    parts = urlsplit(url)
    query = parse_qs(parts.query)
    named = [values[0] for key in ("zone", "item") if (values := query.get(key)) and values[0]]
    screen = f"{parts.path}/{'/'.join(named)}" if named else parts.path
    segments = screen.split("/")
    return "/".join(
        ":id" if _is_person_key(segments, index) else segment
        for index, segment in enumerate(segments)
    )


def _is_person_key(segments: list[str], index: int) -> bool:
    if index > 0 and segments[index - 1] == "ic":
        return True
    segment = segments[index]
    return bool(_UUID.match(segment) or _LONG_NUMBER.match(segment))
