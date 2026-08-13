"""Journey 4 — an anonymous browser is refused an authenticated view.

Why this is a browser test and not an API test, measured rather than asserted:
**every SPA route answers 200 text/html to an anonymous HTTP client.** The edge
serves the same 1056-byte `index.html` for `/`, `/metrics`, `/whats-new` and any
`/ic/<anyone>/personal`, so a curl-level check sees success everywhere and proves
nothing. Refusal exists only inside the browser: the SPA boots, asks
`/auth/me` (401), and the root route's `beforeLoad` sends the window to
`/auth/login`, which the gateway turns into a redirect to the IdP.

That is also why this cannot be folded into `test_gateway.py`'s 401 sweep. The
sweep covers the API surface, where the edge does refuse. This covers the SPA
surface, where it does not — and where the product's protection is client-side.

The context here is deliberately NOT the shared `page`: the other journeys
authenticate theirs, and reusing one would test a signed-out state that never
existed.
"""

from __future__ import annotations

from urllib.parse import urlsplit

import pytest
from insight_stand import SESSION_COOKIE_NAME, Manifest
from playwright.sync_api import Browser, expect

from .pages.keycloak_login_page import KeycloakLoginPage
from .pages.person_view import PersonView

# Quality vector of this module's tests.
pytestmark = pytest.mark.security

#: The routes that need no path parameter. The person-scoped route is NOT here:
#: it needs a real email from the manifest, so it gets its own case below —
#: which is the one that matters most, since a pasted deep link to a colleague's
#: view is exactly how this protection gets tested in the wild.
AUTHENTICATED_ROUTES = ("/", "/metrics", "/whats-new")


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.parametrize("route", AUTHENTICATED_ROUTES)
def test_an_anonymous_browser_is_sent_to_the_idp(
    browser: Browser, base_url: str, route: str
) -> None:
    """No product view, no session cookie — the IdP's own form instead.

    Asserted on the FORM rather than on the URL containing "keycloak": a stand
    pointed at a different IdP should still pass this, and what matters to a user
    is that they were asked to sign in, not which host asked them.
    """
    context = browser.new_context(base_url=base_url)
    try:
        page = context.new_page()
        page.goto(route, wait_until="domcontentloaded")
        page.wait_for_load_state("networkidle")

        expect(KeycloakLoginPage(page).username_field()).to_be_visible()

        # Asserted on the PATH, not the origin: this IdP can be published on
        # the app's own hostname (this stand serves it at /kc), so asserting
        # the browser left base_url fails there. `/protocol/openid-connect/auth`
        # is Keycloak's own URL layout, not something OIDC itself mandates.
        path = urlsplit(page.url).path
        assert "/protocol/openid-connect/auth" in path, (
            f"an anonymous visit to {route} left the browser at {page.url}, which is not an "
            "OIDC authorization endpoint — the product view was served without a session"
        )
        assert [c["name"] for c in context.cookies(base_url)] == [], (
            f"an anonymous visit to {route} left cookies on {base_url}: {context.cookies(base_url)}"
        )
    finally:
        context.close()


@pytest.mark.requires_seed("dev_lead")
def test_an_anonymous_browser_cannot_deep_link_to_someone_elses_view(
    browser: Browser, base_url: str, stand_manifest: Manifest
) -> None:
    """A person-scoped deep link is refused like any other route.

    Worth its own case: the route carries a real person's email in the path, so a
    regression that authenticated the shell but not the person-scoped routes —
    or that rendered the view before resolving the session — would leak one
    person's page to anyone holding the link.
    """
    target = stand_manifest.fixture("dev_lead")

    context = browser.new_context(base_url=base_url)
    try:
        page = context.new_page()
        page.goto(PersonView.path(target.uuid), wait_until="domcontentloaded")
        page.wait_for_load_state("networkidle")

        expect(KeycloakLoginPage(page).username_field()).to_be_visible()
        assert target.display_name not in page.content(), (
            f"the anonymous page mentions {target.display_name}, so some part of "
            "the view rendered before the session was checked"
        )
        assert SESSION_COOKIE_NAME not in [c["name"] for c in context.cookies(base_url)]
    finally:
        context.close()
