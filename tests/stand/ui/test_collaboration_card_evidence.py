"""Journey 6 — supporting data from a metric card's overflow menu.

Why this is a browser test and not an API test: the analytics suite reconciles
every metric's evidence against its value directly, so nothing about the payload
needs a browser. What needs one is the affordance. A group whose drilldown body
carries no timeseries block — Collaboration is the only one — reaches evidence
solely through a card's `⋯` menu, and that menu is the whole path: the card has to
be told the metric supports drilldown, the item has to build a selection from the
card's own metric, and the dialog has to open over the group dialog already on
screen rather than replacing it.

The shape asserted here is the summary grain's, and it is the complement of the
Git journey's: a day and a number, no record reference to copy, because a
summary-grain source has no per-event rows to point at.
"""

from __future__ import annotations

import re
from collections.abc import Callable
from pathlib import Path

import pytest
from insight_stand import PersonaSession
from playwright.sync_api import Page, expect

from .flows import sign_in
from .pages.person_view import PersonView

#: A chat metric rather than a file or meeting one: the label is the server's,
#: and this is the one the group's card leads with.
MESSAGES_SENT = "Messages Sent"


@pytest.mark.requires_seed("dev_lead")
def test_collaboration_card_menu_opens_and_exports_supporting_data(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
    tmp_path: Path,
) -> None:
    persona = session_for("dev_lead")
    sign_in(page, base_url, persona)

    person = PersonView(page)
    person.go(persona.person.uuid)
    expect(person.person_heading(persona.person.display_name)).to_be_visible()

    collaboration = person.open_domain("Collaboration")
    expect(collaboration.dialog).to_be_visible()

    actions = collaboration.card_actions(MESSAGES_SENT)
    expect(actions).to_be_visible()
    evidence = collaboration.open_card_evidence(actions)
    expect(evidence.dialog).to_be_visible()

    table = evidence.table()
    expect(table).to_be_visible()
    expect(evidence.column_header("Date")).to_be_visible()
    expect(evidence.column_header("Value")).to_be_visible()
    expect(table).to_have_attribute("aria-rowcount", re.compile(r"^[1-9]\d*$"))
    assert evidence.copy_ref().count() == 0, (
        "summary-grain evidence has no per-record reference, so nothing should offer to copy one"
    )

    evidence.export().click()
    with page.expect_download() as download_info:
        page.get_by_role("menuitem", name="CSV", exact=True).click()
    download = download_info.value
    assert download.suggested_filename.endswith(".csv")
    destination = tmp_path / download.suggested_filename
    download.save_as(destination)
    assert destination.stat().st_size > 0
