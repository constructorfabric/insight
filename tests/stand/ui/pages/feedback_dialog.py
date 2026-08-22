"""The feedback dialog and the rail control that opens it. Locators and navigation only.

The control is a rail button: an icon plus its name in an `sr-only` span, so it is
reached by role and accessible name like everything else here. The rail is collapsed
until hovered, so opening it is hover-then-click — the same sequence `portal_shell.py`
documents for a zone.

The dialog is not the button's child. `FeedbackDialogProvider` mounts it at the router
root, which is why it is addressed from the page rather than from the rail.
"""

from __future__ import annotations

from playwright.sync_api import Locator, Page

from .portal_shell import RAIL


class FeedbackDialog:
    OPEN = "Send feedback"

    TITLE = "Submit feedback / bug report"

    MESSAGE = "Your feedback"

    SEND = "Send"

    SENT = "Thanks — your feedback was sent."

    def __init__(self, page: Page) -> None:
        self.page = page

    def control(self) -> Locator:
        return self.page.locator(RAIL).get_by_role("button", name=self.OPEN)

    def open(self) -> None:
        self.page.locator(RAIL).hover()
        self.control().click()

    def dialog(self) -> Locator:
        return self.page.get_by_role("dialog", name=self.TITLE)

    def message(self) -> Locator:
        return self.dialog().get_by_role("textbox", name=self.MESSAGE)

    def send(self) -> Locator:
        return self.dialog().get_by_role("button", name=self.SEND, exact=True)

    def sent_toast(self) -> Locator:
        return self.page.get_by_text(self.SENT)
