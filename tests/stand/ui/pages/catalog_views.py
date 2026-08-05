"""The two views that belong to nobody in particular: `/metrics` and `/whats-new`.

Together in one module: each exposes a single locator, and journeys use them
only as navigation targets — somewhere to go that is not a person's view.
"""

from __future__ import annotations

from playwright.sync_api import Locator, Page


class MetricsCatalogView:
    PATH = "/metrics"

    def __init__(self, page: Page) -> None:
        self.page = page

    def go(self) -> None:
        self.page.goto(self.PATH, wait_until="domcontentloaded")

    def heading(self) -> Locator:
        return self.page.get_by_role("heading", name="Metric catalog")


class WhatsNewView:
    PATH = "/whats-new"

    def __init__(self, page: Page) -> None:
        self.page = page

    def go(self) -> None:
        self.page.goto(self.PATH, wait_until="domcontentloaded")

    def heading(self) -> Locator:
        # `.first`: the view renders a page heading and a dated release heading
        # that both carry this text. Narrowing resolves identically every run.
        return self.page.get_by_role("heading", name="What's new").first
