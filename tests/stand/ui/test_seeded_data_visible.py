"""Journey 2 — a person signs in and sees THEIR OWN seeded organisation.

Why this is a browser test and not an API test: `/v1/subchart` already proves the
API knows who reports to a lead, and `tests/stand/api/identity/` asserts
exactly that. What no API call can show is that the deployed SPA takes that
answer, renders it in the signed-in person's view, and renders it as navigable
links. A stand where identity is perfect and the frontend renders an empty shell
passes every API test in this repository.

**No metric number is asserted here, deliberately.** Real values are on screen —
the personal view shows a dozen populated tiles — and asserting one would mean
hand-authoring an expected metric value, which this phase forbids anywhere under
`tests/stand/`. It cannot be sourced from the manifest either: `golden_metrics`
is empty by design. So every seeded fact
asserted here is an IDENTITY fact read from the manifest at runtime — who the
person is, and who the roster places under them.

The dashboard coverage checks every KPI and domain card by its visible product
label, verifying that every seeded domain renders as populated.
"""

from __future__ import annotations

from collections.abc import Callable

import pytest
from insight_stand import Manifest, PersonaSession
from playwright.sync_api import Page, expect

from .flows import sign_in
from .pages.landing_page import LandingPage
from .pages.person_view import PersonView
from .pages.team_view import TeamView


@pytest.mark.requires_seed("dev_lead", "development_ic")
@pytest.mark.reliability
def test_the_landing_view_shows_the_persona_and_their_reports(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    """The signed-in person's own name heads the view, and a report is a link.

    Both expectations come from the manifest: the persona's `display_name`, and a
    person the seeded roster places under them. Nothing is typed from a prior
    run's observed output.
    """
    persona = session_for("dev_lead")
    report = stand_manifest.fixture("development_ic")

    sign_in(page, base_url, persona)

    landing = LandingPage(page)
    expect(landing.person_heading(persona.person.display_name)).to_be_visible()

    # Present AND pointing at that person. Existence alone would pass if every
    # report's link resolved to the same view, or to the signed-in person's own
    # — a plausible regression, since the SPA builds these hrefs from a field it
    # could pick wrongly. The key is the canonical person id since the identity
    # cutover (#2098); the expected path is composed by the page object from the
    # manifest's uuid, so nothing here hardcodes a URL.
    link = landing.person_link(report.display_name)
    expect(link).to_be_visible()
    expect(link).to_have_attribute("href", PersonView.path(report.uuid))


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.versatility
def test_the_personal_dashboard_renders_every_metric_domain(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """The person's dashboard renders every KPI and domain in its seeded state."""
    persona = session_for("dev_lead")

    sign_in(page, base_url, persona)

    view = PersonView(page)
    view.go(persona.person.uuid)
    expect(view.person_heading(persona.person.display_name)).to_be_visible()

    # The dev lead's first KPI_ROW_MAX (4) observed candidates, in KPI_ROW
    # order — the row fills its four-column line, later candidates stay off.
    for label in (
        "Issues closed",
        "Focus Time",
        "Pull requests merged",
        "AI active days",
    ):
        expect(view.kpi_tile(label)).to_be_visible()
        expect(view.kpi_value(label)).not_to_have_text("—")

    for label in ("Task delivery", "Git output", "Collaboration", "AI adoption", "Wiki"):
        expect(view.populated_domain_card(label)).to_be_visible()


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_the_team_view_lists_every_report_the_roster_declares(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    """Not "somebody rendered" — every single person the manifest puts on the team.

    The strongest seeded-data assertion available without touching a metric
    value. The roster is derived from the manifest at runtime (everyone whose
    `team` matches the lead's and whose role is `ic`), so a reshuffled seed moves
    the expectation with it and nothing here is typed from a prior run.

    Asserting ALL of them rather than one matters: a view that renders the first
    report and silently drops the rest — a pagination default, a truncated
    query, a broken key — passes a spot check and fails a person looking for
    their own team.

    Each member is located by their TABLE ROW rather than by a link bearing their
    name: the sidebar carries every person in the org scope on every view, so a
    name-based locator would pass against an empty team table. `TeamView.member_row`
    has the measurement.
    """
    persona = session_for("dev_lead")
    lead = persona.person
    reports = sorted(
        p.display_name for p in stand_manifest.personas if p.team == lead.team and p.role == "ic"
    )
    assert reports, (
        f"the manifest places nobody under {lead.display_name} on team {lead.team!r}, "
        "so this test would assert nothing"
    )

    sign_in(page, base_url, persona)

    personal = PersonView(page)
    personal.go(lead.uuid)
    expect(personal.person_heading(lead.display_name)).to_be_visible()

    team_switch = personal.team_view_switch()
    expect(team_switch).to_be_visible()
    team_switch.click()

    team = TeamView(page)
    expect(page).to_have_url(f"{base_url}{TeamView.path(lead.uuid)}")
    expect(team.team_heading(lead.display_name)).to_be_visible()
    expect(team.metrics_overview()).to_be_visible()

    # Every member renders a cell for every column (recorded or an honest
    # "not recorded") and at least one real recorded value — this catches a
    # dropped member, a blank row, or a truncated column, without demanding
    # every metric for every person. A member can legitimately close issues
    # yet close no bugs, so "Bugs closed: not recorded" for one member is data,
    # not a defect.
    for name in reports:
        row = team.member_row(name)
        expect(row).to_be_visible()
        for metric_label in (
            "Bugs closed",
            "Non-bug issues closed",
            "Time to resolution",
            "Pull requests merged",
            "PR cycle time",
            "Focus Time",
            "Meeting Hours",
            "AI active days",
            "Page edits",
        ):
            expect(team.metric_cell(name, metric_label)).to_be_visible()
        expect(team.any_recorded_metric_cell(name)).to_be_visible()

    for label in ("Task delivery", "Git output", "Collaboration", "AI adoption", "Wiki"):
        card = team.domain_card(label)
        expect(card).to_be_visible()
        expect(card).not_to_contain_text("No metrics with peer data for this period.")
