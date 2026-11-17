from __future__ import annotations

from collections.abc import Callable
from typing import Final

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    ApiClient,
    ApiResponse,
    PersonaSession,
    scratch_name,
    v3_path,
)

from ..schemas import PROBLEM_CONTENT_TYPE, ProblemDocument
from ..schemas.v3_untyped import Dataset, MetricEntry, Page

pytestmark = pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "dev_lead")

ADMIN_ONLY: Final[str] = "admin role required for this operation"


def _ask(client: ApiClient, message: str) -> ApiResponse:
    return client.post(v3_path("/v1/chat"), json_body={"message": message})


def _ask_for_a_metric(client: ApiClient, dataset: Dataset, name: str) -> ApiResponse:
    return _ask(
        client,
        f"Create a metric named {name} that counts the records of the dataset {dataset.name}.",
    )


def _metrics_named(client: ApiClient, name: str) -> Page[MetricEntry]:
    response = client.get(v3_path("/v1/metrics"), params={"q": name})
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    return response.parse(Page[MetricEntry])


@pytest.mark.reliability
def test_the_assistant_without_a_model_key_refuses_and_writes_no_metric(
    v3_admin: ApiClient, scratch_v3_dataset: Dataset
) -> None:
    name = scratch_name("chat-metric")

    response = _ask_for_a_metric(v3_admin, scratch_v3_dataset, name)

    assert response.status_code >= 400, f"status={response.status_code} {response.text[:300]}"
    assert response.content_type == PROBLEM_CONTENT_TYPE
    assert response.parse(ProblemDocument).status == response.status_code

    written = _metrics_named(v3_admin, name)
    assert (written.total, written.items) == (0, [])


@pytest.mark.reliability
@pytest.mark.xfail(
    strict=True,
    reason=(
        "product bug: a keyless POST /v1/chat answers 500 'An internal error occurred. "
        "Please retry later.' — CanonicalError::internal drops the 'assistant is not "
        "configured on this instance' detail that api/chat.rs and .env.compose.example promise"
    ),
)
def test_the_assistant_without_a_model_key_says_it_is_not_configured(
    v3_admin: ApiClient, scratch_v3_dataset: Dataset
) -> None:
    response = _ask(v3_admin, f"How many records does the dataset {scratch_v3_dataset.name} hold?")

    assert "not configured" in response.parse(ProblemDocument).detail


@pytest.mark.security
def test_dev_lead_is_refused_the_assistant_and_nothing_is_written(
    v3_admin: ApiClient, scratch_v3_dataset: Dataset, session_for: Callable[[str], PersonaSession]
) -> None:
    name = scratch_name("chat-metric")

    response = _ask_for_a_metric(session_for("dev_lead").client, scratch_v3_dataset, name)

    assert response.status_code == 403, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).context.get("reason") == ADMIN_ONLY
    assert _metrics_named(v3_admin, name).total == 0
