"""Wiki pages created, counted per person over a window.

Bronze: Outline wiki pages, one row per page authored, every row dated inside one
narrow February window. Gold serves the count of pages a person authored in the
window; the department of five spreads {1,2,3,4,5} and the requested member erin
authored 5. A re-synced duplicate page changes nothing; a window with no rows serves
null.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "wiki_pages_created"

ERIN = "erin@example.com"


def test_wiki_pages_created(spec: SpecRun) -> None:
    """Feb 01 .. Feb 07 holds every row; erin's 5 pages sit in the Feb 04 bucket."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-02-01", "to": "2026-02-07"},
                "metrics": [
                    {
                        "metric_key": "wiki.pages_created",
                        "views": [{"view": "period"}, {"view": "peer"}, {"view": "timeseries", "bucket": "day"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("wiki.pages_created", "period", entity_id=ERIN).equals(value=5)
    r.row("wiki.pages_created", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    points = one(r.series("wiki.pages_created"), entity_id=ERIN)["points"]
    assert float(one(points, bucket_start="2026-02-04")["value"]) == 5.0


def test_wiki_pages_created_empty_window(spec: SpecRun) -> None:
    """A window with no rows serves null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "wiki.pages_created", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("wiki.pages_created", "period", entity_id=ERIN).equals(value=None)
