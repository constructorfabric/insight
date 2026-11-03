"""Soft browser verdicts, kept outside the locator-and-action page objects."""

import time
from collections.abc import Iterator
from contextlib import contextmanager
from urllib.parse import urlsplit

from playwright.sync_api import Error, expect

from .page_watch import PageWatch
from .pages.portal_shell import PortalShell
from .steps import step


class Checks:
    def __init__(self) -> None:
        self.failures: list[str] = []

    @contextmanager
    def check(self, title: str) -> Iterator[None]:
        try:
            with step(title):
                yield
        except (AssertionError, Error) as exc:
            self.failures.append(f"{title}: {exc}")

    def finish(self) -> None:
        assert not self.failures, "\n\n".join(self.failures)


def wait_for_quiet(portal: PortalShell, watch: PageWatch, *, loaded: bool) -> bool:
    deadline = time.monotonic() + 30
    quiet_since: float | None = None
    activity = watch.activity
    while time.monotonic() < deadline:
        target = urlsplit(portal.page.url)
        ready = not watch.pending
        if loaded:
            ready = (
                ready
                and watch.same_origin(portal.page.url)
                and not target.path.startswith(("/auth/", "/kc/"))
                and portal.main.is_visible()
                and portal.loading_markers.count() == 0
            )

        if not ready or activity != watch.activity:
            quiet_since = None
        activity = watch.activity
        if ready:
            quiet_since = quiet_since if quiet_since is not None else time.monotonic()
            if time.monotonic() - quiet_since >= 1:
                return True
        portal.page.wait_for_timeout(50)

    return False


def mark_page_requests(portal: PortalShell, watch: PageWatch) -> None:
    with step("Wait for the previous page's requests to settle"):
        assert wait_for_quiet(portal, watch, loaded=False), (
            f"Previous page requests did not settle within 30 s; "
            f"in-flight paths: {watch.pending_paths()}"
        )
        watch.mark()


def verify_page(portal: PortalShell, watch: PageWatch, test_id: str, page_name: str) -> None:
    checks = Checks()
    with checks.check(f"Verify the {page_name} page is shown"):
        expect(
            portal.page_view(test_id),
            f'Expected page view "{test_id}" to be visible',
        ).to_be_visible(timeout=30_000)

    with checks.check("Verify the page finished loading"):
        assert wait_for_quiet(portal, watch, loaded=True), (
            f"Page did not finish loading within 30 s; "
            f"in-flight paths: {watch.pending_paths()}; "
            f"visible loading markers: {portal.loading_markers.count()}"
        )

    with checks.check("Verify the page shows no error screen or widget errors"):
        details = portal.visible_errors.evaluate_all(
            """elements => elements.map(element => ({
                label: element.getAttribute('aria-label'), text: element.innerText.trim()
            }))"""
        )
        assert not details, f"Visible error screens or widget errors: {details}"

    with checks.check("Verify the stand answered without server errors"):
        assert not watch.failures, "\n".join(watch.failures)

    checks.finish()
