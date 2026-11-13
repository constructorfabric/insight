from __future__ import annotations

import logging

import pytest
from airbyte_cdk.models import FailureType
from airbyte_cdk.utils.traced_exception import AirbyteTracedException
from conftest import FakeClient, config
from source_bamboohr.client import BambooHrAuthError
from source_bamboohr.config import REMEDIES
from source_bamboohr.source import SourceBamboohr

CONFIG = config()

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
        ok, message = SourceBamboohr().check_connection(logger, {**CONFIG, field: value})
        assert not ok, f"should reject {field}={value!r}"
        assert message == REMEDIES[field]

    def test_a_field_of_the_wrong_type_is_rejected_without_quoting_it(self, probe):
        probe(FakeClient({}))

        ok, message = SourceBamboohr().check_connection(logger, {**CONFIG, "bamboohr_start_date": ["s3cret"]})
        assert not ok
        assert "bamboohr_start_date" in message
        assert "s3cret" not in message

    def test_a_domain_that_is_not_a_subdomain_is_rejected_by_name(self, monkeypatch):
        ok, message = SourceBamboohr().check_connection(logger, {**CONFIG, "bamboohr_domain": "evil.example.com"})
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

    def test_an_answer_that_is_not_json_fails_the_check(self, probe):
        probe(FakeClient({"meta/fields": b"<html>login</html>"}))

        ok, message = SourceBamboohr().check_connection(logger, CONFIG)
        assert not ok
        assert "json_invalid" in message

    def test_an_unreachable_host_is_reported(self, probe):
        probe(FakeClient({"meta/fields": OSError("name resolution failed")}))

        ok, message = SourceBamboohr().check_connection(logger, CONFIG)
        assert not ok
        assert "acme" in message


class TestStreamsConfig:
    @pytest.mark.parametrize("field", sorted(REMEDIES))
    @pytest.mark.parametrize("value", ["", "   ", None], ids=["empty", "blank", "missing"])
    def test_a_blank_required_field_is_a_config_error(self, field, value, probe):
        probe(FakeClient({}))

        with pytest.raises(AirbyteTracedException) as caught:
            SourceBamboohr().streams({**CONFIG, field: value})

        assert caught.value.failure_type == FailureType.config_error
        assert caught.value.message == REMEDIES[field]
        assert caught.value.__cause__ is None


class TestWiring:
    def test_the_spec_is_the_shipped_connection_specification(self):
        spec = SourceBamboohr().spec(logger)
        properties = spec.connectionSpecification["properties"]

        assert set(spec.connectionSpecification["required"]) == set(CONFIG)
        assert "bamboohr_employees_custom_fields" not in properties

    def test_all_four_streams_are_wired(self, probe):
        probe(FakeClient({}))
        names = [stream.name for stream in SourceBamboohr().streams(CONFIG)]
        assert names == ["employees", "leave_requests", "meta_fields", "whos_out"]

    def test_every_stream_declares_a_schema_keyed_on_unique_key(self, probe):
        probe(FakeClient({}))
        for stream in SourceBamboohr().streams(CONFIG):
            schema = stream.get_json_schema()
            assert schema["required"] == ["unique_key"], f"{stream.name} schema"
            assert schema["properties"]["unique_key"] == {"type": "string"}, f"{stream.name} schema"
