"""Journey 3 — the session survives moving around the product.

Why this is a browser test and not an API test: the API tests prove one request
carries a session. They cannot show that the SPA, after a route change, still
holds it — a broken cookie scope, a `SameSite` mistake, or a route that re-enters
the OIDC chain on every navigation all leave the API untouched and make the
product unusable. The failure this catches is a login loop, and a login loop is
only observable in a browser.

**Two kinds of navigation, deliberately, because they fail differently.**
Measured on this stand by stamping `window` before the transition and reading it
back after:

    clicking a nav link      window survived   -> CLIENT-SIDE route change
    page.goto(<path>)        window lost       -> full document reload

Clicking is what a user does, and it is the only thing that exercises the SPA's
own router and its in-memory auth state. A reload is what a pasted deep link or
a refresh does, and it is the only thing that re-sends the cookie and re-runs the
whole bootstrap. A version of this journey that only ever called `goto()` would
miss a broken primary navigation entirely, because nothing would ever click the
product's own links.

The routes were read from the shipped bundle's route table rather than guessed:
`/`, `/metrics`, `/whats-new`, and `/ic/$person/{personal,team}`.
"""

from __future__ import annotations

from collections.abc import Callable

import pytest
from insight_stand import SESSION_COOKIE_NAME, PersonaSession
from playwright.sync_api import Page, expect

from .flows import sign_in
from .pages.catalog_views import MetricsCatalogView, WhatsNewView
from .pages.person_view import PersonView
from .pages.sidebar_nav import SidebarNav
from .pages.team_view import TeamView


def _assert_still_signed_in(page: Page, base_url: str, where: str) -> None:
    """Two checks, because either alone is weak.

    The URL check catches a redirect back to the IdP — the visible symptom of a
    lost session. The cookie check catches a session that was dropped but whose
    consequence has not surfaced yet, which is the state that turns into a loop
    one navigation later.

    Exactly one `__Host-sid`, not at least one: a second cookie under the same
    name would mean the chain ran again and left a stale session behind, which is
    the shape of the bug this journey exists to find.
    """
    assert page.url.startswith(base_url), (
        f"after {where} the browser is at {page.url}, which is not {base_url} — "
        "the session was lost and the SPA re-entered the OIDC chain"
    )
    session_cookies = [
        c for c in page.context.cookies(base_url) if c["name"] == SESSION_COOKIE_NAME
    ]
    assert len(session_cookies) == 1, (
        f"after {where} there are {len(session_cookies)} {SESSION_COOKIE_NAME} "
        f"cookies; expected exactly one"
    )


@pytest.mark.requires_seed("dev_lead")
def test_clicking_through_the_product_holds_the_session(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """Four views reached the way a person reaches them: by clicking.

    Every hop is a client-side route change, so this exercises the SPA's own
    router and its in-memory auth state — and, incidentally, proves the primary
    navigation links point where they claim to. A regression that broke a sidebar
    href would fail here and nowhere else in this repository.

    The order is not arbitrary. "Personal" and "Team" are scoped to the person
    section (see `SidebarNav` for where each link renders), so the person hop
    comes before them — clicking "Team" from `/whats-new` would time out on a
    link that view does not have.
    """
    persona = session_for("dev_lead")
    sign_in(page, base_url, persona)

    nav = SidebarNav(page)
    name = persona.person.display_name

    nav.metric_catalog().click()
    expect(MetricsCatalogView(page).heading()).to_be_visible()
    _assert_still_signed_in(page, base_url, "clicking Metric catalog")

    nav.whats_new().click()
    expect(WhatsNewView(page).heading()).to_be_visible()
    _assert_still_signed_in(page, base_url, "clicking What's new")

    # Back into the person section from a view that has no Personal/Team link —
    # the person's own name in the org tree is the way a user does it.
    nav.person(name).first.click()
    expect(PersonView(page).person_heading(name)).to_be_visible()
    _assert_still_signed_in(page, base_url, f"clicking {name}")

    nav.team().click()
    expect(TeamView(page).team_heading(name)).to_be_visible()
    _assert_still_signed_in(page, base_url, "clicking Team")


@pytest.mark.requires_seed("dev_lead")
def test_reloading_a_deep_link_holds_the_session(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """A full document load — a pasted URL or a refresh — keeps the session too.

    Distinct from the clicking journey rather than a duplicate of it. A reload
    throws away every bit of in-memory state and re-runs the whole bootstrap: the
    cookie is re-sent, `/auth/me` is asked again, and the router mounts from
    scratch. A cookie scoped wrongly, or a bootstrap that redirects before it
    resolves the session, breaks here while clicking still works.
    """
    persona = session_for("dev_lead")
    sign_in(page, base_url, persona)

    person = PersonView(page)
    person.go(persona.person.uuid)
    expect(person.person_heading(persona.person.display_name)).to_be_visible()
    _assert_still_signed_in(
        page, base_url, f"loading {PersonView.path(persona.person.uuid)} directly"
    )

    page.reload(wait_until="domcontentloaded")
    expect(person.person_heading(persona.person.display_name)).to_be_visible()
    _assert_still_signed_in(page, base_url, "reloading the same view")
