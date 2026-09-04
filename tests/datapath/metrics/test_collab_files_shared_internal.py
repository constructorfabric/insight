"""Files shared internally per person over a window, and the unified files-shared
metric it feeds.

Bronze: daily OneDrive and SharePoint file-activity reports per person. Silver: each
product deduped to per-person/day share counts. Gold serves the requested member's
sum of files shared internally over the window across both products; the peer view
is the distribution of those per-person sums across the member's department.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_files_shared_internal"

ERIN = "erin@example.com"


def test_collab_files_shared_internal(spec: SpecRun) -> None:
    """Erin's window sum is 50 and the department of five spreads {10,20,30,40,50};
    the unified metric mirrors it under scope=internal."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "collab.files_shared_internal",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    },
                    {
                        "metric_key": "collab.files_shared",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["scope"]},
                        ],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("collab.files_shared_internal", "period", entity_id=ERIN).equals(value=50)
    r.row("collab.files_shared_internal", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    internal = one(r.series("collab.files_shared_internal"), entity_id=ERIN)["points"]
    assert float(one(internal, bucket_start="2026-12-25")["value"]) == 50.0
    by_tool = one(
        r.breakdown("collab.files_shared_internal"),
        entity_id=ERIN,
        dimensions={"key": "tool", "value": "m365"},
    )
    assert float(by_tool["value"]) == 50.0

    r.row("collab.files_shared", "period", entity_id=ERIN).equals(value=50)
    r.row("collab.files_shared", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    shared = one(r.series("collab.files_shared"), entity_id=ERIN)["points"]
    assert float(one(shared, bucket_start="2026-12-25")["value"]) == 50.0
    by_scope = one(
        r.breakdown("collab.files_shared"),
        entity_id=ERIN,
        dimensions={"key": "scope", "value": "internal"},
    )
    assert float(by_scope["value"]) == 50.0


def test_unified_internal_file_sharing_empty_window(spec: SpecRun) -> None:
    """A window with no rows serves an honest null for both metrics."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {"metric_key": "collab.files_shared_internal", "views": [{"view": "period"}]},
                    {"metric_key": "collab.files_shared", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("collab.files_shared_internal", "period", entity_id=ERIN).equals(value=None)
    r.row("collab.files_shared", "period", entity_id=ERIN).equals(value=None)
