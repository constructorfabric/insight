"""The portal shell — the lens rail, the contextual pane, and the screens they reach.

Rail zones and pane items are the same shadcn `Sidebar` button, separable only by
container: the rail by `[data-testid="lens-rail"]`, a pane item by the
`sidebar-group` no rail entry has. The rail is collapsed until hovered, so a click
needs a `hover()` first. A portal screen is identified by its `zone` and `item`
search params, not by its path.
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

_UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", re.I)

_LONG_NUMBER = re.compile(r"^\d{6,}$")


class LensRail:
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
        self.hover()
        self.zone(label).click()


class ContextPane:
    def __init__(self, page: Page) -> None:
        self.page = page

    def items(self) -> Locator:
        return self.page.locator(PANE_ITEM)

    def views(self) -> Locator:
        """The zone's own screens. WorkChart renders `<a>` per person, not `<button>`."""
        return self.page.locator(PANE_VIEW)

    def item(self, label: str) -> Locator:
        return self.items().filter(has_text=re.compile(rf"^{re.escape(label)}$"))

    def view_labels(self) -> list[str]:
        return [text.strip() for text in self.views().all_inner_texts()]

    def open_item(self, label: str, timeout_ms: float | None = None) -> None:
        self.item(label).click(timeout=timeout_ms)

    def wait_settled(self, timeout_ms: float = 15_000) -> None:
        """Wait until the pane stops re-rendering. A re-render detaches the button mid-click."""
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
    PATH = "/portal"

    def __init__(self, page: Page) -> None:
        self.page = page
        self.rail = LensRail(page)
        self.pane = ContextPane(page)

    def go(self) -> None:
        self.page.goto(self.PATH, wait_until="domcontentloaded")

    def wait_url_settled(self, timeout_ms: float = 20_000) -> None:
        """Wait until the app stops changing its own URL.

        `PortalLayout`'s `replaceZone` can rewrite the search params after a zone
        has been opened by hand, detaching every pane entry mid-click.
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
        """The screen's own content area. The shell renders two `main` elements."""
        return self.page.locator("main").last

    def recorded_path(self) -> str:
        return recorded_path_of(self.page.url)


def recorded_path_of(url: str) -> str:
    """The string the product records for a URL, with the person stripped out.

    Mirrors `screenOf()` in usage-collection.ts and `screenPath()` in telemetry.ts.
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
