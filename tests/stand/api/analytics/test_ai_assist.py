"""The `/v1/ai/*` path group on analytics — explaining a chart with Claude.

    GET    /v1/ai/config          200 always · the one route a stand answers
                                  with the feature off, because "off" IS the
                                  answer the SPA needs
    GET    /v1/ai/credentials     404 here · whether the caller stored a key
    PUT    /v1/ai/credentials     404 here · store or replace it
    DELETE /v1/ai/credentials     404 here · forget it
    GET    /v1/ai/settings        404 here · the tenant's system prompt
    PUT    /v1/ai/settings        404 here · rewrite it (admin)
    DELETE /v1/ai/settings        404 here · reset it to the shipped default
    GET    /v1/ai/context         404 here · the caller's notes and the org's
    POST   /v1/ai/context         404 here · add one
    PATCH  /v1/ai/context/{id}    404 here · edit one
    DELETE /v1/ai/context/{id}    404 here · remove one
    POST   /v1/ai/explain         404 here · ask for the reading

This stand does not turn the feature on. `ai_assist.enabled` defaults false and
nothing in the compose stand sets it, so what this module proves is the
contract of a deployment that has not opted in: `config` still answers, and
every other route is absent rather than refusing or erroring.

That is the state most stands are in, and it is the one worth pinning: the
switch is what stops an un-provisioned stand from accepting a key it cannot
seal. A stand WITH the switch on is covered by the service's own live HTTP
tests against a real MariaDB (`api/ai/live_tests/`), which can set the config
this suite cannot.

404 rather than 403: a stand that does not offer the feature should be
indistinguishable from one whose build predates it. `config` is the deliberate
exception — a 404 there would read as "too old to know", and the SPA would have
no way to tell that apart from "switched off".

The 401 half is in `test_gateway.py`, swept over every operation at once.
"""

from __future__ import annotations

from uuid import uuid4

import pytest
from insight_stand import ApiClient, PersonaSession, analytics_path
from insight_stand.api import JsonValue

from ..schemas import ProblemDocument
from ..schemas.analytics import AiConfigResponse

#: Refusals and credential hygiene: what the switch withholds, and that a
#: refused write leaves no key behind.
pytestmark = pytest.mark.security

CONFIG = analytics_path("/v1/ai/config")
CREDENTIALS = analytics_path("/v1/ai/credentials")
SETTINGS = analytics_path("/v1/ai/settings")
CONTEXT = analytics_path("/v1/ai/context")
EXPLAIN = analytics_path("/v1/ai/explain")

#: Any well-formed id. Every route below is refused before it is read, so the
#: entry never has to exist — using a real one would prove nothing extra and
#: would need the feature switched on to create.
ABSENT_ENTRY = analytics_path(f"/v1/ai/context/{uuid4()}")

#: The smallest body the explain route would accept if it were reachable.
#: Sent so the 404 is unambiguous — a malformed body would answer 4xx for a
#: reason that has nothing to do with the switch.
SNAPSHOT = {
    "metric_key": "git.prs_merged",
    "label": "PRs merged",
    "value": "12",
    "period": "day",
    "since": "2026-08-01",
    "until": "2026-08-22",
}


def test_config_answers_on_a_stand_that_does_not_offer_explanations(
    api: ApiClient,
) -> None:
    """`config` is the route that must answer whatever the switch says.

    A 404 here would be indistinguishable from a build that predates the
    feature, and the SPA decides whether to render anything at all from this.
    """
    response = api.get(CONFIG)

    assert response.status_code == 200, response.text
    config = AiConfigResponse.model_validate(response.json())
    assert config.enabled is False
    assert config.model, "the model is named even while the feature is off"


@pytest.mark.parametrize(
    ("method", "url", "body"),
    [
        pytest.param("GET", CREDENTIALS, None, id="get-credentials"),
        pytest.param("PUT", CREDENTIALS, {"token": "sk-ant-not-stored"}, id="put-credentials"),
        pytest.param("DELETE", CREDENTIALS, None, id="delete-credentials"),
        pytest.param("GET", SETTINGS, None, id="get-settings"),
        pytest.param("PUT", SETTINGS, {"system_prompt": "never applied"}, id="put-settings"),
        pytest.param("DELETE", SETTINGS, None, id="delete-settings"),
        pytest.param("GET", CONTEXT, None, id="list-context"),
        pytest.param(
            "POST",
            CONTEXT,
            {"scope": "person", "title": "never stored", "body": "never stored"},
            id="create-context",
        ),
        pytest.param("PATCH", ABSENT_ENTRY, {"title": "never applied"}, id="update-context"),
        pytest.param("DELETE", ABSENT_ENTRY, None, id="delete-context"),
        pytest.param("POST", EXPLAIN, SNAPSHOT, id="explain"),
    ],
)
def test_every_other_route_is_absent_until_a_stand_turns_the_feature_on(
    api: ApiClient, method: str, url: str, body: JsonValue
) -> None:
    """The switch removes the surface rather than refusing at it.

    Sent with a session on purpose: an anonymous call proves only that the edge
    demands a token, which `test_gateway.py` already sweeps.
    """
    response = api.request(method, url, json_body=body)

    assert response.status_code == 404, f"{method} {url} answered {response.status_code}"
    problem = ProblemDocument.model_validate(response.json())
    assert problem.status == 404


def test_the_key_route_stores_nothing_while_the_feature_is_off(
    api: ApiClient,
) -> None:
    """A PUT that is refused must not leave a key behind.

    The switch is the guard that stops a stand accepting a credential before an
    operator has given it a sealing key; if the write landed anyway, that
    guarantee would be worth nothing.
    """
    stored = api.put(CREDENTIALS, json_body={"token": "sk-ant-should-never-be-stored"})
    assert stored.status_code == 404, stored.text

    read_back = api.get(CREDENTIALS)
    assert read_back.status_code == 404, read_back.text


def test_an_admin_gets_the_same_absent_surface(
    admin_operator_session: PersonaSession,
) -> None:
    """The switch is not a permission — holding admin does not reveal the routes.

    Worth its own case because the prompt and the organisation's context ARE
    admin-gated once the feature is on, so "admin sees more" is the plausible
    wrong behaviour.
    """
    response = admin_operator_session.client.get(SETTINGS)

    assert response.status_code == 404, response.text
