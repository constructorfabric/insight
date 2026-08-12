"""Journey 5 — inspect and export evidence behind a Git timeseries value.

Why this is a browser test and not an API test: the analytics suite exercises
the evidence endpoints directly, but it cannot prove that a user can traverse
the deployed SPA from the personal dashboard into Git output, select a concrete
repository-and-time bucket, and receive the nested supporting-data dialog with
working browser downloads.
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

# Quality vector of this module's tests.
pytestmark = pytest.mark.reliability


@pytest.mark.requires_seed("dev_lead")
def test_git_commit_bucket_opens_and_exports_supporting_data(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
    tmp_path: Path,
) -> None:
    persona = session_for("dev_lead")
    page.context.grant_permissions(["clipboard-write"], origin=base_url)
    sign_in(page, base_url, persona)

    person = PersonView(page)
    person.go(persona.person.uuid)
    expect(person.person_heading(persona.person.display_name)).to_be_visible()

    git_output = person.open_git_output()
    expect(git_output.dialog).to_be_visible()
    expect(git_output.repository_table()).to_be_visible()

    evidence = git_output.open_first_commit_bucket()
    expect(evidence.dialog).to_be_visible()

    table = evidence.table()
    expect(table).to_be_visible()
    for heading in ("Ref", "Repository", "Author", "Lines added", "Lines removed", "Date"):
        expect(table.get_by_role("columnheader", name=heading)).to_be_visible()
    expect(table).to_have_attribute("aria-rowcount", re.compile(r"^[1-9]\d*$"))
    expect(table.get_by_role("cell", name=re.compile("insight", re.I)).first).to_be_visible()

    copy_ref = evidence.copy_ref().first
    expect(copy_ref).to_be_visible()
    copy_ref.click()
    expect(evidence.dialog.get_by_role("button", name="Copied")).to_be_visible()

    for menu_item, suffix in (("CSV", ".csv"), ("Excel", ".xlsx")):
        evidence.export().click()
        with page.expect_download() as download_info:
            page.get_by_role("menuitem", name=menu_item, exact=True).click()
        download = download_info.value
        assert download.suggested_filename.endswith(suffix)
        destination = tmp_path / download.suggested_filename
        download.save_as(destination)
        assert destination.stat().st_size > 0
