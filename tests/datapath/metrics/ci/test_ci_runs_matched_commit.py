"""CI runs whose head commit was collected, served at tenant grain.

Bronze: GitHub workflow runs carrying a head_sha, and commits arriving as their own
stream. Silver: both accumulate as classes. Gold counts a decided run when its head
commit exists among the collected commits; PR merge refs and fork commits the stream
never sees stay unmatched by design. Seeded: two decided runs, one whose head_sha
matches a seeded commit and one carrying a synthetic PR merge ref, so 1 of 2 counts.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_runs_matched_commit"


def test_only_the_run_whose_head_commit_was_collected_counts(spec: SpecRun) -> None:
    """Of two decided runs, only the one whose head commit exists among the
    collected commits counts: the period value is 1 and the Mar 01 daily point is 1."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.runs_matched_commit",
                        "views": [{"view": "period"}, {"view": "timeseries", "bucket": "day"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ci.runs_matched_commit", "period", entity_id=spec.tenant).equals(value=1.0)
    points = [p for s in r.series("ci.runs_matched_commit") for p in s["points"]]
    assert some(points, bucket_start="2026-03-01", value=1.0)
