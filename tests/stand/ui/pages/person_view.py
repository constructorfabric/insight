"""One person's own view — `/ic/$person_id/personal`. Locators and navigation only.

`$person_id` is the person's canonical UUID since the identity cutover (#2098);
it was the email before. The SPA builds the link with
`encodeURIComponent`, so `@` arrives as `%40`. Encoding it here rather than in a
test keeps the URL shape a property of the view.

Accessibility-first, like every page object here: the published SPA carries no
`data-testid` attributes at all (re-verified across the whole shipped bundle),
so roles and accessible names are the only stable handles.
"""

from __future__ import annotations

from urllib.parse import quote

from playwright.sync_api import Locator, Page


class GitOutputDrilldown:
    def __init__(self, page: Page) -> None:
        self.page = page
        self.dialog = page.get_by_role("dialog", name="Git output")

    def table(self) -> Locator:
        return self.dialog.get_by_role("table").filter(has_text="PRs merged")

    def chart_view(self) -> Locator:
        return (
            self.table()
            .locator('xpath=ancestor::*[@data-slot="card"][1]')
            .get_by_role("button", name="Chart view")
        )

    def export(self) -> Locator:
        return (
            self.table()
            .locator('xpath=ancestor::*[@data-slot="card"][1]')
            .get_by_role("button", name="Export")
        )

    def metric_selector(self) -> Locator:
        return self.dialog.get_by_role("combobox", name="Metric").filter(has_text="Commits")

    def close(self) -> Locator:
        return self.dialog.get_by_role("button", name="Close")


class PersonView:
    def __init__(self, page: Page) -> None:
        self.page = page

    @staticmethod
    def path(person_id: str) -> str:
        return f"/ic/{quote(person_id, safe='')}/personal"

    def go(self, person_id: str) -> None:
        self.page.goto(self.path(person_id), wait_until="domcontentloaded")

    def person_heading(self, display_name: str) -> Locator:
        return self.page.get_by_role("heading", name=display_name)

    def metric_tile(self, label: str) -> Locator:
        """The tile for one named metric.

        Addressed by its visible label rather than by position: the set of tiles
        and their order are product decisions, and a test that indexed into them
        would fail on a layout change that broke nothing.
        """
        return self.page.get_by_role("listitem").filter(has_text=label).first

    def open_git_output(self) -> GitOutputDrilldown:
        self.page.get_by_role("button", name="Open Git output details").click()
        return GitOutputDrilldown(self.page)
