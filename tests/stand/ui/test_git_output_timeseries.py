"""Journey 5 — Git output exposes its repository timeseries as a table and chart.

Why this is a browser test and not an API test: the analytics contract proves
that grouped timeseries data is returned, but it cannot prove that the deployed
SPA opens the Git output dialog, defaults its first generic timeseries block to
the table presentation, switches presentations, or produces browser downloads.
Those behaviors exist only after the component renders and handles user input.

Metric values are not asserted because the stand manifest declares no golden
metrics. The journey instead verifies the stable structure built from seeded Git
activity and that both export paths produce non-empty browser downloads.
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


@pytest.mark.requires_seed("dev_lead")
def test_git_output_repository_timeseries_switches_views_and_downloads(
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

    git_output = person.open_git_output()
    expect(git_output.dialog).to_be_visible()

    table = git_output.table()
    expect(table).to_be_visible()
    expect(
        table.get_by_role("columnheader", name=re.compile("insight", re.I)).first
    ).to_be_visible()
    for heading in ("Commits", "PRs merged", "Lines"):
        expect(table.get_by_role("columnheader", name=heading).first).to_be_visible()
    expect(table.get_by_role("cell", name="Total", exact=True)).to_be_visible()
    expect(table.get_by_role("cell", name="Grand total", exact=True)).to_be_visible()

    for menu_item, suffix in (("CSV (.csv)", ".csv"), ("Excel (.xlsx)", ".xlsx")):
        git_output.export().click()
        with page.expect_download() as download_info:
            page.get_by_role("menuitem", name=menu_item).click()
        download = download_info.value
        assert download.suggested_filename.startswith("output-by-repository_")
        assert download.suggested_filename.endswith(suffix)
        destination = tmp_path / download.suggested_filename
        download.save_as(destination)
        assert destination.stat().st_size > 0

    git_output.chart_view().click()
    expect(git_output.metric_selector()).to_be_visible()
    expect(table).not_to_be_visible()
