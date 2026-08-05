"""A lead's team view — `/ic/$person_id/team`. Locators and navigation only.

`$person_id` is the canonical person UUID since the identity cutover (#2098),
the same key `PersonView` uses. Keying it on the email sent the SPA to a
route it could not resolve, and it rendered the PERSONAL view instead — a
redirect, not an error, which is why the failure read as a missing table
rather than as a bad URL.
"""

from __future__ import annotations

import re
from urllib.parse import quote

from playwright.sync_api import Locator, Page


class TeamView:
    def __init__(self, page: Page) -> None:
        self.page = page

    @staticmethod
    def path(person_id: str) -> str:
        return f"/ic/{quote(person_id, safe='')}/team"

    def team_heading(self, display_name: str) -> Locator:
        """The heading naming whose team this is.

        The accessible name is composed by the SPA ("Team of <name>"), so it is
        matched as a substring rather than reconstructed here — reconstructing it
        would put the product's copy in the test.
        """
        return self.page.get_by_role("heading").filter(has_text=display_name).first

    def metrics_overview(self) -> Locator:
        return self.page.get_by_text(re.compile(r"^Members . metrics$"))

    def member_row(self, display_name: str) -> Locator:
        """That member's row in the team table.

        A ROW, not a link named after them, and the distinction is the whole
        value of the locator. The sidebar renders every person in the signed-in
        user's org scope on EVERY view, so `get_by_role("link", name=<person>)`
        matches on the team view whether or not the team table rendered at all —
        measured at three matches per name, only one of which is the table.
        An assertion built on that would pass against an empty team view.

        The table row is unique per member (measured: exactly one) and exists
        only if the table rendered, which is what a caller actually means.
        """
        return self.page.get_by_role("row").filter(has_text=display_name)

    def recorded_metric_cell(self, display_name: str, metric_label: str) -> Locator:
        name = re.compile(
            rf"^{re.escape(display_name)} — {re.escape(metric_label)}: (?!not recorded)"
        )
        return self.page.get_by_role("button", name=name)

    def unrecorded_metric_cell(self, display_name: str, metric_label: str) -> Locator:
        return self.page.get_by_role(
            "button",
            name=f"{display_name} — {metric_label}: not recorded",
            exact=True,
        )

    def metric_cell(self, display_name: str, metric_label: str) -> Locator:
        """That member's cell for a metric, recorded or an honest "not recorded".

        Presence proves the row rendered the column; it does not require a
        value, so a legitimately unrecorded metric (a member who closed tasks
        but fixed no bugs) is not a failure.
        """
        name = re.compile(rf"^{re.escape(display_name)} — {re.escape(metric_label)}: ")
        return self.page.get_by_role("button", name=name)

    def any_recorded_metric_cell(self, display_name: str) -> Locator:
        """Any metric this member has a recorded value for — the row is not blank."""
        name = re.compile(rf"^{re.escape(display_name)} — .+: (?!not recorded)")
        return self.page.get_by_role("button", name=name).first

    def domain_card(self, label: str) -> Locator:
        return self.page.get_by_role("button", name=f"Open {label} details")
