"""Backend-generated reports through the deployed gateway.

    POST /v1/reports/preview   200 · 403 outside the visible set
    POST /v1/reports/export    200 CSV/XLSX · 403 outside the visible set
"""

from __future__ import annotations

import csv
import io

import pytest
from insight_stand import ApiClient, Manifest, analytics_path
from insight_stand.api import JsonValue

from ..schemas import MetricDefinitionListResponse, ProblemDocument, ReportPreviewResponse
from . import query_window

REPORT_PREVIEW = analytics_path("/v1/reports/preview")
REPORT_EXPORT = analytics_path("/v1/reports/export")


def _recipe(api: ApiClient, manifest: Manifest) -> dict[str, JsonValue]:
    response = api.get(analytics_path("/v1/metric-definitions"))
    assert response.status_code == 200, f"definitions: {response.status_code}"
    metrics = response.parse(MetricDefinitionListResponse).metrics
    metric = next(
        (
            metric
            for metric in metrics
            if metric.is_enabled and metric.entity_type.value == "person"
        ),
        None,
    )
    assert metric is not None, "no enabled person metric is available"
    start, end = query_window(manifest)

    return {
        "subject": {
            "type": "people",
            "ids": [manifest.fixture("dev_lead").uuid],
        },
        "period": {"from": start, "to": end},
        "granularity": "month",
        "metric_keys": [metric.metric_key],
    }


@pytest.mark.reliability
def test_report_preview_returns_positional_rows_without_internal_ids(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    response = api.post(REPORT_PREVIEW, json_body=_recipe(api, stand_manifest))

    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    preview = response.parse(ReportPreviewResponse)
    assert preview.total_rows >= 1
    assert preview.rows
    assert all(len(row.root) == len(preview.columns) for row in preview.rows)
    assert "person_id" not in {column.key for column in preview.columns}


@pytest.mark.reliability
def test_report_export_downloads_a_complete_csv(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    request = {**_recipe(api, stand_manifest), "format": "csv"}

    response = api.post(REPORT_EXPORT, json_body=request)

    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    assert response.content_type.startswith("text/csv")
    assert ".csv" in response.headers.get("content-disposition", "")
    rows = list(csv.reader(io.StringIO(response.content.decode("utf-8-sig"))))
    assert len(rows) >= 2


@pytest.mark.reliability
def test_report_export_downloads_a_complete_xlsx(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    request = {**_recipe(api, stand_manifest), "format": "xlsx"}

    response = api.post(REPORT_EXPORT, json_body=request)

    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    assert response.content_type == (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    )
    assert ".xlsx" in response.headers.get("content-disposition", "")
    assert response.content.startswith(b"PK\x03\x04")


@pytest.mark.requires_seed("dev_lead", "sales_ic")
@pytest.mark.security
@pytest.mark.parametrize("endpoint", [REPORT_PREVIEW, REPORT_EXPORT])
def test_reports_refuse_a_person_outside_the_visible_set(
    api: ApiClient, stand_manifest: Manifest, endpoint: str
) -> None:
    request = _recipe(api, stand_manifest)
    request["subject"] = {
        "type": "people",
        "ids": [stand_manifest.fixture("sales_ic").uuid],
    }
    if endpoint == REPORT_EXPORT:
        request["format"] = "csv"

    response = api.post(endpoint, json_body=request)

    assert response.status_code == 403, (
        f"out-of-scope report answered {response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 403
