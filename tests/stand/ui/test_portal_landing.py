"""Journey — a person signs in and lands in the portal.

The portal is the product's only shell: `/` is a redirect into it rather than a
page, so a reader who asks for nothing in particular still arrives there with
the lens rail rendered. Nothing below the browser is stubbed — the real
Keycloak form, the real session cookie, the published SPA.
"""

from __future__ import annotations

import re
from collections.abc import Callable

import pytest
from insight_stand import PersonaSession
from playwright.sync_api import Page, expect

from .pages.keycloak_login_page import KeycloakLoginPage
from .pages.login_page import LoginPage
from .pages.portal_shell import PortalShell

# Quality vector of this module's tests.
pytestmark = pytest.mark.reliability


@pytest.mark.requires_seed("dev_lead")
def test_signing_in_lands_in_the_portal(
    page: Page,
    session_for: Callable[[str], PersonaSession],
) -> None:
    persona = session_for("dev_lead")

    LoginPage(page).go()
    KeycloakLoginPage(page).fill_and_submit(persona.email, persona.password)

    portal = PortalShell(page)

    # Matched on the path, not the whole URL: a stand may serve the SPA from
    # its own origin rather than the gateway's, and the journey is about where
    # the app puts the reader, not about which host answered.
    expect(page).to_have_url(re.compile(r"/portal(\?|$)"))
    expect(portal.rail.zones().first).to_be_visible()
