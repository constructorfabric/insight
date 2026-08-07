"""Journey 8 — supporting data from a timeseries block, whole-block and total.

Why this is a browser test and not an API test: the endpoint answers about one
metric at a time, so the two things exercised here exist only in the SPA. A block
that charts several metrics together opens ONE dialog for all of them and lets the
reader switch between them inside it — a branch with its own title, its own
selector and its own query per target. And a table's Total cell asks for the
block's whole period rather than a bucket, which is the one selection whose period
is not the cell's own row.

Both groups the sweep covers here are picked for their block shape rather than
their domain: one block carries several metrics, the other carries one, and the
selector must appear in exactly the first case.
"""

from __future__ import annotations

import re
from collections.abc import Callable

import pytest
from insight_stand import PersonaSession
from playwright.sync_api import Page, expect

from .evidence_requests import evidence_selection
from .flows import sign_in
from .pages.person_view import PersonView

#: The first column of the Task delivery table, so the Total cell opened below
#: names this metric. The label is the server's.
TASKS_CLOSED = "Issues closed"

#: A dialog opened for several metrics at once titles itself with all of their
#: labels joined, and that joining is the assertion — no single label would do.
_JOINED_TITLE = re.compile(r" & ")


@pytest.mark.requires_seed("dev_lead")
def test_multi_metric_block_offers_every_metric_in_one_dialog(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """Git output, because it is one of the few blocks plotting several metrics.

    The choice of block is the whole precondition: a block's evidence covers
    every metric it plots, so a single-metric block can only ever open a
    single-metric dialog no matter what else the journey does. Git output plots
    four, and all four advertise a drilldown capability, so the joined dialog
    this asserts is reachable at all.
    """
    persona = session_for("dev_lead")
    sign_in(page, base_url, persona)

    person = PersonView(page)
    person.go(persona.person.uuid)
    expect(person.person_heading(persona.person.display_name)).to_be_visible()

    block = person.open_domain("Git output")
    expect(block.dialog).to_be_visible()

    # The block plots four metrics but groups by repository, so the chart shows
    # one at a time. Table presentation is what makes the block's evidence cover
    # every metric it plots. Clicking an already-active toggle is a no-op, and
    # the table's presence is what proves the state — the choice is persisted
    # per block in localStorage, so it is established here, never assumed.
    block.table_view().click()
    expect(block.block_table()).to_be_visible()

    with evidence_selection(page) as opened_selection:
        block.evidence_button().click()

    evidence = block.evidence_for(_JOINED_TITLE)
    expect(evidence.dialog).to_be_visible()

    selector = evidence.metric_selector()
    expect(selector).to_be_visible()
    opened_with = selector.inner_text().strip()

    selector.click()
    options = page.get_by_role("option")
    labels = [label.strip() for label in options.all_inner_texts()]
    assert len(labels) > 1, f"a joined dialog must list every metric it was opened for: {labels}"
    other = next(label for label in labels if label != opened_with)

    with evidence_selection(page) as switched_selection:
        page.get_by_role("option", name=other, exact=True).click()
    expect(selector).to_contain_text(other)
    expect(evidence.table()).to_be_visible()
    expect(evidence.column_header("Date")).to_be_visible()

    assert switched_selection["metric_key"] != opened_selection["metric_key"], (
        f"choosing {other} left the dialog querying {opened_selection['metric_key']}, so the "
        "table below the selector is the previous metric's"
    )


@pytest.mark.requires_seed("dev_lead")
def test_single_metric_block_opens_that_metrics_dialog(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    persona = session_for("dev_lead")
    sign_in(page, base_url, persona)

    person = PersonView(page)
    person.go(persona.person.uuid)
    expect(person.person_heading(persona.person.display_name)).to_be_visible()

    ai = person.open_domain("AI adoption")
    expect(ai.dialog).to_be_visible()

    heading = ai.block_metric_heading().first
    expect(heading).to_be_visible()
    metric = heading.inner_text().strip()

    ai.evidence_button().click()
    evidence = ai.evidence_for(metric)
    expect(evidence.dialog).to_be_visible()
    expect(evidence.table()).to_be_visible()
    expect(evidence.column_header("Date")).to_be_visible()
    assert evidence.metric_selector().count() == 0, (
        f"{metric} is the block's only metric, so its dialog must offer nothing to switch to"
    )


@pytest.mark.requires_seed("dev_lead")
def test_table_total_opens_supporting_data_for_the_whole_period(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """A total is the one cell whose period is the block's, not a row's.

    Asserted against a body row from the same table rather than against a date
    typed here: the total's period has to contain the bucket's and to be wider
    than it, which is exactly what sending a bucket for the total would break.
    """
    persona = session_for("dev_lead")
    sign_in(page, base_url, persona)

    person = PersonView(page)
    person.go(persona.person.uuid)
    expect(person.person_heading(persona.person.display_name)).to_be_visible()

    tasks = person.open_domain("Task delivery")
    expect(tasks.dialog).to_be_visible()
    tasks.table_view().click()
    expect(tasks.block_table()).to_be_visible()

    with evidence_selection(page) as bucket_selection:
        bucket = tasks.open_bucket_evidence(TASKS_CLOSED)
    expect(bucket.dialog).to_be_visible()
    bucket.close().click()
    expect(bucket.dialog).not_to_be_visible()

    with evidence_selection(page) as total_selection:
        evidence = tasks.open_total_row_evidence(TASKS_CLOSED)
    expect(evidence.dialog).to_be_visible()

    table = evidence.table()
    expect(table).to_be_visible()
    expect(evidence.column_header("Ref")).to_be_visible()
    expect(evidence.column_header("Date")).to_be_visible()
    expect(table).to_have_attribute("aria-rowcount", re.compile(r"^[1-9]\d*$"))

    total_period = total_selection["period"]
    bucket_period = bucket_selection["period"]
    assert total_period["from"] <= bucket_period["from"], (total_period, bucket_period)
    assert total_period["to"] >= bucket_period["to"], (total_period, bucket_period)
    assert total_period != bucket_period, (
        f"the Total row asked for {total_period}, the same period as a single bucket"
    )
