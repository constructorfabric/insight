"""Multi-step browser actions that compose page objects.

Page objects answer "where is it" and nothing else — no assertions, no test data,
no branching. A sign-in is three of them in sequence, which belongs neither in a
page object nor in a test that is about something else.

Kept here rather than in a fixture on purpose. A `signed_in_page` fixture would
share one authenticated page across journeys, and each journey is a statement
about a complete round trip from a cold browser; sharing would make the later
ones depend on the earlier ones having run.


"""

from __future__ import annotations

from insight_stand import PersonaSession
from playwright.sync_api import Page

from .pages.keycloak_login_page import KeycloakLoginPage
from .pages.login_page import LoginPage


def sign_in(page: Page, base_url: str, persona: PersonaSession) -> None:
    """Drive the deployed OIDC chain until the app renders for that persona.

    No shortcut at any step: an unauthenticated visit to `/` starts
    authorization-code+PKCE by itself, Keycloak serves its real form, and the
    authenticator sets `__Host-sid` at the callback. Nothing is minted.
    """
    LoginPage(page).go()
    KeycloakLoginPage(page).fill_and_submit(persona.email, persona.password)
    page.wait_for_url(f"{base_url}/**")
