"""Journey 1 — a person signs in and lands on their own view.

The first thing in this repository that drives the product the way a human
does: a real browser, the published SPA image, the real Keycloak form, and the
same session cookie a real login produces. Everything below the browser is the
deployed stack — nothing is stubbed and no token is minted.


"""

from __future__ import annotations

from collections.abc import Callable

import pytest
from insight_stand import SESSION_COOKIE_NAME, PersonaSession
from playwright.sync_api import Cookie, Page, expect

from .pages.keycloak_login_page import KeycloakLoginPage
from .pages.landing_page import LandingPage
from .pages.login_page import LoginPage


def _cookie_failure(page: Page, origin: str, found: list[Cookie]) -> str:
    """Explain a missing session cookie, including the usual cause.

    A `__Host-` cookie is dropped silently on an origin the browser does not
    trust, and the visible symptom is an endless login redirect — so the
    diagnosis belongs here, where someone actually meets the failure, rather
    than in a preflight fixture every journey has to remember to use.
    """
    lines = [
        f"expected exactly one {SESSION_COOKIE_NAME} cookie on {origin}, "
        f"found {[c['name'] for c in found]}"
    ]
    if not page.evaluate("window.isSecureContext"):
        lines.append(
            f"{origin} is not a trustworthy origin in this browser, so a "
            f"{SESSION_COOKIE_NAME} cookie can never be stored. Run the container in "
            "the gateway's network namespace (--network container:insight-gateway) "
            "with INSIGHT_STAND_BASE_URL pointing at localhost:<gateway port>."
        )
    return "\n".join(lines)


@pytest.mark.requires_seed("dev_lead")
def test_login_lands_on_authenticated_view(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    # `session_for` resolves the persona through the manifest's fixture catalog
    # and verifies their realm roles before anything drives a browser. Its own
    # API-side session is not what is under test here — the browser wins its
    # own — but reusing the credential it already resolved guarantees both
    # halves authenticate as the same human with the same secret.
    persona = session_for("dev_lead")
    person = persona.person

    login_page = LoginPage(page)
    keycloak_login_page = KeycloakLoginPage(page)
    landing_page = LandingPage(page)

    login_page.go()
    # No sign-in click: an unauthenticated visit starts the OIDC chain itself,
    # so the browser is already sitting on Keycloak's form by now.
    keycloak_login_page.fill_and_submit(persona.email, persona.password)
    page.wait_for_url(f"{base_url}/**")

    cookies = page.context.cookies(base_url)
    session_cookies = [c for c in cookies if c["name"] == SESSION_COOKIE_NAME]
    assert len(session_cookies) == 1, _cookie_failure(page, base_url, cookies)

    cookie = session_cookies[0]
    # The `__Host-` prefix is a contract, not decoration: host-locked, path-/,
    # secure. Asserting the attributes is what proves the browser accepted it
    # on those terms rather than storing something weaker under the same name.
    assert cookie["secure"] is True, f"{SESSION_COOKIE_NAME} is not Secure: {cookie}"
    assert cookie["path"] == "/", f"{SESSION_COOKIE_NAME} is not Path=/: {cookie}"
    assert cookie["httpOnly"] is True, f"{SESSION_COOKIE_NAME} is not HttpOnly: {cookie}"

    expect(landing_page.main_landmark()).to_be_visible()
    # The view is the PERSONA's, not just any authenticated view: their name
    # heads it, and the account control carries their identity.
    expect(landing_page.person_heading(person.display_name)).to_be_visible()
    expect(landing_page.user_menu(person.display_name)).to_be_visible()
