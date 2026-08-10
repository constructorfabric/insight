"""A metric group's details dialog, and the evidence dialog it opens.

One object for all five groups, because the SPA renders one component for all
five: the dialog's accessible name is the group's title, and everything inside it
is composed from the same metric widgets. A per-group object would be five copies
of these locators differing only in a string.

The evidence dialog is the second, nested one. Its accessible name is the metric
label the API served, so a caller either passes the label it read from the view or
a pattern — never a label this suite invented, since the wording is the server's.

Accessibility-first, like every page object here: the published SPA carries no
`data-testid` attributes, so roles and accessible names are the only stable
handles. Two exceptions, both unavoidable and both narrow: `[data-slot="card"]`
to scope a locator to the block that owns a control, and positional row/cell
indexes where a value button's only accessible name is the number it renders.
"""

from __future__ import annotations

import re

from playwright.sync_api import Locator, Page

#: The evidence affordance every metric widget shares, whether it hangs off a
#: card's overflow menu or a timeseries block's header.
SUPPORTING_DATA = "View supporting data"


class MetricEvidenceDialog:
    def __init__(self, page: Page, metric: str | re.Pattern[str]) -> None:
        self.page = page
        self.dialog = page.get_by_role("dialog", name=metric)

    def table(self) -> Locator:
        return self.dialog.get_by_role("table")

    def column_header(self, name: str) -> Locator:
        return self.table().get_by_role("columnheader", name=name)

    def export(self) -> Locator:
        return self.dialog.get_by_role("button", name="Export")

    def copy_ref(self) -> Locator:
        return self.dialog.get_by_role("button", name=re.compile(r"^Copy "))

    def close(self) -> Locator:
        return self.dialog.get_by_role("button", name="Close")

    def metric_selector(self) -> Locator:
        """Present only when the caller opened more than one metric at once."""
        return self.dialog.get_by_role("combobox", name="Metric")

    def empty_state(self) -> Locator:
        return self.dialog.get_by_text("No supporting data for this selection")


class GroupDialog:
    def __init__(self, page: Page, title: str) -> None:
        self.page = page
        self.title = title
        self.dialog = page.get_by_role("dialog", name=title)

    def close(self) -> Locator:
        return self.dialog.get_by_role("button", name="Close")

    def card_actions(self, label: str) -> Locator:
        """The overflow menu of the first widget about this metric.

        A metric can carry more than one such menu in a group dialog — its
        summary card and its peer-story row each own one — so this resolves to
        the first in DOM order, which is the summary card's: the drilldown body
        renders the card grid above the peer story.
        """
        return self.dialog.get_by_role("button", name=f"More actions for {label}").first

    def any_card_actions(self) -> Locator:
        return self.dialog.get_by_role("button", name=re.compile(r"^More actions for ")).first

    def card_label(self, actions: Locator) -> str:
        """The metric a card is about, read from the overflow button it owns.

        The label belongs to the server, so a test that needs it to name the
        evidence dialog reads it from the view rather than restating it.
        """
        name = actions.get_attribute("aria-label") or ""
        return name.removeprefix("More actions for ")

    def supporting_data_item(self) -> Locator:
        return self.page.get_by_role("menuitem", name=SUPPORTING_DATA)

    def evidence_for(self, metric: str | re.Pattern[str]) -> MetricEvidenceDialog:
        """The evidence dialog this group's widgets open, by the name it takes."""
        return MetricEvidenceDialog(self.page, metric)

    def open_card_evidence(self, actions: Locator) -> MetricEvidenceDialog:
        label = self.card_label(actions)
        actions.click()
        self.supporting_data_item().click()
        return MetricEvidenceDialog(self.page, label)

    def evidence_button(self) -> Locator:
        return self.dialog.get_by_role("button", name=SUPPORTING_DATA).first

    def timeseries_block(self) -> Locator:
        """The card that owns the first timeseries block's header controls."""
        return self.evidence_button().locator('xpath=ancestor::*[@data-slot="card"][1]')

    def block_metric_heading(self) -> Locator:
        """The block's own heading, rendered when it carries a single metric."""
        return self.timeseries_block().get_by_role("heading")

    def table_view(self) -> Locator:
        return self.timeseries_block().get_by_role("button", name="Table view")

    def chart_view(self) -> Locator:
        return self.timeseries_block().get_by_role("button", name="Chart view")

    def block_table(self) -> Locator:
        return self.timeseries_block().get_by_role("table")

    def open_bucket_evidence(self, metric: str) -> MetricEvidenceDialog:
        """Evidence behind the table's first body row — one time bucket."""
        body = self.block_table().get_by_role("rowgroup").nth(1)
        body.get_by_role("row").first.get_by_role("button").first.click()
        return MetricEvidenceDialog(self.page, metric)

    def open_total_row_evidence(self, metric: str) -> MetricEvidenceDialog:
        """Evidence behind the table's Total row — the whole period, not a bucket.

        The footer rather than a body row on purpose: a total passes no bucket to
        the selection, so it is the one cell whose period is the block's own.
        """
        footer = self.block_table().get_by_role("rowgroup").last
        footer.get_by_role("row").first.get_by_role("button").first.click()
        return MetricEvidenceDialog(self.page, metric)
