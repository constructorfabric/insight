"""Pure smoke boundary rules: envelope framing, authorize queries and soft verdicts.

These tests need neither a browser nor an API connection. Browser journeys
consume these same parsers and aggregation rules.
"""

from urllib.parse import parse_qsl, urlsplit

import pytest
from insight_stand import Capabilities, ManifestError

from .catalogue import PAGE_SCENARIOS, PERSONAS, missing_capabilities, scenarios_for
from .checks import Checks
from .envelopes import envelope_has_event
from .login import password_form_url

pytestmark = pytest.mark.reliability


@pytest.mark.parametrize(
    ("body", "expected"),
    [
        (b'{}\n{"type":"event"}\n{}', True),
        (b'{}\n{"type":"transaction"}\n{}\n', False),
        (b'{}\n{"type":"transaction"}\n{}\n{"type":"event"}\n{}', True),
        (b'{}\n{"type":"attachment","length":3}\na\nb\n{"type":"event"}\n{}', True),
        (b'{}\n{"type":"attachment","length":2}\n\xc3\xa9\n{"type":"event"}\n{}', True),
        (b'{}\n{"type":"attachment","length":-1}\nx\n{"type":"event"}\n{}', False),
        (b'{}\n{"type":"attachment","length":true}\nx\n{"type":"event"}\n{}', False),
        (b'{}\n{"type":"attachment","length":1}\nab\n{"type":"event"}\n{}', False),
        (b'{}\n[]\n{"type":"event"}\n{}', False),
        (b"{}\n\xff\n{}", False),
        (b'{}\n{"type":"event"}', False),
    ],
)
def test_only_event_items_in_valid_envelope_framing_fail_smoke(body: bytes, expected: bool) -> None:
    assert envelope_has_event(body) == expected, f"envelope: {body!r}"


@pytest.mark.parametrize("hint", ["", "&kc_idp_hint=example", "&kc_idp_hint=&kc_idp_hint=example"])
def test_password_form_hint_preserves_the_authorize_request(hint: str) -> None:
    original = (
        "https://example.com/authorize?state=example&redirect_uri=%2Fauth%2Fcallback&scope=openid+email&blank="
        + hint
    )
    target = urlsplit(password_form_url(original))
    query = parse_qsl(target.query, keep_blank_values=True)
    untouched = [(key, value) for key, value in query if key != "kc_idp_hint"]
    expected = [
        (key, value)
        for key, value in parse_qsl(urlsplit(original).query, keep_blank_values=True)
        if key != "kc_idp_hint"
    ]

    assert target._replace(query="") == urlsplit(original)._replace(query="")
    assert untouched == expected
    assert [(key, value) for key, value in query if key == "kc_idp_hint"] == [("kc_idp_hint", "")]


def test_soft_verdict_reports_every_failed_check() -> None:
    checks = Checks()
    for number in range(4):
        with checks.check(f"Check {number}"):
            raise AssertionError(f"Failure {number}")

    with pytest.raises(AssertionError) as failure:
        checks.finish()
    for number in range(4):
        assert f"Check {number}: Failure {number}" in str(failure.value)


@pytest.mark.parametrize(
    "name",
    [
        "portal_dashboards",
        "portal_metrics",
        "portal_widgets",
        "portal_datasets",
        "custom_catalogue",
        "files_sharing",
    ],
)
def test_optional_capability_declarations_are_boolean(name: str) -> None:
    fields: dict[str, object] = {"ingestion": False, "idp": "keycloak"}
    assert not Capabilities.parse(fields).has(name)
    assert Capabilities.parse({**fields, name: True}).has(name)
    with pytest.raises(ManifestError, match="must be bool"):
        Capabilities.parse({**fields, name: "false"})


@pytest.mark.parametrize(
    "name", ["portal_dashboards", "portal_metrics", "portal_widgets", "portal_datasets"]
)
@pytest.mark.parametrize("catalogue", [None, False])
def test_readable_definitions_imply_the_custom_catalogue_service(
    name: str, catalogue: bool | None
) -> None:
    fields: dict[str, object] = {"ingestion": False, "idp": "keycloak", name: True}
    if catalogue is not None:
        fields["custom_catalogue"] = catalogue

    assert Capabilities.parse(fields).has("custom_catalogue"), f"declaration: {fields!r}"


@pytest.mark.parametrize(
    ("declared", "expected"),
    [
        ({}, ("custom_catalogue", "portal_metrics")),
        ({"custom_catalogue": True}, ("portal_metrics",)),
        ({"portal_metrics": True}, ()),
        ({"portal_widgets": True}, ("portal_metrics",)),
    ],
)
def test_first_metric_requires_its_own_definition_and_the_service(
    declared: dict[str, bool], expected: tuple[str, ...]
) -> None:
    capabilities = Capabilities.parse({"ingestion": False, "idp": "keycloak", **declared})
    scenario = next(page for page in PAGE_SCENARIOS if page.first == "/portal/custom/metrics/")

    assert missing_capabilities(scenario, capabilities) == expected, f"declaration: {declared!r}"


@pytest.mark.parametrize(
    ("declared", "expected"),
    [
        ({}, (False, False, False, False)),
        ({"custom_catalogue": True}, (True, False, False, False)),
        ({"portal_dashboards": True}, (True, True, False, False)),
        ({"files_sharing": True}, (False, False, True, False)),
        ({"ingestion": True}, (False, False, False, True)),
    ],
)
def test_navigation_requires_only_entries_supported_by_the_stand(
    declared: dict[str, bool], expected: tuple[bool, ...]
) -> None:
    capabilities = Capabilities.parse({"ingestion": False, "idp": "keycloak", **declared})
    selected: set[str] = set()
    for persona in PERSONAS:
        selected.update(scenario.title for scenario in scenarios_for(persona, capabilities))

    gated_titles = (
        "All dashboards list loads without errors",
        "First dashboard loads without errors",
        "Collaboration Files and sharing loads without errors",
        "Sources and connectors load without errors",
    )
    assert tuple(title in selected for title in gated_titles) == expected, declared
    assert "At a glance loads without errors" in selected
