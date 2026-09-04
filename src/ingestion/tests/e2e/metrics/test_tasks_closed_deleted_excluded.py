"""Tasks closed under the deletion and visibility mechanism: an issue deleted at the
source leaves the metric, an issue whose whole project merely became invisible stays.

Bronze: Jira issues and their close events, plus the absence censuses -- the issue
census (ids observed alive) and the project visibility roster. An issue in bronze but
absent from the census is deleted when its project is still in the roster and
access_lost when the project is gone; silver records the classification as availability
events in the field history, and gold excludes only deleted/trashed issues.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_closed_deleted_excluded"

ERIN = "erin@example.com"
CAROL = "carol@example.com"


def test_deleted_issue_leaves_tasks_closed_access_lost_issue_stays(spec: SpecRun) -> None:
    """Erin's deleted DELX-1 is out (5, not 6), carol's access-lost LSTC-1 is in (3+1=4);
    the department distribution {1,2,4,4,5} follows the same rule."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN, CAROL]},
                "period": {"from": "2026-12-20", "to": "2026-12-31"},
                "metrics": [{"metric_key": "tasks.closed", "views": [{"view": "period"}, {"view": "peer"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.closed", "period", entity_id=ERIN).equals(value=5)
    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=4)
    r.row("tasks.closed", "peer", entity_id=ERIN).equals(target_value=5, p25=2, median=4, p75=4, min=1, max=5, n=5)


def test_tasks_closed_empty_window(spec: SpecRun) -> None:
    """A window with no close events serves null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "tasks.closed", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.closed", "period", entity_id=ERIN).equals(value=None)
