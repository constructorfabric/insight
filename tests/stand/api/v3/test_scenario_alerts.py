from __future__ import annotations

import os
from collections.abc import Callable
from pathlib import Path
from typing import Final

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    ApiClient,
    ApiResponse,
    JsonValue,
    PersonaSession,
    scratch_name,
    v3_path,
)
from insight_stand.stand import candidate_env_files, parse_env_file

from .. import scratch
from ..schemas import ProblemDocument
from ..schemas.v3 import (
    AlertsPage,
    DestinationsResponse,
    NotificationsPage,
    PreviewResponse,
    RuleResponse,
    SummaryResponse,
    WitnessResponse,
)
from ..schemas.v3_untyped import Dataset, MetricDefinition, RunResult

pytestmark = pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "dev_lead")

ADMIN_ONLY: Final[str] = "admin role required for this operation"

INGEST_TOKEN_KEY: Final[str] = "INSIGHT_V3_INGEST_TOKEN"
COMPOSE_INGEST_TOKEN: Final[str] = "insight-v3-core-local-dev-ingest-token"
_REPO_ROOT: Final[Path] = Path(__file__).resolve().parents[4]

RECORDS: Final[tuple[dict[str, JsonValue], ...]] = (
    {"day": "2026-01-01T00:00:00Z", "lines": 4},
    {"day": "2026-01-02T00:00:00Z", "lines": 9},
    {"day": "2026-01-03T00:00:00Z", "lines": 2},
)


def _ingest_token() -> str:
    explicit = os.environ.get(INGEST_TOKEN_KEY, "").strip()
    if explicit:
        return explicit

    for candidate in candidate_env_files(_REPO_ROOT, os.environ):
        if candidate.is_file():
            configured = parse_env_file(candidate).get(INGEST_TOKEN_KEY, "").strip()
            return configured or COMPOSE_INGEST_TOKEN

    return COMPOSE_INGEST_TOKEN


def _feed(client: ApiClient, dataset: Dataset, record: dict[str, JsonValue]) -> None:
    response = client.post(
        v3_path("/v1/raw-data"),
        json_body={"dataset": dataset.name, "raw_data": record},
        headers={"X-Insight-Token": _ingest_token()},
    )
    assert response.status_code == 204, f"status={response.status_code} {response.text[:300]}"


def _total_above(threshold: int, column: str = scratch.SCRATCH_METRIC_COLUMN) -> JsonValue:
    return {"rows": "one", "where": {"column": column, "op": ">", "value": threshold}}


def _preview(client: ApiClient, metric: MetricDefinition, threshold: int) -> PreviewResponse:
    response = client.post(
        v3_path("/v1/alerts/preview"),
        json_body={"metric": metric.id, "condition": _total_above(threshold)},
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(PreviewResponse)


def _run(client: ApiClient, metric: MetricDefinition) -> RunResult:
    response = client.post(v3_path(f"/v1/metrics/{metric.id}/run"), json_body={})
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(RunResult)


def _destinations(client: ApiClient) -> list[str]:
    response = client.get(v3_path("/v1/alert-destinations"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return [destination.name for destination in response.parse(DestinationsResponse).destinations]


def _alerts_named(client: ApiClient, name: str) -> AlertsPage:
    response = client.get(v3_path("/v1/alerts"), params={"q": name})
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(AlertsPage)


def _violations(response: ApiResponse) -> dict[str, str]:
    violations = response.parse(ProblemDocument).context.get("field_violations")
    assert isinstance(violations, list), f"no field_violations: {response.text[:300]}"

    return {
        str(violation["field"]): str(violation["description"])
        for violation in violations
        if isinstance(violation, dict)
    }


def _rule(response: ApiResponse) -> RuleResponse:
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(RuleResponse)


def _draft(name: str, metric: str, destination: str) -> dict[str, JsonValue]:
    return {
        "name": name,
        "metric": metric,
        "destination": destination,
        "interval_secs": 3600,
        "enabled": False,
        "condition": _total_above(0),
    }


@pytest.mark.reliability
def test_a_preview_judges_the_records_fed_to_the_metric_it_watches(
    v3_admin: ApiClient, scratch_v3_dataset: Dataset, scratch_v3_metric: MetricDefinition
) -> None:
    empty = _preview(v3_admin, scratch_v3_metric, 0)

    assert empty.outcome == "no_breach"
    assert empty.witness is not None
    assert (empty.witness.answered, empty.witness.matched) == (1, 0)

    for record in RECORDS:
        _feed(v3_admin, scratch_v3_dataset, record)

    fed = len(RECORDS)
    met = _preview(v3_admin, scratch_v3_metric, fed - 1)
    unmet = _preview(v3_admin, scratch_v3_metric, fed)

    assert met.outcome == "breach"
    assert met.reason is None
    assert met.witness == WitnessResponse(
        answered=1, matched=1, columns=[scratch.SCRATCH_METRIC_COLUMN], rows=[[fed]]
    )
    assert unmet.outcome == "no_breach"
    assert unmet.witness is not None
    assert (unmet.witness.answered, unmet.witness.matched) == (1, 0)
    assert _run(v3_admin, scratch_v3_metric).rows == [[fed]]


@pytest.mark.reliability
def test_a_rule_the_validator_refuses_is_never_stored(
    v3_admin: ApiClient, scratch_v3_metric: MetricDefinition
) -> None:
    unconfigured = scratch_name("destination")
    assert unconfigured not in _destinations(v3_admin)

    name = scratch_name("alert")
    draft = _draft(name, scratch_v3_metric.id, unconfigured)
    refused = {
        "expected_revision": {**draft, "expected_revision": 0},
        "condition.where.column": {**draft, "condition": _total_above(0, "absent")},
        "interval_secs": {**draft, "interval_secs": 59},
        "destination": draft,
    }

    described: dict[str, str] = {}
    for field, body in refused.items():
        response = v3_admin.post(v3_path("/v1/alerts"), json_body=body)
        assert response.status_code == 400, f"{field}: {response.status_code} {response.text[:300]}"

        violations = _violations(response)
        assert list(violations) == [field], response.text[:300]
        described.update(violations)

    assert unconfigured in described["destination"]

    stored = _alerts_named(v3_admin, name)
    assert (stored.total, stored.alerts) == (0, [])


@pytest.mark.security
def test_dev_lead_is_refused_every_alert_operation_before_any_lookup(
    v3_admin: ApiClient,
    scratch_v3_metric: MetricDefinition,
    session_for: Callable[[str], PersonaSession],
) -> None:
    lead = session_for("dev_lead").client
    name = scratch_name("alert")
    draft = _draft(name, scratch_v3_metric.id, scratch_name("destination"))
    revision: JsonValue = {"expected_revision": 0}
    unknown = f"/v1/alerts/{scratch.UNKNOWN_ID}"

    calls: tuple[tuple[str, str, JsonValue], ...] = (
        ("GET", "/v1/alert-destinations", None),
        (
            "POST",
            "/v1/alerts/preview",
            {"metric": scratch_v3_metric.id, "condition": _total_above(0)},
        ),
        ("POST", "/v1/alerts", draft),
        ("GET", "/v1/alerts", None),
        ("GET", unknown, None),
        ("PUT", unknown, {**draft, "expected_revision": 0}),
        ("POST", f"{unknown}/disable", revision),
        ("POST", f"{unknown}/enable", revision),
        ("GET", f"{unknown}/notifications", None),
        ("DELETE", unknown, None),
    )

    for method, suffix, body in calls:
        response = lead.request(method, v3_path(suffix), json_body=body)
        assert response.status_code == 403, (
            f"{method} {suffix}: {response.status_code} {response.text[:300]}"
        )
        assert response.parse(ProblemDocument).context.get("reason") == ADMIN_ONLY

    assert v3_admin.get(v3_path(unknown)).status_code == 404
    assert _alerts_named(v3_admin, name).total == 0


@pytest.mark.reliability
def test_an_alert_rule_is_replaced_toggled_and_deleted_through_its_revisions(
    v3_admin: ApiClient, scratch_v3_alert: RuleResponse
) -> None:
    created = scratch_v3_alert
    path = v3_path(f"/v1/alerts/{created.id}")

    assert created.destination in _destinations(v3_admin)
    assert _alerts_named(v3_admin, created.name).alerts == [
        SummaryResponse(enabled=False, id=created.id, metric=created.metric, name=created.name)
    ]
    assert _rule(v3_admin.get(path)) == created

    replacement = {
        **_draft(created.name, created.metric, created.destination),
        "interval_secs": 7200,
        "expected_revision": created.revision,
    }
    replaced = _rule(v3_admin.put(path, json_body=replacement))

    assert replaced.revision == created.revision + 1
    assert (replaced.interval_secs, replaced.enabled) == (7200, False)
    assert replaced.condition.model_dump(mode="json", by_alias=True) == _total_above(0)

    stale = v3_admin.put(path, json_body={**replacement, "interval_secs": 60})

    assert stale.status_code == 409, f"status={stale.status_code} {stale.text[:300]}"
    assert stale.parse(ProblemDocument).status == 409
    assert _rule(v3_admin.get(path)) == replaced

    enabled = _rule(
        v3_admin.post(f"{path}/enable", json_body={"expected_revision": replaced.revision})
    )

    assert (enabled.enabled, enabled.revision) == (True, replaced.revision + 1)
    assert _rule(v3_admin.get(path)).enabled is True

    disabled = _rule(
        v3_admin.post(f"{path}/disable", json_body={"expected_revision": enabled.revision})
    )

    assert (disabled.enabled, disabled.revision) == (False, enabled.revision + 1)
    assert [alert.enabled for alert in _alerts_named(v3_admin, created.name).alerts] == [False]

    notifications = v3_admin.get(f"{path}/notifications")
    assert notifications.status_code == 200, notifications.text[:300]
    assert notifications.parse(NotificationsPage).notifications == []

    deleted = v3_admin.delete(path)
    assert deleted.status_code == 204, f"status={deleted.status_code} {deleted.text[:300]}"

    assert v3_admin.get(path).status_code == 404
    assert v3_admin.get(f"{path}/notifications").status_code == 404
    assert _alerts_named(v3_admin, created.name).total == 0
