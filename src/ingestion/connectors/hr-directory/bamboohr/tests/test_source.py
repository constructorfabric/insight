from __future__ import annotations

import logging

import pytest
from conftest import FakeClient
from source_bamboohr.client import BambooHrAuthError
from source_bamboohr.source import SourceBamboohr

CONFIG = {
    "insight_tenant_id": "T",
    "insight_source_id": "S",
    "bamboohr_api_key": "key",
    "bamboohr_domain": "acme",
}

logger = logging.getLogger("test")


@pytest.fixture
def probe(monkeypatch):
    def install(client):
        monkeypatch.setattr("source_bamboohr.source._client", lambda _config: client)
        return client

    return install


class TestCheckConnection:
    @pytest.mark.parametrize("field", sorted(CONFIG))
    @pytest.mark.parametrize("value", ["", "   ", None], ids=["empty", "blank", "missing"])
    def test_a_blank_required_field_fails_before_any_request(self, field, value, probe):
        probe(FakeClient({}))
        config = {**CONFIG, field: value}

        ok, message = SourceBamboohr().check_connection(logger, config)
        assert not ok, f"should reject {field}={value!r}"
        assert field in message

    def test_a_domain_that_is_not_a_subdomain_is_rejected_by_name(self, monkeypatch):
        ok, message = SourceBamboohr().check_connection(
            logger, {**CONFIG, "bamboohr_domain": "evil.example.com"}
        )
        assert not ok
        assert "bamboohr_domain" in message

    def test_a_reachable_instance_passes(self, probe):
        probe(FakeClient({"meta/fields": []}))
        assert SourceBamboohr().check_connection(logger, CONFIG) == (True, None)

    def test_a_rejected_key_is_reported_with_its_remedy(self, probe):
        probe(FakeClient({"meta/fields": BambooHrAuthError(401, "url", "the API key was rejected")}))

        ok, message = SourceBamboohr().check_connection(logger, CONFIG)
        assert not ok
        assert "rejected" in message

    def test_an_unreachable_host_is_reported(self, probe):
        probe(FakeClient({"meta/fields": OSError("name resolution failed")}))

        ok, message = SourceBamboohr().check_connection(logger, CONFIG)
        assert not ok
        assert "acme" in message


class TestWiring:
    def test_the_spec_is_the_shipped_connection_specification(self):
        spec = SourceBamboohr().spec(logger)
        properties = spec.connectionSpecification["properties"]

        assert set(spec.connectionSpecification["required"]) == set(CONFIG)
        assert "bamboohr_employees_custom_fields" not in properties

    def test_all_three_streams_are_wired(self, probe):
        probe(FakeClient({}))
        names = [stream.name for stream in SourceBamboohr().streams(CONFIG)]
        assert names == ["employees", "leave_requests", "meta_fields"]

    def test_every_stream_declares_a_schema_keyed_on_unique_key(self, probe):
        probe(FakeClient({}))
        for stream in SourceBamboohr().streams(CONFIG):
            schema = stream.get_json_schema()
            assert schema["required"] == ["unique_key"], f"{stream.name} schema"
            assert schema["properties"]["unique_key"] == {"type": "string"}, f"{stream.name} schema"
