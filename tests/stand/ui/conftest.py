"""Browser wiring for the UI journeys.

The browser's base URL is not set here. `tests/stand/conftest.py` resolves the
stand once into pytest-base-url's `base_url` option, and pytest-playwright
already reads that fixture to configure every context — so `page.goto("/")`
lands on the stand, and a journey that needs the address asks for `base_url`
and gets the one the browser was actually given.

That URL has to name a **trustworthy** origin, and not for convenience.
`__Host-` prefixed cookies are stored from nowhere else: any `https://` stand
qualifies, and over plain http a browser trusts exactly one host name.
Point a runner at `gateway:8080` and the session cookie is dropped without a
word: the SPA sees `/auth/me` 401, restarts the login, and loops until the
gateway's rate limiter turns it into a 503 that looks like a broken backend.
Chromium's `--unsafely-treat-insecure-origin-as-secure` does not help —
`window.isSecureContext` was measured as `false` with the flag on Chromium 149,
in `launch()` and `launch_persistent_context()`, with and without
`--user-data-dir`.

So a plain-http compose stand is addressed as `localhost:<port>`, which is
genuinely trustworthy and needs no flags: a containerised runner joins the
gateway's network namespace (`--network container:insight-gateway`), a
host-side run uses the published port. A deployed stand is reached at its own
https origin. Every one of them matches the stand's registered
`/auth/callback` redirect URI, so one configuration serves them all.
"""

from __future__ import annotations

import os
import time
from collections.abc import Callable, Iterator
from contextlib import ExitStack
from pathlib import Path

import pytest
from _pytest.terminal import TerminalReporter
from insight_stand import (
    ADMIN_ROLE,
    LEAD_ROLE,
    Manifest,
    ManifestError,
    persona_password,
    verify_realm_roles,
)
from insight_stand.session import DEFAULT_MAX_SESSION_AGE_S
from playwright.sync_api import Browser, Page, StorageState, expect
from pytest_playwright.pytest_playwright import CreateContextCallback

from .catalogue import PageScenario
from .page_watch import USAGE_PATH, PageWatch
from .pages.keycloak_login_page import KeycloakLoginPage
from .pages.login_page import LoginPage
from .pages.portal_shell import PortalShell
from .steps import step

# Playwright's own defaults are already generous where it matters — 30s for
# actions and navigation — so they are left alone. `expect()` is the exception:
# its 5s default is tight for a cold SPA that renders after an OIDC round trip,
# and raising it is what lets the journeys use web-first assertions instead of
# sleeping or retrying.
expect.set_options(timeout=15_000)
_NAVIGATION_EXTRAS = pytest.StashKey[list[str]]()


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    chosen = config.getoption("--stand-manifest")
    try:
        manifest = Manifest.load(Path(chosen) if chosen else None)
    except ManifestError as exc:
        raise pytest.UsageError(f"stand manifest unusable: {exc}") from exc
    for item in items:
        if not item.path.is_relative_to(Path(__file__).parent):
            continue
        callspec = getattr(item, "callspec", None)
        scenario = callspec.params.get("scenario") if callspec else None
        if not isinstance(scenario, PageScenario) or scenario.capability is None:
            continue
        if not manifest.has_capability(scenario.capability):
            item.add_marker(
                pytest.mark.skip(
                    reason=(
                        f"requires capability {scenario.capability!r}: no existing definition "
                        f"declared for this first-item visit ({manifest.source_path})"
                    )
                )
            )


def pytest_terminal_summary(terminalreporter: TerminalReporter) -> None:
    extras = terminalreporter.config.stash.get(_NAVIGATION_EXTRAS, None)
    if extras is None:
        return
    terminalreporter.write_sep("-", "Navigation entries absent from the catalogue")
    for entry in extras or ["None"]:
        terminalreporter.write_line(entry)


@pytest.fixture
def navigation_extras(pytestconfig: pytest.Config) -> list[str]:
    extras: list[str] = []
    pytestconfig.stash[_NAVIGATION_EXTRAS] = extras
    return extras


@pytest.fixture(scope="session")
def browser_state_for(
    browser: Browser, stand_manifest: Manifest, base_url: str
) -> Callable[[str], StorageState]:
    states: dict[str, tuple[float, StorageState]] = {}

    def state_for(name: str) -> StorageState:
        person = stand_manifest.fixture(name)
        verify_realm_roles(person)
        if name == "ceo" and not {ADMIN_ROLE, LEAD_ROLE}.issubset(person.realm_roles):
            raise pytest.UsageError("portal smoke requires a persona with admin and lead roles")

        cached = states.get(person.email)
        if cached and time.monotonic() - cached[0] < DEFAULT_MAX_SESSION_AGE_S:
            return cached[1]

        with step(f"Sign in to Insight as the {name} persona"):
            context = browser.new_context(base_url=base_url, service_workers="block")
            context.route(f"**{USAGE_PATH}", lambda route: route.fulfill(status=204))
            try:
                page = context.new_page()
                login = LoginPage(page)
                response = login.start()
                assert 300 <= response.status < 400, "Login must redirect to the IdP"
                location = response.headers.get("location")
                assert location, "Login redirect must name the authorize endpoint"
                login.open_password_form(location, base_url)
                KeycloakLoginPage(page).fill_and_submit(
                    person.email, persona_password(person.email)
                )
                with step("Verify the Insight portal opened"):
                    expect(PortalShell(page).rail.zones().first).to_be_visible(timeout=30_000)
                states[person.email] = (time.monotonic(), context.storage_state())
            finally:
                context.close()

        return states[person.email][1]

    return state_for


@pytest.fixture
def browser_page_for(
    browser_state_for: Callable[[str], StorageState], new_context: CreateContextCallback
) -> Callable[[str], Page]:
    def page_for(name: str) -> Page:
        state = browser_state_for(name)
        context = new_context(storage_state=state, service_workers="block")
        context.route(f"**{USAGE_PATH}", lambda route: route.fulfill(status=204))
        return context.new_page()

    return page_for


@pytest.fixture
def portal_smoke_for(
    browser_page_for: Callable[[str], Page], base_url: str
) -> Iterator[Callable[[str], tuple[PortalShell, PageWatch]]]:
    with ExitStack() as observers:

        def portal_for(name: str) -> tuple[PortalShell, PageWatch]:
            page = browser_page_for(name)
            watch = PageWatch(page.context, base_url)
            observers.enter_context(watch.observing(os.environ.get("INSIGHT_FORCE_500") or None))
            return PortalShell(page), watch

        yield portal_for
