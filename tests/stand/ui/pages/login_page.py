"""The application entry point."""

from __future__ import annotations

from urllib.parse import urljoin

from playwright.sync_api import APIResponse, Locator, Page

from ..login import password_form_url


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

    def start(self) -> APIResponse:
        return self.page.request.get("/auth/login", max_redirects=0)

    def open_password_form(self, location: str, base_url: str) -> None:
        target = password_form_url(urljoin(base_url, location))
        self.page.goto(target, wait_until="domcontentloaded")
