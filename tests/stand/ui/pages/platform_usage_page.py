"""The Manage zone's Platform usage screen. Locators and navigation only.

Reached at `/portal?zone=manage&item=platform-usage`, admin-only. The tables are
virtualized and keep about nine rows in the DOM. The Page column shows
`screenLabel()`'s name; the raw path is in a hover tooltip.
"""

from __future__ import annotations

from playwright.sync_api import Locator, Page

TABLE_ROW = '[data-slot="table-row"]'

PEOPLE_TABLE = "Who opened it"

PAGES_TABLE = "What they opened"

FEEDBACK_TABLE = "What people told us"


class PlatformUsagePage:
    ITEM = "Platform usage"

    PATH = "/portal?zone=manage&item=platform-usage"

    def __init__(self, page: Page) -> None:
        self.page = page

    def go(self) -> None:
        self.page.goto(self.PATH, wait_until="domcontentloaded")

    def chart_heading(self) -> Locator:
        return self.page.get_by_role("heading", name="Visits per day")

    def table(self, label: str) -> Locator:
        return self.page.get_by_role("table", name=label)

    def rows(self, label: str) -> Locator:
        """Data rows only — `data-index` is the virtualizer's, and the header has none."""
        return self.table(label).locator(f"{TABLE_ROW}[data-index]")

    def row_at(self, label: str, index: int) -> Locator:
        """One row by `data-index` — identity in a virtualized table, where position is not."""
        return self.table(label).locator(f'{TABLE_ROW}[data-index="{index}"]')

    def header(self, label: str) -> Locator:
        """A column header — somewhere to park the pointer that opens no tooltip."""
        return self.table(label).locator('[data-slot="table-head"]').first

    def page_cell(self, row: Locator) -> Locator:
        return row.locator('[data-slot="table-cell"]').first

    def page_label(self, row: Locator) -> Locator:
        """The tooltip trigger is the span inside the cell, not the cell."""
        return self.page_cell(row).locator('[data-slot="tooltip-trigger"]')

    def tooltips(self) -> Locator:
        """Every mounted tooltip. One animating out still matches `[data-open]`."""
        return self.page.locator('[data-slot="tooltip-content"]')
