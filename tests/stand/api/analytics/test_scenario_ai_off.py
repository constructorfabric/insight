from __future__ import annotations

from collections.abc import Callable
from uuid import uuid4

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, analytics_path
from insight_stand.api import JsonValue
from pydantic import BaseModel

from .. import scratch
from ..schemas import ProblemDocument
from ..schemas.analytics import (
    AiConfigResponse,
    CreateContextRequest,
    ExplainRequest,
    PutCredentialRequest,
    PutSettingsRequest,
    Scope,
    UpdateContextRequest,
)
from . import query_window

pytestmark = pytest.mark.security

CONFIG = analytics_path("/v1/ai/config")
CREDENTIALS = analytics_path("/v1/ai/credentials")
SETTINGS = analytics_path("/v1/ai/settings")
CONTEXT = analytics_path("/v1/ai/context")
EXPLAIN = analytics_path("/v1/ai/explain")

TAG = f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-ai"


def _body(model: BaseModel) -> JsonValue:
    return model.model_dump(mode="json", exclude_none=True)


def _config(client: ApiClient) -> AiConfigResponse:
    response = client.get(CONFIG)
    assert response.status_code == 200, response.text[:300]
    return response.parse(AiConfigResponse)


def _setup_steps(manifest: Manifest) -> list[tuple[str, str, JsonValue]]:
    entry = analytics_path(f"/v1/ai/context/{uuid4()}")
    since, until = query_window(manifest, max_days=30)
    explain = ExplainRequest.model_validate(
        {
            "metric_key": "tasks.closed",
            "label": "Tasks closed",
            "value": "12",
            "period": "month",
            "since": since,
            "until": until,
        }
    )
    return [
        ("PUT", CREDENTIALS, _body(PutCredentialRequest(token=f"sk-ant-{TAG}"))),
        ("GET", CREDENTIALS, None),
        ("PUT", SETTINGS, _body(PutSettingsRequest(system_prompt=TAG))),
        ("GET", SETTINGS, None),
        ("POST", CONTEXT, _body(CreateContextRequest(scope=Scope.tenant, title=TAG, body=TAG))),
        ("GET", CONTEXT, None),
        ("PATCH", entry, _body(UpdateContextRequest(title=TAG))),
        ("POST", EXPLAIN, _body(explain)),
        ("DELETE", entry, None),
        ("DELETE", SETTINGS, None),
        ("DELETE", CREDENTIALS, None),
    ]


@pytest.mark.requires_seed("admin_operator", "dev_lead")
def test_an_operator_setting_up_ai_finds_no_surface_and_changes_nothing(
    admin_operator_session: PersonaSession,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    admin = admin_operator_session.client
    before = _config(admin)
    if before.enabled:
        pytest.skip(
            "this stand offers AI explanations, so the switched-off contract does not apply"
        )

    answers = {
        f"{method} {url}": admin.request(method, url, json_body=body)
        for method, url, body in _setup_steps(stand_manifest)
    }

    assert len(answers) == 11

    refused = {step: response.status_code for step, response in answers.items()}
    assert set(refused.values()) == {404}, refused
    for step, response in answers.items():
        assert response.parse(ProblemDocument).status == 404, step

    assert _config(admin) == before
    assert _config(session_for("dev_lead").client) == before
