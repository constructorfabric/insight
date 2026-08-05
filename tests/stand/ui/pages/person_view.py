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
