"""Wiki edit sessions collapse rapid saves and count distinct pages edited.

Bronze: Outline page versions, every persona saving one page on May 06. Alice's two
saves ten minutes apart collapse into one session; bob, carol, dave and erin save
hourly, so sessions run 1..5 while every persona edited exactly one page. Gold serves
both counts per person over the window; the peer view is the department distribution.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "wiki_edits"

ERIN = "erin@example.com"


def test_wiki_edit_sessions_and_pages_edited(spec: SpecRun) -> None:
    """May 01 .. May 10 takes every May 06 save: erin's five hourly saves are five
    sessions on one page, and the department spreads {1,2,3,4,5}."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-05-01", "to": "2026-05-10"},
                "metrics": [
                    {
                        "metric_key": "wiki.edits",
                        "views": [{"view": "period"}, {"view": "peer"}, {"view": "timeseries", "bucket": "day"}],
                    },
                    {
                        "metric_key": "wiki.pages_edited",
                        "views": [{"view": "period"}, {"view": "peer"}, {"view": "timeseries", "bucket": "day"}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("wiki.edits", "period", entity_id=ERIN).equals(value=5)
    r.row("wiki.edits", "peer", entity_id=ERIN).equals(target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5)
    edits = one(r.series("wiki.edits"), entity_id=ERIN)["points"]
    assert float(one(edits, bucket_start="2026-05-06")["value"]) == 5.0

    r.row("wiki.pages_edited", "period", entity_id=ERIN).equals(value=1)
    r.row("wiki.pages_edited", "peer", entity_id=ERIN).equals(target_value=1, p25=1, median=1, p75=1, min=1, max=1, n=5)
    pages = one(r.series("wiki.pages_edited"), entity_id=ERIN)["points"]
    assert float(one(pages, bucket_start="2026-05-06")["value"]) == 1.0


def test_unified_wiki_edits_empty_window(spec: SpecRun) -> None:
    """A window with no saves serves an honest null for both counts, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [
                    {"metric_key": "wiki.edits", "views": [{"view": "period"}]},
                    {"metric_key": "wiki.pages_edited", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("wiki.edits", "period", entity_id=ERIN).equals(value=None)
    r.row("wiki.pages_edited", "period", entity_id=ERIN).equals(value=None)
