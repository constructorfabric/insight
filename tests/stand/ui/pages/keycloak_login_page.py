"""Keycloak's real sign-in form. Locators and actions only — no assertions,
no test data, no branching on page state.

Accessible names confirmed against the live form served by this stand's realm,
not guessed:

    <label for="username">Username or email</label>
    <label for="password">Password</label>
    <input type="submit" name="login" value="Sign In">

Both fields are addressed by ROLE + accessible name rather than by label text.
`get_by_label("Password")` is ambiguous on this page — it also matches the
"Show password" toggle button, which carries `aria-controls="password"` — and
Playwright fails such a locator in strict mode. Narrowing to the `textbox` role
removes the ambiguity without falling back to CSS or a `data-testid`.
"""

from __future__ import annotations

from playwright.sync_api import Locator, Page


class KeycloakLoginPage:
    def __init__(self, page: Page) -> None:
        self.page = page

    def username_field(self) -> Locator:
        return self.page.get_by_role("textbox", name="Username or email")

    def password_field(self) -> Locator:
        return self.page.get_by_role("textbox", name="Password")

    def submit_button(self) -> Locator:
        return self.page.get_by_role("button", name="Sign In")

    def fill_and_submit(self, username: str, password: str) -> None:
        self.username_field().fill(username)
        self.password_field().fill(password)
        self.submit_button().click()
