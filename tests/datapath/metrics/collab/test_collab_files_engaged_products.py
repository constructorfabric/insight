"""OneDrive and SharePoint add up, and the SharePoint feeder applies the shared rules.

Bronze: the daily OneDrive and SharePoint activity reports. Staging drops a row with
an empty product list (an unlicensed account) and a report day that is a copy of an
earlier report. Gold sums files viewed or edited across both products.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_files_engaged_products"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"


@pytest.mark.parametrize(
    ("person", "expected"),
    [
        pytest.param(ALICE, 15, id="onedrive-and-sharepoint-add-up"),
        pytest.param(BOB, 4, id="unlicensed-sharepoint-row-is-dropped"),
        pytest.param(CAROL, 6, id="carried-forward-sharepoint-day-is-dropped"),
    ],
)
def test_files_engaged_sums_both_products_under_the_shared_rules(
    spec: SpecRun, person: str, expected: int
) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [person]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [{"metric_key": "collab.files_engaged", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200, f"should answer 200 for {person}"
    r.row("collab.files_engaged", "period", entity_id=person).equals(value=expected)
