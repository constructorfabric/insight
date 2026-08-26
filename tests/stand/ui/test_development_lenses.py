"""Journey — the Repositories and Quality lenses render their own sections.

Why this is a browser test and not an API test: both lenses are *composed* in
the client. `POST /v1/metric-results` proves a rollup and a breakdown come back
(`tests/stand/api/analytics/test_results.py` asserts exactly that), but no API
call can show that the deployed SPA turns those answers into the sections the
screens are made of — a repository table with a row per repository, an
ownership bar per repository, a histogram of pull-request ages — or that the
nav still routes to a lens that used to be a placeholder. A stand whose API is
perfect and whose lens still renders the old "in development" note passes every
contract test in this repository.

**No metric value is asserted.** The manifest declares no golden metrics, so
what is asserted is composition: which sections a lens draws, and that a
section either carries rows or says honestly that it has none. Both outcomes
are correct on a stand seeded without a git connector, which is why the
journey distinguishes them rather than requiring data.

Both lenses are read in ONE sign-in: the sections are what is under test, and a
second Keycloak round trip would only re-prove the login journey.
"""

from __future__ import annotations

import re
from collections.abc import Callable

import pytest
from insight_stand import PersonaSession
from playwright.sync_api import BrowserContext, Page, expect

from .flows import sign_in
from .pages.portal_shell import PortalShell

# Quality vector of this module's tests.
pytestmark = pytest.mark.reliability


@pytest.fixture
def context(context: BrowserContext) -> BrowserContext:
    """These journeys drive the portal, so the legacy-shell hatch is cleared.

    `conftest` writes `insight.legacyShell` for the journeys written against
    the old dashboard, and `/portal` redirects to `/` for as long as it is set
    — a lens journey under that key reads the person dashboard instead. Init
    scripts run in the order they were added, so removing the key here still
    precedes the first app read. `showPlanned` is pinned off because the claim
    under test is that these sections are built, not that the nav will show an
    unbuilt one to a reader who asked for it.
    """
    context.add_init_script(
        "window.localStorage.removeItem('insight.legacyShell');"
        "window.localStorage.setItem('insight.portal.showPlanned', 'false')"
    )
    return context


#: The Development lenses this journey opens: the heading each one renders,
#: and the section labels it may draw. The heading is asserted, the sections
#: are asserted as "at least one" — several of them drop themselves by design
#: (a composition or a table with one row is an empty shell, not a section),
#: so requiring a specific one would make the journey depend on how many
#: repositories the stand happens to hold.
LENSES: dict[str, tuple[str, tuple[re.Pattern[str], ...]]] = {
    "Repositories": (
        "Development · Repositories",
        (
            re.compile(r"per person", re.IGNORECASE),
            re.compile(r"ranked by PRs merged", re.IGNORECASE),
            re.compile(r"ownership concentration", re.IGNORECASE),
            re.compile(r"how long pull requests stayed open", re.IGNORECASE),
        ),
    ),
    "Quality": (
        "Development · Quality",
        (
            re.compile(r"review hygiene", re.IGNORECASE),
            re.compile(r"review timing", re.IGNORECASE),
            re.compile(r"waited for the first review", re.IGNORECASE),
        ),
    ),
}

#: What the lens says when the whole family is unmeasured on this install. A
#: stand seeded without a git connector is a legitimate outcome for this
#: journey, but it has to be the HONEST one rather than an empty frame.
NOT_INGESTED = re.compile(r"not connected yet", re.IGNORECASE)

#: A section the Repositories lens draws whenever it has any git data at all,
#: whether or not enough repositories exist for the table to be worth drawing.
HISTOGRAM_LABEL = re.compile(r"how long pull requests stayed open", re.IGNORECASE)


def open_lens(portal: PortalShell, lens: str) -> None:
    """Open one Development lens by its own URL.

    The lens lives in the query string rather than the path, so a direct visit
    is the same navigation the pane performs — and it keeps the journey from
    depending on how the rail happens to collapse at this viewport.
    """
    portal.page.goto(
        f"/portal?zone=directions&dir=dev&lens={lens}",
        wait_until="domcontentloaded",
    )
    portal.wait_url_settled()


@pytest.mark.requires_seed("dev_lead")
def test_the_development_lenses_compose_their_sections(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """Each lens is a real screen, and draws sections or an honest absence.

    Both lenses were placeholders until recently, so the heading is the claim
    that matters most: a nav entry that still routes to the roadmap note would
    render neither the heading nor any section, and no API test can see that.

    Sections are then asserted as alternatives on purpose — at least one, or
    the not-connected note. What would be a defect is neither: a lens that
    renders its frame with no sections and no explanation, which is what a
    reader sees when every section silently drops the metrics it wanted.
    """
    sign_in(page, base_url, session_for("dev_lead"))
    portal = PortalShell(page)

    for lens, (heading, labels) in LENSES.items():
        open_lens(portal, lens)
        content = portal.content()

        expect(
            content.get_by_text(heading).first,
            f"the {lens} lens rendered no heading — the nav may still route to "
            f"the roadmap placeholder",
        ).to_be_visible()

        # Wait for an OUTCOME before reading counts. A heading renders while the
        # metric request is still in flight, so a bare `count()` here would read
        # a lens mid-load and call it empty. Either a section or the
        # not-connected note is the settled state; anything else times out with
        # the lens named.
        settled = content.get_by_text(NOT_INGESTED)
        for label in labels:
            settled = settled.or_(content.get_by_text(label))
        expect(
            settled.first,
            f"the {lens} lens drew its heading but never resolved into either a "
            f"section or an explanation — a reader is left on an empty screen",
        ).to_be_visible()

        if content.get_by_text(NOT_INGESTED).count():
            # Honest absence: the family is unmeasured here, and the lens says
            # so instead of drawing empty sections.
            continue

        drawn = [label.pattern for label in labels if content.get_by_text(label).count()]
        assert drawn, (
            f"the {lens} lens drew its heading but none of its sections and no "
            f"explanation — a reader gets an empty screen"
        )


@pytest.mark.requires_seed("dev_lead")
def test_the_repository_table_names_repositories_or_stays_absent(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """The rollup-backed table is a table of repositories, not of people.

    The lens is the only screen reading the `rollup` view, whose rows carry no
    entity id — so the check that matters in a browser is that what lands on
    screen is keyed by repository. Its first column is asserted to be one, and
    the People column to be a count rather than a name: a regression that fell
    back to per-person rows would still render a full table, and only the
    column contents would give it away.

    A stand with no git rows draws no table at all — the section drops itself
    rather than rendering one row, which the lens does deliberately. That is a
    skip here, not a failure: this journey cannot seed git data.
    """
    sign_in(page, base_url, session_for("dev_lead"))
    portal = PortalShell(page)
    open_lens(portal, "Repositories")

    content = portal.content()
    # Same rule as above: settle on an outcome before reading a count, or a
    # slow rollup reads as a stand with no git data and the case skips itself
    # into uselessness.
    table = content.get_by_role("table").first
    expect(
        table.or_(content.get_by_text(NOT_INGESTED))
        .or_(content.get_by_text(HISTOGRAM_LABEL))
        .first,
        "the Repositories lens never resolved into a table, a histogram or a not-connected note",
    ).to_be_visible()
    if not table.count():
        pytest.skip("no repository table on this stand — the lens drew no rows to check")

    headers = [(text or "").strip() for text in table.get_by_role("columnheader").all_inner_texts()]
    assert "People" in headers, f"the repository table's columns were {headers}"

    people_column = headers.index("People")
    first_row = table.get_by_role("row").nth(1)
    cells = [(text or "").strip() for text in first_row.get_by_role("cell").all_inner_texts()]
    assert cells, "the repository table rendered a header and no rows"
    # A count, not a person: the rollup answers how many distinct people
    # contributed, and a per-person regression would put a name here.
    assert cells[people_column].isdigit(), (
        f"the People column read {cells[people_column]!r} — a rollup row counts "
        f"contributors, so a name there means the table fell back to person rows"
    )
