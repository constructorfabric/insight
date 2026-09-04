"""Files shared externally from OneDrive and SharePoint, served per person over a window.

Bronze: daily OneDrive and SharePoint file-activity reports per person. Silver: each
product deduped to per-person/day share counts. Gold serves the member's sum of files
shared externally over the window across both products; the peer view is the
distribution of those per-person sums across the member's department. A re-synced
duplicate row changes nothing.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_files_shared_external"

ERIN = "erin@example.com"


def test_collab_files_shared_external(spec: SpecRun) -> None:
    """Dec 01 .. Dec 31 takes each member's one Dec 25 row; the department spreads
    {10,20,30,40,50} and erin's sum is 50."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "collab.files_shared_external",
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

    r.row("collab.files_shared_external", "period", entity_id=ERIN).equals(value=50)
    r.row("collab.files_shared_external", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    shared = one(r.series("collab.files_shared_external"), entity_id=ERIN)["points"]
    assert float(one(shared, bucket_start="2026-12-25")["value"]) == 50.0
    by_tool = one(
        r.breakdown("collab.files_shared_external"), entity_id=ERIN, dimensions={"key": "tool", "value": "m365"}
    )
    assert float(by_tool["value"]) == 50.0


def test_collab_files_shared_external_empty_window(spec: SpecRun) -> None:
    """A window with no rows serves a null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [{"metric_key": "collab.files_shared_external", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("collab.files_shared_external", "period", entity_id=ERIN).equals(value=None)
