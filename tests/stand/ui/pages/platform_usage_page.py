"""The Manage zone's Platform usage screen — the adoption surface.

Locators and navigation only.

Reached at `/portal?zone=manage&item=platform-usage`, and only by a viewer
holding the active `admin` identity role: the pane entry is `adminOnly`, and the
view itself renders behind `AdminGate`. The server refuses the summary
regardless of what the frontend draws, which is what `tests/stand/api` covers —
here the interest is what an admin SEES.

Measured on a mock-mode build of `src/frontend` at this commit:

    period bar   buttons "Week" "Month" "Quarter" "Year", and one whose name
                 carries the active range ("Custom date range: 17 Jul ...") —
                 the range is part of the accessible name, so it is matched by
                 prefix rather than in full
    three KPIs   a bordered tile per figure, label under value: "visits",
                 "people", "pages opened"
    chart        heading "Visits per day"
    three tables role=table with aria-label "Who opened it",
                 "What they opened", "Drill-downs and other actions"
                 (the section heading above the last one reads
                 "Drill-downs and other actions, by opens" — the table's own
                 label is the shorter string)

Two shapes a journey has to plan around, both measured rather than assumed:

**The KPI tiles carry no role, no `aria-label`, no `data-slot`.** The label text
is the only handle, and the figure is its preceding sibling — hence
`kpi_figure()`'s one-step xpath walk. It is the weakest locator in this module;
a `data-slot="card"` on the tile, or an `aria-label` naming the figure, would
retire the walk. Worth raising as a product change rather than working around
forever.

**The tables are virtualized and only render what is in view.** Measured with
mock data: `tr[data-slot="table-row"]` for every row, `th[data-slot="table-head"]`
in the header one, and `data-index` present ONLY on data rows — which makes
`rows()` exact without counting the header out. The container scrolls at
`max-h-90` with `overscan: 8`, so a table longer than about nine rows has the
rest outside the DOM entirely: reading a full column means scrolling the
container, which `flows.collect_usage_rows` does.

**The Page column shows the screen's label, not its path.** `screenLabel()`
turns `/portal/manage/platform-usage` into the zone and item names joined by its
own `SEPARATOR`; the raw path is in a tooltip that renders on hover, as
`[data-slot="tooltip-content"]`.
So a journey asserting what was recorded hovers the cell (`page_path_tooltip`),
and a journey asserting a person was NOT recorded checks the whole page rather
than the visible cells.
"""

from __future__ import annotations

from playwright.sync_api import Locator, Page

TABLE_ROW = '[data-slot="table-row"]'

PEOPLE_TABLE = "Who opened it"

PAGES_TABLE = "What they opened"

ACTIONS_TABLE = "Drill-downs and other actions"


class PlatformUsagePage:
    ZONE = "Manage"

    ITEM = "Platform usage"

    #: Deep link, for the journey that proves the gate is the server's and not
    #: the pane's: a viewer holding no admin row can address this.
    PATH = "/portal?zone=manage&item=platform-usage"

    def __init__(self, page: Page) -> None:
        self.page = page

    def go(self) -> None:
        self.page.goto(self.PATH, wait_until="domcontentloaded")

    def period(self, label: str) -> Locator:
        return self.page.get_by_role("button", name=label, exact=True)

    def custom_range(self) -> Locator:
        return self.page.get_by_role("button", name="Custom date range:")

    def kpi_figure(self, label: str) -> Locator:
        """The number above a KPI label. See the module docstring on the walk."""
        return (
            self.page.get_by_text(label, exact=True)
            .locator("xpath=preceding-sibling::div[1]")
            .first
        )

    def chart_heading(self) -> Locator:
        return self.page.get_by_role("heading", name="Visits per day")

    def table(self, label: str) -> Locator:
        return self.page.get_by_role("table", name=label)

    def rows(self, label: str) -> Locator:
        """Data rows only — `data-index` is the virtualizer's, and the header has none."""
        return self.table(label).locator(f"{TABLE_ROW}[data-index]")

    def empty_state(self) -> Locator:
        return self.page.get_by_text("No usage in this period yet")

    def load_failed(self) -> Locator:
        return self.page.get_by_text("Usage could not be loaded")

    def page_cell(self, row: Locator) -> Locator:
        return row.locator('[data-slot="table-cell"]').first

    def page_path_tooltip(self) -> Locator:
        """The raw path behind a Page label, revealed by hovering that label."""
        return self.page.locator('[data-slot="tooltip-content"]')
