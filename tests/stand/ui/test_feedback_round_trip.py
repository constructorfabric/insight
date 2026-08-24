"""Journey — what a reader types into the feedback dialog reaches the operator's table.

Why this is a browser test and not an API test, measured rather than asserted: the
screen a report carries is computed in the page and nowhere else. `api/analytics/
test_feedback.py` sends a submission and reads it back, and the screen it sends is a
constant typed into the test (`SENT_FROM = "/ic/:id/personal"`) — so what it proves is
that the server stores the path it is handed. The dialog does not hand it a constant:
it reads `currentScreen()` from telemetry.ts, which holds whatever the router
subscription in usage-collection.ts last recorded, reduced by `screenPath()` — the one
thing that turns `/ic/<uuid>/personal` into `/ic/:id/personal`. Drive the SPA to a
person's screen and the value below is produced rather than declared.

The way in is browser-only too. `FeedbackDialogProvider` mounts the dialog at the
router root rather than under the control that opens it, because the rail's settings
popover unmounts anything it owns the moment the dialog takes focus. Whether the rail's
button reaches a mounted dialog is a statement about the rendered shell.

Feedback rows cannot be removed — no operation deletes a submission, so `api/scratch.py`'s
create-then-delete policy does not reach them. This journey's message carries
`SCRATCH_PREFIX` and the run tag for the reason the API module's does: a row left on the
stand stays attributable to the run that wrote it.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

import pytest
from api import scratch
from insight_stand import PersonaSession, wait_for
from playwright.sync_api import Browser, BrowserContext, Page, expect

from .flows import collect_rows, revisit_usage, sign_in
from .pages.feedback_dialog import FeedbackDialog
from .pages.platform_usage_page import FEEDBACK_TABLE, PlatformUsagePage
from .pages.portal_shell import recorded_path_of

pytestmark = pytest.mark.reliability

PERSON, MESSAGE, SCREEN = 1, 2, 3

SENT_FROM = "/ic/:id/personal"

SENT_FROM_LABEL = "Person › Personal"  # noqa: RUF001 (screen-label.ts's own separator)


@dataclass(frozen=True)
class Report:
    message: str
    row: list[str]


def _message() -> str:
    return f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG} sent from the feedback dialog"


def _portal_context(browser: Browser, base_url: str) -> BrowserContext:
    """A context the suite's legacy-shell hatch never reaches — the rail and the
    listing are both portal surfaces."""
    return browser.new_context(base_url=base_url)


def _send(page: Page, message: str) -> None:
    """The dialog closes in `onSuccess` alone — a refused send keeps it open and says
    why — so it going away is the browser's own account of an accepted submission."""
    dialog = FeedbackDialog(page)
    dialog.open()
    expect(dialog.dialog()).to_be_visible()
    dialog.message().fill(message)
    dialog.send().click()
    expect(dialog.sent_toast()).to_be_visible()
    expect(dialog.dialog()).not_to_be_visible()


def _row_for(usage: PlatformUsagePage, message: str) -> list[str] | None:
    """A remount, not a reload: reloading re-asks `/auth/me` and a poll trips the
    gateway's `auth_per_ip` limiter. A period holding no feedback renders no table at
    all, so the walk is gated on one existing."""
    revisit_usage(usage.page)
    if usage.table(FEEDBACK_TABLE).count() == 0:
        return None
    for row in collect_rows(usage.table(FEEDBACK_TABLE)):
        if row[MESSAGE] == message:
            return row
    return None


@pytest.fixture(scope="module")
def reported(
    browser: Browser,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> Report:
    """Two contexts: one browser profile holds one session, and only the admin is served
    the listing. Module-scoped so the run leaves one undeletable row rather than one per
    test, and so both assertions describe the same submission."""
    lead = session_for("dev_lead")
    operator = session_for("admin_operator")

    lead_context = _portal_context(browser, base_url)
    lead_page = lead_context.new_page()
    sign_in(lead_page, base_url, lead)
    lead_page.goto(f"/ic/{lead.person.uuid}/personal", wait_until="domcontentloaded")
    recorded = recorded_path_of(lead_page.url)
    assert recorded == SENT_FROM, (
        f"the sender is on {recorded!r}, so the assertion below would not be about "
        f"a redacted person key ({SENT_FROM!r})"
    )

    message = _message()
    _send(lead_page, message)
    lead_context.close()

    admin_context = _portal_context(browser, base_url)
    admin_page = admin_context.new_page()
    sign_in(admin_page, base_url, operator)
    usage = PlatformUsagePage(admin_page)
    usage.go()
    expect(usage.chart_heading()).to_be_visible()
    row = wait_for(
        lambda: _row_for(usage, message),
        timeout_s=45,
        description=f"the report {lead.person.display_name} sent from {SENT_FROM}",
    )
    admin_context.close()

    return Report(message=message, row=row)


@pytest.mark.requires_seed("dev_lead", "admin_operator")
def test_the_listing_names_the_person_who_sent_it(
    reported: Report, session_for: Callable[[str], PersonaSession]
) -> None:
    expected = session_for("dev_lead").person.display_name
    assert reported.row[PERSON] == expected, (
        f"the row for {reported.message!r} names {reported.row[PERSON]!r}, not {expected!r}"
    )


@pytest.mark.requires_seed("dev_lead", "admin_operator")
def test_the_report_names_the_screen_its_sender_was_on(reported: Report) -> None:
    assert reported.row[SCREEN] == SENT_FROM_LABEL, (
        f"the row for {reported.message!r} was sent from {SENT_FROM!r} and is listed "
        f"as {reported.row[SCREEN]!r}, not {SENT_FROM_LABEL!r}"
    )
