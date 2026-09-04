"""Wiki comments received per person, served over one narrow window.

Bronze: Outline wiki pages (one anchor page per person, dated outside every window)
and the comments left on them, all dated 2026-08-05. Silver joins comments to pages,
so the per-person value is the number of comments received in the window: alice 1
through erin 5, the department spreading {1,2,3,4,5}. A re-synced duplicate comment is
not doubled; an empty window serves null.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one, some
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "wiki_comments"

ERIN = "erin@example.com"


def test_wiki_comments(spec: SpecRun) -> None:
    """Aug 02 .. Aug 08 holds every comment; erin received 5 and the department
    spreads {1,2,3,4,5}."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-08-02", "to": "2026-08-08"},
                "metrics": [
                    {
                        "metric_key": "wiki.comments",
                        "views": [{"view": "period"}, {"view": "peer"}, {"view": "timeseries", "bucket": "day"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("wiki.comments", "period", entity_id=ERIN).equals(value=5)
    r.row("wiki.comments", "peer", entity_id=ERIN).equals(target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5)
    points = one(r.series("wiki.comments"), entity_id=ERIN)["points"]
    assert some(points, value=5.0)


def test_wiki_comments_empty_window(spec: SpecRun) -> None:
    """A window with no comments serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "wiki.comments", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("wiki.comments", "period", entity_id=ERIN).equals(value=None)
