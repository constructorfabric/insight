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


@pytest.fixture
def context(context: BrowserContext) -> BrowserContext:
    """Every journey drives the legacy dashboard shell, stated explicitly.

    The portal became opt-out (an ABSENT `insight.portal` key renders it), so
    a fresh browser context stopped meaning "the shell these journeys were
    written against" and every selector started timing out inside the portal.
    The suite's tested surface stays the legacy shell until portal journeys
    exist; the init script runs before any app code on every page, which is
    the one moment the flag is guaranteed to precede the first read.
    """
    context.add_init_script("window.localStorage.setItem('insight.portal', 'false')")
    return context
