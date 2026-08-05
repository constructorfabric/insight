"""The authenticated landing view. Locators and getters only — no assertions,
no test data, no branching on page state.

Every locator here is accessibility-first (role + accessible name). That is not
just a preference on this app: the published SPA carries **no `data-testid`
attributes at all** (checked across the whole landing DOM), so roles and
accessible names are the only stable handles available without asking the
frontend owners for markup changes.
"""

from __future__ import annotations

from playwright.sync_api import Locator, Page


class LandingPage:
    def __init__(self, page: Page) -> None:
        self.page = page

    def main_landmark(self) -> Locator:
        # `.first`: this view renders two `main` landmarks (the shell and the
        # content area). Narrowing a locator is not state branching — it
        # resolves the same way on every run.
        return self.page.get_by_role("main").first

    def person_heading(self, display_name: str) -> Locator:
        """The heading naming whoever the view is scoped to."""
        return self.page.get_by_role("heading", name=display_name)

    def user_menu(self, display_name: str) -> Locator:
        """The account control, whose accessible name carries the signed-in
        person's name and email."""
        return self.page.get_by_role("button", name=display_name)

    def person_link(self, display_name: str) -> Locator:
        """A link to another person in the signed-in user's org scope."""
        return self.page.get_by_role("link", name=display_name)
