"""Shares inside and outside the organization add up across OneDrive and SharePoint.

Bronze: the daily OneDrive and SharePoint activity reports with files shared
internally and externally. Gold serves files_shared as the sum of both scopes and
both products with a scope dimension, and the internal and external metrics as its
parts.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_files_shared_scopes"

ALICE = "alice@example.com"


def test_files_shared_adds_both_scopes_and_the_breakdown_splits_them(spec: SpecRun) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "collab.files_shared",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["scope"]},
                        ],
                    },
                    {"metric_key": "collab.files_shared_internal", "views": [{"view": "period"}]},
                    {"metric_key": "collab.files_shared_external", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("collab.files_shared", "period", entity_id=ALICE).equals(value=12)
    by_scope = r.breakdown("collab.files_shared")
    for scope, expected in (("internal", 5), ("external", 7)):
        row = one(by_scope, entity_id=ALICE, dimensions={"key": "scope", "value": scope})
        assert float(row["value"]) == expected, f"should split {scope} as {expected}: {row}"
    r.row("collab.files_shared_internal", "period", entity_id=ALICE).equals(value=5)
    r.row("collab.files_shared_external", "period", entity_id=ALICE).equals(value=7)
