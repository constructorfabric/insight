"""The product's primary navigation. Locators and actions only.

These are the links a person actually clicks, and clicking one is a CLIENT-SIDE
route change: the SPA swaps the view without reloading the document. That is a
different operation from `page.goto()`, which throws the document away and boots
the app again, and only one of the two is what a user does to get around.

Measured on this stand: a click keeps `window` alive, `page.goto()` does not.
`test_navigation_holds_session.py` exercises both deliberately.

Accessible names come from the rendered sidebar, not from the bundle's i18n
catalog: "Metric catalog", "What's new", "Personal", "Team", plus one link per
person in the signed-in user's org scope, named by display name.

Not every link is on every view, which a caller has to plan around. Observed:

    view                     links present
    /                        person names, Metric catalog, What's new, Personal, Team
    /metrics, /whats-new     person names, Metric catalog, What's new
    /ic/<person>/personal    person names, Metric catalog, What's new, Personal, Team

So "Personal" and "Team" are scoped to the person section and to the landing
view; the person links and the two catalog links are everywhere. A page object
reports where things are, so this table lives here rather than in a test.

**"Personal" and "Team" are anchors carrying an explicit `role="button"`**, so
they are reached by the BUTTON role even though they navigate. Measured, after
`get_by_role("link", name="Team")` matched nothing while `a[href$='/team']`
matched one visible element:

    text "Team"  display flex  visibility visible  role="button"  href=/ic/<email>/team

That is a product decision this page object reports rather than argues with —
though it is worth someone's attention, since an explicit `role="button"` tells
assistive technology the control acts in place when it actually navigates, and
costs a screen-reader user the link affordances (open in a new tab, "list all
links") that the underlying anchor would otherwise give them.
"""

from __future__ import annotations

from playwright.sync_api import Locator, Page


class SidebarNav:
    def __init__(self, page: Page) -> None:
        self.page = page

    def link(self, name: str) -> Locator:
        return self.page.get_by_role("link", name=name)

    def metric_catalog(self) -> Locator:
        return self.link("Metric catalog")

    def whats_new(self) -> Locator:
        return self.link("What's new")

    def personal(self) -> Locator:
        """An anchor with `role="button"` — see the module docstring."""
        return self.page.get_by_role("button", name="Personal")

    def team(self) -> Locator:
        """An anchor with `role="button"` — see the module docstring."""
        return self.page.get_by_role("button", name="Team")

    def person(self, display_name: str) -> Locator:
        """A colleague in the signed-in person's org scope."""
        return self.link(display_name)
