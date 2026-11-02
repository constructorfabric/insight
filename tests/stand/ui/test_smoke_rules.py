"""Pure smoke boundary rules: envelope framing, authorize queries and soft verdicts.

These tests need neither a browser nor an API connection. Browser journeys
consume these same parsers and aggregation rules.
"""

from urllib.parse import parse_qsl, urlsplit

import pytest
from insight_stand import Capabilities, ManifestError

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
    "name", ["portal_dashboards", "portal_metrics", "portal_widgets", "portal_datasets"]
)
def test_definition_capabilities_require_explicit_boolean_declarations(name: str) -> None:
    fields: dict[str, object] = {"ingestion": False, "idp": "keycloak"}
    assert not Capabilities.parse(fields).has(name)
    assert Capabilities.parse({**fields, name: True}).has(name)
    with pytest.raises(ManifestError, match="must be bool"):
        Capabilities.parse({**fields, name: "false"})
