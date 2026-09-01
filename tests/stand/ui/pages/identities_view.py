"""Manage → Identities — the operator's correction console. Locators and navigation only.

The console is one screen in three modes, all of them query state on
`/portal`: `zone=manage`, `item=identities`, `mode=queue|person|accounts`,
plus `acct=<source>:<source_id>:<account_id>` for the account window. That is
deliberate on the product's side — "a reload restores the same screen, a link
reproduces it for someone else" — and it is what lets a journey assert that a
correction survives a refresh rather than only that it rendered once.

Accessibility-first, like every page object here: the published SPA carries no
`data-testid` attributes, so roles and accessible names are the only stable
handles. An account opens into a modal dialog named after the account itself,
and while it is open the rest of the screen is hidden from accessibility — so
the window, not the page, is what a journey scopes its assertions to.
"""

from __future__ import annotations

from urllib.parse import quote

from playwright.sync_api import Locator, Page

#: The person picker inside an account window, labelled the same way.
PICKER_LABEL = "Search people by name, email, handle or person id…"


def account_key(source: str, source_id: str, account_id: str) -> str:
    """The `?acct=` key, packed exactly as `account-key.ts` packs it.

    Each part is URI-encoded before joining on `:`, so an `account_id`
    containing the separator cannot forge a different triple. A journey builds
    the key here rather than reading it off the URL when it needs to open one
    account directly.
    """
    return ":".join(quote(part, safe="") for part in (source, source_id, account_id))


class IdentitiesView:
    """The console screen, its account listing, and the window a row opens."""

    def __init__(self, page: Page) -> None:
        self.page = page

    def go(
        self,
        mode: str = "accounts",
        acct: str | None = None,
        person: str | None = None,
    ) -> None:
        query = f"?zone=manage&item=identities&mode={mode}"
        if acct is not None:
            query += f"&acct={quote(acct, safe='')}"
        if person is not None:
            query += f"&person={quote(person, safe='')}"
        self.page.goto(f"/portal{query}")

    def heading(self) -> Locator:
        return self.page.get_by_role("heading", name="Identities", exact=True)

    def refusal(self) -> Locator:
        """What a caller without the admin role gets instead of the console.

        `role="alert"` on the product's side. The gate fails CLOSED while the
        check is in flight, so a journey asserting the refusal must also
        assert the console is absent — otherwise it would pass on a spinner.
        """
        return self.page.get_by_role("alert").filter(
            has_text="Identities is an admin surface"
        )

    def rate_figure(self, label: str) -> Locator:
        """The number on one headline tile, as its own node.

        The figure, not the tile: a tile-wide substring match would accept any
        number CONTAINING the expected one — 1 passes on 10 and on 21 — so the
        assertion has to land on the element whose whole text is the figure.
        The label is the tile's other child, which is what finds the tile.
        """
        tile = self.page.get_by_text(label, exact=True).locator("xpath=..")
        return tile.locator("div").first

    def person_window(self, person_id: str) -> Locator:
        """The window one person is worked in, opened by `?person=`.

        Found by the id it carries rather than by its heading: arriving by a
        link there is no card to name them — search resolves values, not ids —
        so the product titles the window "Unnamed person" and prints the id
        beside it. Filtering on the id keeps this working whichever way the
        window was reached.
        """
        return self.page.get_by_role("dialog").filter(has_text=person_id)

    # --- the account window ---------------------------------------------
    #
    # A modal dialog: while it is open the rest of the screen is hidden from
    # accessibility, so nothing outside it — the console heading included —
    # can be asserted at the same time. Everything below is scoped to it.

    def account_window(self, account_label: str) -> Locator:
        """The window one account is decided in, named by the account itself."""
        return self.page.get_by_role("dialog", name=account_label)

    def current_binding(self, window: Locator) -> Locator:
        """The section naming who holds the account RIGHT NOW.

        Scoped to the section rather than the window, and that is the whole
        point: the window also carries a decision history, in which every
        person the account was ever bound to is named. An assertion over the
        whole window would be satisfied by a history entry — it would pass
        even if the correction had not moved the binding at all.
        """
        return window.locator("section").filter(
            has=self.page.get_by_text("Currently bound to", exact=True)
        )

    def person_search(self) -> Locator:
        """The picker sits open on the screen — there is no button to reveal it.

        Its section is labelled "Assign to a person", or "Assign to someone
        else" once the account has a holder; the field itself is labelled the
        same either way, so a journey does not have to know which.
        """
        return self.page.get_by_role("searchbox", name=PICKER_LABEL)

    def person_option(self, display_name: str) -> Locator:
        """One person offered by the picker.

        A `role="button"` rather than a listbox option, and deliberately so on
        the product's side: the card inside carries its own copy control, and
        a button may not nest a button.
        """
        return self.page.get_by_role("button", name=display_name, exact=True)

    def confirmation(self, title: str) -> Locator:
        """The second dialog — a correction is always confirmed before it is sent."""
        return self.page.get_by_role("dialog", name=title)

    def confirm(self, label: str) -> Locator:
        return self.page.get_by_role("button", name=label, exact=True)
