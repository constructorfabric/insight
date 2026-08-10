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

from .group_dialog import GroupDialog, MetricEvidenceDialog


class GitOutputDetails(GroupDialog):
    """The Git group's dialog, plus the repository timeseries it leads with.

    Everything general lives in `GroupDialog`; what is here is specific to the
    one block this group opens on — a table by repository, and the commit column
    inside it.
    """

    def __init__(self, page: Page) -> None:
        super().__init__(page, "Git output")

    def repository_table(self) -> Locator:
        return self.dialog.get_by_role("table").filter(has_text="PRs merged")

    def table(self) -> Locator:
        return self.repository_table()

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

    def open_first_commit_bucket(self) -> MetricEvidenceDialog:
        table = self.repository_table()
        data_row = table.get_by_role("rowgroup").nth(1).get_by_role("row").first
        data_row.get_by_role("cell").nth(1).get_by_role("button").click()
        return MetricEvidenceDialog(self.page, "Commits")


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

    def team_view_switch(self) -> Locator:
        return self.page.get_by_role("button", name="Team", exact=True)

    def kpi_tile(self, label: str) -> Locator:
        return self.page.get_by_role("button", name=f"Open {label} details")

    def kpi_value(self, label: str) -> Locator:
        return self.kpi_tile(label).locator("[data-slot='card-title']")

    def populated_domain_card(self, label: str) -> Locator:
        return self.page.get_by_role("button", name=f"Open {label} details")

    def empty_domain_card(self, label: str) -> Locator:
        return self.page.locator("[data-slot='card']").filter(
            has=self.page.get_by_text(label, exact=True)
        )

    def open_domain(self, label: str) -> GroupDialog:
        self.populated_domain_card(label).click()
        return GroupDialog(self.page, label)

    def open_git_output(self) -> GitOutputDetails:
        self.populated_domain_card("Git output").click()
        return GitOutputDetails(self.page)
