"""The application entry point."""

from __future__ import annotations

from playwright.sync_api import Locator, Page


class LoginPage:
    """The app's own origin, before a session exists.

    On this stand there is nothing to click: an unauthenticated visit to `/`
    starts the OIDC chain by itself and the browser lands on the IdP's form.
    `sign_in_control()` is kept for stands whose SPA renders an explicit
    control instead.
    """

    def __init__(self, page: Page) -> None:
        self.page = page

    def go(self) -> None:
        self.page.goto("/", wait_until="domcontentloaded")

    def sign_in_control(self) -> Locator:
        return self.page.get_by_role("link", name="Log in")
