"""Unlicensed M365 users never leak into collaboration metrics.

The M365 teams_activity report includes unlicensed users (guests, ex-employees,
service accounts) flagged isLicensed=false. The silver feeder keeps only rows with an
explicit isLicensed=true, so an unlicensed or unknown-license user never reaches
gold. alice (licensed, 3 meetings) is the control proving the pipeline is healthy;
frank (isLicensed=false) and grace (isLicensed=null) must serve null.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_meeting_unlicensed_excluded"

ALICE = "alice@example.com"
FRANK = "frank@example.com"
GRACE = "grace@example.com"


def test_unlicensed_and_unknown_license_users_excluded_licensed_control(spec: SpecRun) -> None:
    """alice's licensed row flows through as 3; frank's unlicensed and grace's
    unknown-license rows are dropped at silver and serve null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, FRANK, GRACE]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [{"metric_key": "collab.meetings_count", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    r.row("collab.meetings_count", "period", entity_id=ALICE).equals(value=3)
    r.row("collab.meetings_count", "period", entity_id=FRANK).equals(value=None)
    r.row("collab.meetings_count", "period", entity_id=GRACE).equals(value=None)
