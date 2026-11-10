"""The shared v3 scratch fixtures create what they claim, and read back.

    PUT  /v1/datasets/{name}   200
    POST /v1/metrics           201 · GET /v1/metrics/{id}     200
    POST /v1/widgets           201 · GET /v1/widgets/{id}     200
    POST /v1/dashboards        201 · GET /v1/dashboards/{id}  200
    POST /v1/folders           201
    POST /v1/alerts            201

Every scenario module under this directory builds on these fixtures, so a
fixture that quietly made the wrong thing would turn each of them into a test of
something else.
"""

from __future__ import annotations

import pytest
from insight_stand import ADMIN_OPERATOR_FIXTURE, SCRATCH_PREFIX

from .. import scratch
from ..schemas.v3 import RuleResponse
from ..schemas.v3_untyped import (
    DashboardDefinition,
    Dataset,
    Folder,
    MetricDefinition,
    WidgetDefinition,
)

pytestmark = [pytest.mark.reliability, pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE)]


def test_the_definition_chain_names_each_link(
    scratch_v3_dataset: Dataset,
    scratch_v3_metric: MetricDefinition,
    scratch_v3_widget: WidgetDefinition,
    scratch_v3_dashboard: DashboardDefinition,
) -> None:
    assert scratch_v3_dataset.name.startswith(SCRATCH_PREFIX)
    assert scratch_v3_metric.body["dataset"] == scratch_v3_dataset.name
    assert scratch_v3_widget.body["metric"] == scratch_v3_metric.id
    assert scratch_v3_widget.body["columns"] == [scratch.SCRATCH_METRIC_COLUMN]
    assert scratch_v3_dashboard.body["items"] == [{"widget": scratch_v3_widget.id}]
    assert scratch_v3_dashboard.access == "owner"


def test_the_folder_is_the_callers_own(scratch_v3_folder: Folder) -> None:
    assert scratch_v3_folder.name.startswith(SCRATCH_PREFIX)
    assert scratch_v3_folder.access == "owner"


def test_the_alert_watches_the_metric_and_never_notifies(
    scratch_v3_alert: RuleResponse, scratch_v3_metric: MetricDefinition
) -> None:
    assert scratch_v3_alert.metric == scratch_v3_metric.id
    assert scratch_v3_alert.enabled is False
