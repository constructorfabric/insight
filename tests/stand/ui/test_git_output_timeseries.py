"""Journey 5 — Git output exposes its repository timeseries as a table and chart.

Why this is a browser test and not an API test: the analytics contract proves
that grouped timeseries data is returned, but it cannot prove that the deployed
SPA opens the Git output dialog, defaults its first generic timeseries block to
the table presentation, switches presentations, or produces browser downloads.
Those behaviors exist only after the component renders and handles user input.

Metric values are not asserted against anything this suite invented, because the
stand manifest declares no golden metrics. What is asserted is a reconciliation:
the numbers in both downloaded files are the numbers the deployed table renders,
which holds whatever the seed contains — including the footer split the formats
keep deliberately, totals in the workbook and none in the CSV.
"""

from __future__ import annotations

import re
from collections.abc import Callable
from pathlib import Path

import pytest
from insight_stand import PersonaSession
from playwright.sync_api import Page, expect

from .downloads import Table, download_export, rendered_rows
from .flows import sign_in
from .pages.person_view import PersonView

# Quality vector of this module's tests.
pytestmark = pytest.mark.reliability

#: A row of the timeseries proper, as opposed to a header or a totals row: it
#: leads with the bucket it covers.
BUCKET = re.compile(r"\d{4}-\d{2}-\d{2}")

#: The two footer rows the table renders under the series.
FOOTERS = ("Total", "Grand total")


def timeseries_rows(exported: Table) -> Table:
    return [row for row in exported if row and BUCKET.fullmatch(row[0])]


def labelled_row(table: Table, label: str) -> list[str]:
    matches = [row for row in table if row and row[0] == label]
    assert len(matches) == 1, f"expected one {label!r} row, found {len(matches)}"

    return matches[0]


def numbers(text: str) -> list[str]:
    """Every number in a rendered cell, digit-grouping and signs removed."""
    return [match.replace(",", "") for match in re.findall(r"[\d,]+", text)]


def assert_same_counts(shown: list[str], written: list[str]) -> None:
    """One rendered row against its exported one, read as numbers in order.

    Not cell against column: the export writes one column per metric
    (`model.columns` x `model.metrics`) while the table may render several of
    them in a single cell — a signed lines pair is one cell against two
    columns — and both visit the metrics in the same order.

    Absence lines up as well. A point the response does not carry renders as a
    dash, which `rendered_rows` normalizes to empty, and exports as an empty
    cell; neither yields a number. And a metric missing from the response drops
    its column from both sides at once, because the table resolves its columns
    against the same `model.metrics` the export iterates.

    Reading the row as one sequence is what keeps a column added to the block
    from turning this into a positional puzzle again.
    """
    label, *cells = shown
    assert numbers(" ".join(cells)) == numbers(" ".join(written[1:])), (
        f"{label}: the numbers on screen are not the ones exported"
    )


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
    # Each total and the default-branch reading beside it: the pair is the point
    # of the block, and a column silently dropped for a metric the response did
    # not carry would otherwise read as a passing table.
    for heading in (
        "Commits",
        "Commits (default)",
        "PRs merged",
        "PRs merged (default)",
        "Lines",
        "Lines (default)",
    ):
        expect(table.get_by_role("columnheader", name=heading).first).to_be_visible()
    expect(table.get_by_role("cell", name="Total", exact=True)).to_be_visible()
    expect(table.get_by_role("cell", name="Grand total", exact=True)).to_be_visible()

    exported: dict[str, Table] = {}
    for menu_item, suffix in (("CSV (.csv)", ".csv"), ("Excel (.xlsx)", ".xlsx")):
        git_output.export().click()
        filename, table_out = download_export(page, menu_item, into=tmp_path, exact=False)
        assert filename.startswith("output-by-repository_"), filename
        assert filename.endswith(suffix), filename
        exported[suffix] = table_out

    assert timeseries_rows(exported[".csv"]) == timeseries_rows(exported[".xlsx"]), (
        "the two formats are written by different code and must still carry the same series"
    )

    on_screen = rendered_rows(table)
    repository = on_screen[0][1]
    assert exported[".csv"][0][0] == on_screen[0][0], (
        "exported bucket header differs from the table's"
    )
    assert all(column.startswith(f"{repository} —") for column in exported[".csv"][0][1:]), (
        f"csv columns are not the rendered repository's: {exported['.csv'][0]}"
    )
    assert repository in exported[".xlsx"][0], (
        f"xlsx columns lost the repository: {exported['.xlsx'][0]}"
    )

    rendered_series = [row for row in on_screen if row and BUCKET.fullmatch(row[0])]
    assert rendered_series, "no bucket rows on screen to reconcile the export against"
    for shown, written in zip(rendered_series, timeseries_rows(exported[".csv"]), strict=True):
        assert_same_counts(shown, written)

    # The two formats carry the footers differently on purpose: the CSV is the
    # machine-readable copy and stops at the last bucket, while the workbook
    # reproduces what the table shows. Asserting the split keeps a regression in
    # either direction visible instead of silently accepted.
    assert not [row for row in exported[".csv"] if row and row[0] in FOOTERS], (
        f"csv gained a footer row, which its consumers parse as data: {exported['.csv']}"
    )
    assert_same_counts(labelled_row(on_screen, "Total"), labelled_row(exported[".xlsx"], "Total"))
    assert numbers(labelled_row(on_screen, "Grand total")[1]) == numbers(
        labelled_row(exported[".xlsx"], "Grand total")[1]
    ), "the workbook's grand totals are not the ones the table renders"

    git_output.chart_view().click()
    expect(git_output.metric_selector()).to_be_visible()
    expect(table).not_to_be_visible()
