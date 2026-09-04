"""Files engaged (viewed or edited) per person over a window, from M365 file activity.

Bronze: daily OneDrive and SharePoint file-activity reports per person. Silver: each
product deduped to per-person/day file counts. Gold serves the requested member's
per-person sum of files viewed-or-edited over the window across both products; the
peer view is the distribution of those sums across the member's department. A
duplicate bronze row dedupes rather than doubling the count.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_files_engaged"

ERIN = "erin@example.com"


def test_collab_files_engaged(spec: SpecRun) -> None:
    """Five department members with base rates {10,20,30,40,50} on one day; erin (50)
    is the requested member, so value is 50 and the peer view spreads the department."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "collab.files_engaged",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("collab.files_engaged", "period", entity_id=ERIN).equals(value=50)
    r.row("collab.files_engaged", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    points = one(r.series("collab.files_engaged"), entity_id=ERIN)["points"]
    assert float(one(points, bucket_start="2026-12-25")["value"]) == 50.0
    by_tool = one(
        r.breakdown("collab.files_engaged"),
        entity_id=ERIN,
        dimensions={"key": "tool", "value": "m365"},
    )
    assert float(by_tool["value"]) == 50.0


def test_collab_files_engaged_empty_window(spec: SpecRun) -> None:
    """A window with no file activity serves null, not zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [{"metric_key": "collab.files_engaged", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("collab.files_engaged", "period", entity_id=ERIN).equals(value=None)
