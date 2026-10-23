"""Every closed-issue evidence row names its issue.

An issue the census observes under a second key is two availability entities, one
of which knows no readable key; the key the issue is served under must still be
the one its other rows carry.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = [pytest.mark.fixture, pytest.mark.shared_dataset]

SPEC = "jira_tasks_closed_evidence_ref"

CAROL = "carol@example.com"


def test_every_closure_in_the_evidence_names_its_issue(spec: SpecRun) -> None:
    """Both closures are listed, each with its key, the issue the census holds twice included."""
    r = spec.call(
        {
            "url": "/v1/metric-drilldown",
            "method": "POST",
            "body": {
                "metric_key": "tasks.closed",
                "entity": {"type": "person", "id": CAROL},
                "period": {"from": "2026-08-01", "to": "2026-08-31"},
            },
        }
    )
    assert r.status == 200, r.payload

    refs = sorted(str(row["values"].get("ref")) for row in r.payload["rows"])
    assert refs == ["RFA-1", "RFB-1"], f"evidence refs: {refs}"
