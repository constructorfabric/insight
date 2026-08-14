"""Journey 7 — supporting data behind one member's cell on the team heatmap.

Why this is a browser test and not an API test: the API takes a person, a metric
and a period, and the analytics suite already asks it about every metric. What it
cannot show is that a lead looking at somebody else's cell gets THAT person's
evidence. The heatmap builds its selection from the cell — a different person per
row and a different metric per column, on a surface whose scope is the team rather
than the signed-in user — and a selection built from the wrong axis would still
answer 200 with somebody's rows.

The member and the cell both come from the manifest and from the view: the roster
decides who is on the team, and the cell's own accessible name decides which
metric the dialog should be about.
"""

from __future__ import annotations

import re
from collections.abc import Callable

import pytest
from insight_stand import Manifest, PersonaSession
from playwright.sync_api import Page, expect

from .evidence_requests import evidence_selection
from .flows import sign_in
from .pages.person_view import PersonView
from .pages.team_view import TeamView

# Quality vector of this module's tests.
pytestmark = pytest.mark.reliability


@pytest.mark.requires_seed("dev_lead")
def test_team_heatmap_cell_opens_that_members_supporting_data(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    persona = session_for("dev_lead")
    lead = persona.person
    reports = sorted(
        (p for p in stand_manifest.personas if p.team == lead.team and p.role == "ic"),
        key=lambda p: p.display_name,
    )
    assert reports, (
        f"the manifest places nobody under {lead.display_name} on team {lead.team!r}, "
        "so this test would have no cell to open"
    )
    member = reports[0]

    sign_in(page, base_url, persona)

    personal = PersonView(page)
    personal.go(lead.uuid)
    expect(personal.person_heading(lead.display_name)).to_be_visible()
    personal.team_view_switch().click()

    team = TeamView(page)
    expect(page).to_have_url(f"{base_url}{TeamView.path(lead.uuid)}")
    expect(team.metrics_overview()).to_be_visible()

    cell = team.any_recorded_metric_cell(member.display_name)
    expect(cell).to_be_visible()
    with evidence_selection(page) as selection:
        evidence = team.open_cell_evidence(cell, member.display_name)
    expect(evidence.dialog).to_be_visible()

    assert selection["entity"]["id"] == member.uuid, (
        f"the cell belongs to {member.display_name}, and the request asked about "
        f"{selection['entity']}"
    )

    table = evidence.table()
    expect(table).to_be_visible()
    expect(evidence.column_header("Date")).to_be_visible()
    expect(table).to_have_attribute("aria-rowcount", re.compile(r"^[1-9]\d*$"))
    assert evidence.metric_selector().count() == 0, (
        "a cell names one metric, so its dialog must not offer a metric to choose"
    )
