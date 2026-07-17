"""Hubspot api client: check_connection, property discovery, schema generation."""

from __future__ import annotations

import logging

import pytest
import requests
from airbyte_cdk.models import FailureType
from airbyte_cdk.utils import AirbyteTracedException
from source_hubspot import api as api_mod
from source_hubspot.api import Hubspot, _prop_to_json_schema, _TimeoutSession
from source_hubspot.constants import BASE_URL
from tests.conftest import FakeHttpClient, FakeResponse


def make_client(responses=()):
    hs = Hubspot("pat-test-token")
    hs._http_client = FakeHttpClient(responses)
    return hs


def prop(name, hubspot_defined=True, type_="string"):
    return {"name": name, "hubspotDefined": hubspot_defined, "type": type_}


class TestTimeoutSession:
    def test_default_timeout_injected(self, monkeypatch):
        captured = {}

        def fake_request(self, method, url, **kwargs):
            captured.update(kwargs)
            return "resp"

        monkeypatch.setattr(requests.Session, "request", fake_request)
        session = _TimeoutSession()
        assert session.request("GET", "https://x") == "resp"
        assert captured["timeout"] == (10, 120)

    def test_explicit_timeout_wins(self, monkeypatch):
        captured = {}
        monkeypatch.setattr(requests.Session, "request", lambda self, method, url, **kw: captured.update(kw))
        _TimeoutSession().request("GET", "https://x", timeout=5)
        assert captured["timeout"] == 5


class TestInit:
    def test_empty_token_rejected(self):
        with pytest.raises(ValueError, match="access_token is required"):
            Hubspot("")

    def test_bearer_header_installed(self):
        hs = Hubspot("pat-test-token")
        assert hs.session.headers["Authorization"] == "Bearer pat-test-token"


class TestCheckConnection:
    def test_success(self):
        hs = make_client([FakeResponse({"results": []})])
        assert hs.check_connection() is None
        call = hs._http_client.calls[0]
        assert call["url"] == f"{BASE_URL}/crm/v3/owners/"
        assert call["params"] == {"limit": 1}

    def test_traced_exception_message_returned(self):
        hs = make_client([AirbyteTracedException(message="bad token")])
        assert hs.check_connection() == "bad token"

    def test_transport_error_wrapped(self):
        hs = make_client([requests.ConnectionError("refused")])
        reason = hs.check_connection()
        assert "connectivity check failed" in reason
        assert "refused" in reason

    def test_non_ok_response_surfaced(self):
        hs = make_client([FakeResponse(status_code=404, text="nope")])
        reason = hs.check_connection()
        assert "HTTP 404" in reason and "nope" in reason


class TestPropertiesFor:
    def test_v3_results_wrapper(self):
        hs = make_client([FakeResponse({"results": [prop("email")]})])
        props = hs.properties_for("contacts")
        assert props == (prop("email"),)
        assert hs._http_client.calls[0]["url"] == f"{BASE_URL}/crm/v3/properties/contacts"

    def test_cached_per_object(self):
        hs = make_client([FakeResponse({"results": [prop("email")]})])
        hs.properties_for("contacts")
        hs.properties_for("contacts")  # second call must not hit HTTP
        assert len(hs._http_client.calls) == 1

    def test_v2_bare_list_accepted(self):
        hs = make_client([FakeResponse([prop("email")])])
        assert hs.properties_for("contacts") == (prop("email"),)

    def test_http_error_is_config_error(self):
        hs = make_client([FakeResponse(status_code=403, text="forbidden")])
        with pytest.raises(AirbyteTracedException) as exc_info:
            hs.properties_for("deals")
        assert exc_info.value.failure_type == FailureType.config_error
        assert "crm.objects" in exc_info.value.message

    def test_non_json_body_is_system_error(self):
        hs = make_client([FakeResponse(ValueError("not json"), text="<html>")])
        with pytest.raises(AirbyteTracedException) as exc_info:
            hs.properties_for("deals")
        assert exc_info.value.failure_type == FailureType.system_error
        assert "non-JSON" in exc_info.value.message

    def test_unexpected_shape_is_system_error(self):
        hs = make_client([FakeResponse("just a string")])
        with pytest.raises(AirbyteTracedException) as exc_info:
            hs.properties_for("deals")
        assert "Unexpected HubSpot properties payload shape" in exc_info.value.message


class TestPropertySelection:
    DESCRIPTORS = [
        prop("amount"),  # hubspotDefined + curated → kept
        prop("uncurated_std"),  # hubspotDefined, not curated → dropped
        prop("my_custom", hubspot_defined=False),  # custom → always kept
        {"hubspotDefined": True},  # nameless → dropped
    ]

    def test_property_names_curated_plus_custom(self):
        hs = make_client([FakeResponse({"results": self.DESCRIPTORS})])
        assert hs.property_names("deals") == ("amount", "my_custom")

    def test_custom_property_names(self):
        hs = make_client([FakeResponse({"results": self.DESCRIPTORS})])
        assert hs.custom_property_names("deals") == frozenset({"my_custom"})

    def test_unknown_object_has_no_curated_names(self):
        hs = make_client([FakeResponse({"results": [prop("anything")]})])
        assert hs.property_names("unknown_object") == ()


class TestGenerateSchema:
    def test_curated_props_added_with_string_type(self):
        hs = make_client(
            [
                FakeResponse(
                    {
                        "results": [
                            prop("amount", type_="number"),
                            prop("uncurated_std"),
                            prop("my_custom", hubspot_defined=False),
                        ]
                    }
                )
            ]
        )
        schema = hs.generate_schema("deals")
        props = schema["properties"]
        # Base record fields always present.
        assert props["id"] == {"type": ["string", "null"]}
        assert props["archivedAt"]["format"] == "date-time"
        # number maps to string on purpose (Bronze stays Nullable(String)).
        assert props["properties_amount"] == {"type": ["string", "null"]}
        assert "properties_uncurated_std" not in props
        assert "properties_my_custom" not in props  # customs ride in custom_fields

    def test_unknown_type_warns_once(self, caplog):
        hs = make_client(
            [
                FakeResponse(
                    {
                        "results": [
                            {"name": "amount", "hubspotDefined": True, "type": "alien"},
                            {"name": "closedate", "hubspotDefined": True, "type": "alien"},
                        ]
                    }
                )
            ]
        )
        with caplog.at_level(logging.WARNING, logger="airbyte"):
            schema = hs.generate_schema("deals")
        assert schema["properties"]["properties_amount"] == {"type": ["string", "null"]}
        assert caplog.text.count("Unknown HubSpot property type") == 1

    def test_prop_to_json_schema_format_passthrough(self, monkeypatch):
        # No current mapping carries a format — patch one in to cover the
        # format branch.
        monkeypatch.setitem(api_mod.HUBSPOT_TYPE_TO_JSON_SCHEMA, "datetime", ("string", "date-time"))
        out = _prop_to_json_schema({"name": "x", "type": "datetime"}, set())
        assert out == {"type": ["string", "null"], "format": "date-time"}

    def test_prop_to_json_schema_defaults_missing_type_to_string(self):
        assert _prop_to_json_schema({"name": "x"}, set()) == {"type": ["string", "null"]}


class TestProbeAssociationScope:
    def test_success(self):
        hs = make_client([FakeResponse({"status": "COMPLETE", "results": []})])
        assert hs.probe_association_scope() is None
        call = hs._http_client.calls[0]
        assert call["url"] == (f"{BASE_URL}/crm/v4/associations/contacts/companies/batch/read")
        assert call["json"] == {"inputs": [{"id": "1"}]}

    def test_config_error_surfaced(self):
        hs = make_client([AirbyteTracedException(message="missing scope", failure_type=FailureType.config_error)])
        assert hs.probe_association_scope() == "missing scope"

    def test_transient_traced_error_swallowed(self, caplog):
        hs = make_client([AirbyteTracedException(message="flaky", failure_type=FailureType.transient_error)])
        with caplog.at_level(logging.WARNING, logger="airbyte"):
            assert hs.probe_association_scope() is None
        assert "transient error" in caplog.text

    def test_unexpected_error_swallowed(self, caplog):
        hs = make_client([RuntimeError("boom")])
        with caplog.at_level(logging.WARNING, logger="airbyte"):
            assert hs.probe_association_scope() is None
        assert "unexpected error" in caplog.text
