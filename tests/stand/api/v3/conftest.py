"""v3 authoring is admin-only by the identity `admin` row, not the realm role.
Teardown runs in dependency order: v3 answers 409 to removing a definition another still names.
"""

from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path

import pytest
from insight_stand import ApiClient, PersonaSession, v3_path

from .. import scratch
from ..schemas.v3 import DestinationsResponse, RuleResponse
from ..schemas.v3_untyped import (
    DashboardDefinition,
    Dataset,
    Folder,
    MetricDefinition,
    WidgetDefinition,
)

_HERE = Path(__file__).resolve().parent


@pytest.hookimpl(tryfirst=True)
def pytest_collection_modifyitems(items: list[pytest.Item]) -> None:
    for item in items:
        if _HERE in item.path.parents:
            item.add_marker(pytest.mark.requires_insight_v3_core)


@pytest.fixture
def v3_admin(admin_operator_session: PersonaSession) -> ApiClient:
    return admin_operator_session.client


@pytest.fixture
def scratch_v3_dataset(v3_admin: ApiClient) -> Iterator[Dataset]:
    dataset = scratch.create_v3_dataset(v3_admin, "dataset")
    yield dataset
    v3_admin.delete(v3_path(f"/v1/datasets/{dataset.name}"))


@pytest.fixture
def scratch_v3_metric(
    v3_admin: ApiClient, scratch_v3_dataset: Dataset
) -> Iterator[MetricDefinition]:
    metric = scratch.create_v3_metric(v3_admin, "metric", scratch_v3_dataset.name)
    yield metric
    v3_admin.delete(v3_path(f"/v1/metrics/{metric.id}"))


@pytest.fixture
def scratch_v3_widget(
    v3_admin: ApiClient, scratch_v3_metric: MetricDefinition
) -> Iterator[WidgetDefinition]:
    widget = scratch.create_v3_widget(v3_admin, "widget", scratch_v3_metric.id)
    yield widget
    v3_admin.delete(v3_path(f"/v1/widgets/{widget.id}"))


@pytest.fixture
def scratch_v3_dashboard(
    v3_admin: ApiClient, scratch_v3_widget: WidgetDefinition
) -> Iterator[DashboardDefinition]:
    dashboard = scratch.create_v3_dashboard(v3_admin, "dashboard", [scratch_v3_widget.id])
    yield dashboard
    v3_admin.delete(v3_path(f"/v1/dashboards/{dashboard.id}"))


@pytest.fixture
def scratch_v3_folder(v3_admin: ApiClient) -> Iterator[Folder]:
    folder = scratch.create_v3_folder(v3_admin, "folder")
    yield folder
    v3_admin.delete(v3_path(f"/v1/folders/{folder.id}"))


@pytest.fixture
def scratch_v3_alert(
    v3_admin: ApiClient, scratch_v3_metric: MetricDefinition
) -> Iterator[RuleResponse]:
    response = v3_admin.get(v3_path("/v1/alert-destinations"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    destinations = response.parse(DestinationsResponse).destinations
    if not destinations:
        pytest.skip(
            "no alert destination is configured on this stand, and a rule must name one; "
            "compose ships none"
        )

    alert = scratch.create_v3_alert(v3_admin, "alert", scratch_v3_metric.id, destinations[0].name)
    yield alert
    v3_admin.delete(v3_path(f"/v1/alerts/{alert.id}"))
