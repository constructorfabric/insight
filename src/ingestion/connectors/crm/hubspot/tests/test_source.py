"""SourceHubspot: spec, check_connection, stream discovery, config parsing."""

from __future__ import annotations

import logging
import sys

import pendulum
import pytest
from airbyte_cdk.models import FailureType
from airbyte_cdk.utils.traced_exception import AirbyteTracedException
from source_hubspot import source as source_mod
from source_hubspot.constants import CURATED_STREAMS, STREAM_REGISTRY
from source_hubspot.source import SourceHubspot
from source_hubspot.streams import CrmArchivedListStream, CrmSearchStream, OwnersArchivedStream, OwnersStream

CONFIG = {
    "hubspot_access_token": "pat-test-token",
    "insight_tenant_id": "T",
    "insight_source_id": "S",
    "hubspot_start_date": "2024-01-01",
}


class FakeHubspotApi:
    """Configurable stand-in for the Hubspot api client used by the source."""

    check_reason = None
    properties_error = None
    association_reason = None

    def __init__(self, access_token):
        self.access_token = access_token

    def check_connection(self):
        return self.check_reason

    def properties_for(self, object_type):
        if self.properties_error is not None:
            raise self.properties_error
        return ()

    def probe_association_scope(self):
        return self.association_reason

    # streams() constructs streams around this instance
    def property_names(self, object_type):
        return ()

    def custom_property_names(self, object_type):
        return frozenset()

    def generate_schema(self, object_type):
        return {"type": "object", "properties": {}}


@pytest.fixture
def source(monkeypatch) -> SourceHubspot:
    # Reset class-level knobs so tests don't leak into each other.
    FakeHubspotApi.check_reason = None
    FakeHubspotApi.properties_error = None
    FakeHubspotApi.association_reason = None
    monkeypatch.setattr(source_mod, "Hubspot", FakeHubspotApi)
    return SourceHubspot()


class TestSpec:
    def test_spec_loads_from_package(self):
        spec = SourceHubspot().spec(logging.getLogger("airbyte"))
        required = spec.connectionSpecification["required"]
        assert set(required) == {"insight_tenant_id", "insight_source_id", "hubspot_access_token"}


class TestCheckConnection:
    def test_all_probes_pass(self, source):
        assert source.check_connection(logging.getLogger(), CONFIG) == (True, None)

    def test_token_check_failure(self, source):
        FakeHubspotApi.check_reason = "bad token"
        assert source.check_connection(logging.getLogger(), CONFIG) == (False, "bad token")

    def test_properties_probe_failure(self, source):
        FakeHubspotApi.properties_error = AirbyteTracedException(
            message="no scope", failure_type=FailureType.config_error
        )
        ok, reason = source.check_connection(logging.getLogger(), CONFIG)
        assert (ok, reason) == (False, "no scope")

    def test_association_probe_failure(self, source):
        FakeHubspotApi.association_reason = "no assoc scope"
        ok, reason = source.check_connection(logging.getLogger(), CONFIG)
        assert (ok, reason) == (False, "no assoc scope")

    def test_owners_only_skips_crm_probes(self, source, monkeypatch):
        # An owners-only stream list has no properties endpoint and no
        # associations — neither probe may run.
        FakeHubspotApi.properties_error = AssertionError("must not be called")
        FakeHubspotApi.association_reason = "must not be returned"
        monkeypatch.setattr(source, "_resolve_stream_list", lambda config: ["owners"])
        assert source.check_connection(logging.getLogger(), CONFIG) == (True, None)

    def test_unknown_stream_names_skipped(self, source, monkeypatch):
        monkeypatch.setattr(source, "_resolve_stream_list", lambda config: ["bogus", "owners"])
        assert source._pick_properties_probe_object(CONFIG) is None
        assert source._has_association_streams(CONFIG) is False

    def test_probe_object_is_first_crm_stream(self, source):
        assert source._pick_properties_probe_object(CONFIG) == "contacts"

    def test_has_association_streams(self, source):
        assert source._has_association_streams(CONFIG) is True


class TestStreams:
    def test_stream_classes_by_registry_shape(self, source):
        streams = {s.name: s for s in source.streams(CONFIG)}
        assert set(streams) == set(STREAM_REGISTRY)
        assert isinstance(streams["contacts"], CrmSearchStream)
        assert isinstance(streams["contacts_archived"], CrmArchivedListStream)
        assert isinstance(streams["owners"], OwnersStream)
        assert isinstance(streams["owners_archived"], OwnersArchivedStream)
        # All streams share the configured scope.
        assert streams["deals"]._tenant_id == "T"
        assert streams["deals"]._source_id == "S"
        assert streams["deals"]._start_date == pendulum.datetime(2024, 1, 1, tz="UTC")

    def test_resolve_stream_list_appends_archived_siblings(self, source):
        names = source._resolve_stream_list(CONFIG)
        assert names[: len(CURATED_STREAMS)] == list(CURATED_STREAMS)
        assert "contacts_archived" in names
        # meetings has archived_supported=False → no archived sibling.
        assert "engagements_meetings_archived" not in names


class TestResolveStartDate:
    def test_datetime_normalized_to_utc(self, source):
        got = source._resolve_start_date({**CONFIG, "hubspot_start_date": "2024-01-01T06:00:00+05:00"})
        assert got == pendulum.datetime(2024, 1, 1, 1, tz="UTC")
        assert got.timezone_name == "UTC"

    def test_date_only_string(self, source):
        got = source._resolve_start_date(CONFIG)
        assert got == pendulum.datetime(2024, 1, 1, tz="UTC")

    def test_date_object_branch(self, source, monkeypatch):
        # pendulum.parse returns DateTime for date strings by default; force
        # the Date branch to verify UTC-midnight normalization.
        monkeypatch.setattr(source_mod.pendulum, "parse", lambda raw: pendulum.date(2024, 2, 3))
        got = source._resolve_start_date(CONFIG)
        assert got == pendulum.datetime(2024, 2, 3, tz="UTC")

    def test_invalid_string_raises_config_error(self, source):
        with pytest.raises(AirbyteTracedException) as exc_info:
            source._resolve_start_date({**CONFIG, "hubspot_start_date": "garbage"})
        assert exc_info.value.failure_type == FailureType.config_error
        assert "Invalid hubspot_start_date" in exc_info.value.message

    def test_non_date_parse_result_raises(self, source, monkeypatch):
        monkeypatch.setattr(source_mod.pendulum, "parse", lambda raw: "not-a-datetime")
        with pytest.raises(AirbyteTracedException) as exc_info:
            source._resolve_start_date(CONFIG)
        assert "parsed as" in exc_info.value.message

    def test_fallback_is_two_years_ago(self, source):
        got = source._resolve_start_date({k: v for k, v in CONFIG.items() if k != "hubspot_start_date"})
        expected = pendulum.now("UTC").subtract(years=2)
        assert abs(got.diff(expected).in_seconds()) < 60


class TestMain:
    def test_main_launches_entrypoint(self, monkeypatch):
        import airbyte_cdk.entrypoint as entrypoint_mod

        called = {}
        monkeypatch.setattr(entrypoint_mod, "launch", lambda source, args: called.update(source=source, args=args))
        monkeypatch.setattr(sys, "argv", ["source-hubspot-insight", "spec"])
        source_mod.main()
        assert isinstance(called["source"], SourceHubspot)
        assert called["args"] == ["spec"]
