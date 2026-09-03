"""The portal shell — the lens rail a signed-in reader lands on.

Rail zones and pane items are the same shadcn `Sidebar` button, separable only
by container, so the rail is addressed through `[data-testid="lens-rail"]`.
"""

from __future__ import annotations

from playwright.sync_api import Locator, Page

RAIL = '[data-testid="lens-rail"]'

MENU_BUTTON = '[data-slot="sidebar-menu-button"]'


class LensRail:
    def __init__(self, page: Page) -> None:
        self.page = page
        self.rail = page.locator(RAIL)

    def zones(self) -> Locator:
        return self.rail.locator(MENU_BUTTON)


class PortalShell:
    PATH = "/portal"

    def __init__(self, page: Page) -> None:
        self.page = page
        self.rail = LensRail(page)

    def go(self) -> None:
        self.page.goto(self.PATH, wait_until="domcontentloaded")
