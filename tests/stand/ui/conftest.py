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

import pytest
from playwright.sync_api import BrowserContext, expect

# Playwright's own defaults are already generous where it matters — 30s for
# actions and navigation — so they are left alone. `expect()` is the exception:
# its 5s default is tight for a cold SPA that renders after an OIDC round trip,
# and raising it is what lets the journeys use web-first assertions instead of
# sleeping or retrying.
expect.set_options(timeout=15_000)

#: Verified on the live compose stand: with this set, the rail drops Scorecard
#: and every pane lists only entries that render.
SHOW_PLANNED_OFF = "window.localStorage.setItem('insight.portal.showPlanned', 'false')"

LEGACY_SHELL = "window.localStorage.setItem('insight.portal', 'false')"


def apply_portal_prefs(context: BrowserContext) -> None:
    """What the `context` fixture pins, for a journey that builds its own context.

    A module-scoped context cannot ask for the function-scoped fixture below, and
    a second copy of the flag names is a second place to drift.
    """
    context.add_init_script(SHOW_PLANNED_OFF)


@pytest.fixture
def context(context: BrowserContext, request: pytest.FixtureRequest) -> BrowserContext:
    """A journey drives the shell a customer gets, which is the portal.

    The portal is opt-out — an ABSENT `insight.portal` key renders it — so the
    suite pinning it to `'false'` left every journey asserting a surface no
    install shows by default. Nothing pins it here any more: a fresh context
    boots the shell the product ships, and `/` redirects to `/portal`.

    `insight.portal.showPlanned` IS pinned, to `'false'`. It defaults to on
    (`readBoolPref` returns true for anything that is not literally `"false"`),
    which lists not-yet-built zones and pane entries beside the real ones —
    measured on a mock-mode build, the rail offers Scorecard, whose zone carries
    `readiness: "unbuilt"`, and hiding planned entries drops it. With them
    hidden, what the rail and pane render IS the built set, so a journey can
    enumerate navigation by walking it instead of hard-coding a list that rots.

    `legacy_shell` pins the old flag back for journeys still written against the
    retired dashboard. Those modules are the porting backlog, and the marker is
    how a reader tells "not ported yet" from "deliberately about the legacy
    shell"; it is not a configuration to write a new journey against.

    Both scripts run before any app code on every page, which is the one moment
    a flag is guaranteed to precede the app's first read of it.
    """
    if request.node.get_closest_marker("legacy_shell") is not None:
        context.add_init_script(LEGACY_SHELL)
        return context
    apply_portal_prefs(context)
    return context
