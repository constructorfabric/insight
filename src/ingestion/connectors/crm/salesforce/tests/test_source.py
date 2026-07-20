"""Tests for source_salesforce.source: config parsing, check, stream assembly."""

from __future__ import annotations

import json
import logging
from datetime import timedelta
from unittest.mock import Mock

import pytest
import source_salesforce.source as source_module
from airbyte_cdk.models import (
    AirbyteStream,
    ConfiguredAirbyteCatalog,
    ConfiguredAirbyteStream,
    DestinationSyncMode,
    SyncMode,
)
from airbyte_cdk.utils.traced_exception import AirbyteTracedException
from requests import exceptions
from source_salesforce.source import SourceSalesforce
from source_salesforce.streams import IncrementalRestSalesforceStream, RestSalesforceStream, RestSalesforceSubStream
from tests.conftest import ACCOUNT_SCHEMA, CONFIG, make_http_response, make_sf

logger = logging.getLogger("test")


def make_source(config=None, catalog=None, state=None) -> SourceSalesforce:
    return SourceSalesforce(catalog, CONFIG if config is None else config, state)


def make_catalog(name: str, sync_mode: SyncMode = SyncMode.incremental) -> ConfiguredAirbyteCatalog:
    return ConfiguredAirbyteCatalog(
        streams=[
            ConfiguredAirbyteStream(
                stream=AirbyteStream(
                    name=name, json_schema={}, supported_sync_modes=[SyncMode.full_refresh, SyncMode.incremental]
                ),
                sync_mode=sync_mode,
                destination_sync_mode=DestinationSyncMode.append,
            )
        ]
    )


# ---------------------------------------------------------------------------
# Constructor: concurrency parsing
# ---------------------------------------------------------------------------


class TestConcurrencyConfig:
    @pytest.fixture
    def created(self, monkeypatch):
        """Spy on ConcurrentSource.create to capture the resolved concurrency."""
        captured = {}
        original = source_module.ConcurrentSource.create

        def spy(num_workers, initial_number_of_partitions_to_generate, *args, **kwargs):
            captured["workers"] = num_workers
            captured["initial_partitions"] = initial_number_of_partitions_to_generate
            return original(num_workers, initial_number_of_partitions_to_generate, *args, **kwargs)

        monkeypatch.setattr(source_module.ConcurrentSource, "create", spy)
        return captured

    def test_string_value_parsed(self, created):
        make_source({**CONFIG, "salesforce_num_workers": "8"})
        assert created["workers"] == 8
        assert created["initial_partitions"] == 4

    def test_invalid_value_falls_back_to_default(self, created):
        make_source({**CONFIG, "salesforce_num_workers": "not-a-number"})
        assert created["workers"] == 1

    def test_clamped_to_max(self, created):
        make_source({**CONFIG, "salesforce_num_workers": 999})
        assert created["workers"] == 50

    def test_clamped_to_min(self, created):
        make_source({**CONFIG, "salesforce_num_workers": 0})
        assert created["workers"] == 1

    def test_no_config_uses_default(self, created):
        make_source(config=False)  # falsy config -> default concurrency
        assert created["workers"] == 1


# ---------------------------------------------------------------------------
# Spec + validation helpers
# ---------------------------------------------------------------------------


class TestSpec:
    def test_spec_loads_with_prefixed_keys(self):
        spec = make_source().spec(logger)
        props = spec.connectionSpecification["properties"]
        assert "salesforce_instance_url" in props
        assert "salesforce_client_id" in props
        assert "insight_tenant_id" in props


class TestToTimedelta:
    def test_timedelta_passthrough(self):
        delta = timedelta(days=3)
        assert SourceSalesforce._to_timedelta(delta) is delta

    def test_duration_with_totimedelta(self):
        import isodate

        result = SourceSalesforce._to_timedelta(isodate.parse_duration("P1M"))
        assert isinstance(result, timedelta)

    def test_object_without_totimedelta(self):
        assert SourceSalesforce._to_timedelta(object()) is None

    def test_totimedelta_raising_returns_none(self):
        broken = Mock()
        broken.totimedelta.side_effect = ValueError("nope")
        assert SourceSalesforce._to_timedelta(broken) is None


class TestValidateStreamSliceStep:
    def test_valid_and_empty_pass(self):
        SourceSalesforce._validate_stream_slice_step("P30D")
        SourceSalesforce._validate_stream_slice_step("")
        SourceSalesforce._validate_stream_slice_step(None)

    def test_too_small_rejected(self):
        with pytest.raises(AirbyteTracedException, match="too small"):
            SourceSalesforce._validate_stream_slice_step("PT0.5S")

    def test_garbage_rejected(self):
        with pytest.raises(AirbyteTracedException):
            SourceSalesforce._validate_stream_slice_step("30 days")

    def test_uncomparable_duration_rejected(self, monkeypatch):
        monkeypatch.setattr(SourceSalesforce, "_to_timedelta", staticmethod(lambda d: None))
        with pytest.raises(AirbyteTracedException, match="ISO 8601"):
            SourceSalesforce._validate_stream_slice_step("P1M")


class TestValidateLookbackWindow:
    def test_valid_and_empty_pass(self):
        SourceSalesforce._validate_lookback_window("PT10M")
        SourceSalesforce._validate_lookback_window("")
        SourceSalesforce._validate_lookback_window(None)

    def test_negative_rejected(self):
        with pytest.raises(AirbyteTracedException, match="lookback_window value is invalid"):
            SourceSalesforce._validate_lookback_window("-PT10M")

    def test_garbage_rejected(self):
        with pytest.raises(AirbyteTracedException, match="lookback_window value is invalid"):
            SourceSalesforce._validate_lookback_window("ten minutes")

    def test_uncomparable_duration_rejected(self, monkeypatch):
        monkeypatch.setattr(SourceSalesforce, "_to_timedelta", staticmethod(lambda d: None))
        with pytest.raises(AirbyteTracedException, match="ISO 8601"):
            SourceSalesforce._validate_lookback_window("P1M")


# ---------------------------------------------------------------------------
# check_connection + _get_sf_object
# ---------------------------------------------------------------------------


class TestGetSfObject:
    def test_builds_client_and_logs_in(self, monkeypatch):
        instances = []

        class FakeSalesforce:
            def __init__(self, **kwargs):
                self.kwargs = kwargs
                self.login_called = False
                instances.append(self)

            def login(self):
                self.login_called = True

        monkeypatch.setattr(source_module, "Salesforce", FakeSalesforce)
        sf = SourceSalesforce._get_sf_object(CONFIG)
        assert sf.login_called is True
        assert sf.kwargs == {
            "instance_url": CONFIG["salesforce_instance_url"],
            "client_id": CONFIG["salesforce_client_id"],
            "client_secret": CONFIG["salesforce_client_secret"],
            "start_date": CONFIG["salesforce_start_date"],
        }


class TestCheckConnection:
    def test_success(self, monkeypatch):
        sf = Mock()
        monkeypatch.setattr(SourceSalesforce, "_get_sf_object", staticmethod(lambda config: sf))
        source = make_source()
        assert source.check_connection(logger, CONFIG) == (True, None)
        sf.describe.assert_called_once()

    def test_invalid_slice_step_fails_before_login(self, monkeypatch):
        get_sf = Mock()
        monkeypatch.setattr(SourceSalesforce, "_get_sf_object", staticmethod(get_sf))
        source = make_source()
        with pytest.raises(AirbyteTracedException):
            source.check_connection(logger, {**CONFIG, "salesforce_stream_slice_step": "bad"})
        get_sf.assert_not_called()

    def test_invalid_lookback_fails_before_login(self, monkeypatch):
        get_sf = Mock()
        monkeypatch.setattr(SourceSalesforce, "_get_sf_object", staticmethod(get_sf))
        source = make_source()
        with pytest.raises(AirbyteTracedException):
            source.check_connection(logger, {**CONFIG, "salesforce_lookback_window": "bad"})
        get_sf.assert_not_called()


# ---------------------------------------------------------------------------
# Stream type selection + assembly
# ---------------------------------------------------------------------------


class TestGetStreamType:
    def test_substream_for_parented_object(self):
        full_refresh, incremental = SourceSalesforce._get_stream_type("ContentDocumentLink")
        assert full_refresh is RestSalesforceSubStream
        assert incremental is IncrementalRestSalesforceStream

    def test_plain_rest_otherwise(self):
        full_refresh, _ = SourceSalesforce._get_stream_type("Account")
        assert full_refresh is RestSalesforceStream


def _sf_stub():
    """Real Salesforce client with describe-time HTTP monkeypatched out."""
    sf = make_sf()
    sf.generate_schemas = Mock(
        side_effect=lambda stream_objects: {name: dict(ACCOUNT_SCHEMA) for name in stream_objects}
    )
    sf.get_custom_field_names = Mock(return_value=frozenset({"Custom__c"}))
    return sf


class TestPrepareStream:
    def test_incremental_when_replication_key_present(self):
        source = make_source()
        stream_class, kwargs = source.prepare_stream(
            "Account", ACCOUNT_SCHEMA, {"queryable": True}, _sf_stub(), Mock(), CONFIG
        )
        assert stream_class is IncrementalRestSalesforceStream
        assert kwargs["replication_key"] == "SystemModstamp"
        assert kwargs["stream_slice_step"] == "P30D"
        assert kwargs["tenant_id"] == CONFIG["insight_tenant_id"]
        assert kwargs["source_id"] == CONFIG["insight_source_id"]
        assert kwargs["custom_field_names"] == frozenset({"Custom__c"})

    def test_full_refresh_without_replication_key(self):
        source = make_source()
        schema = {"properties": {"Id": {"type": ["string", "null"]}}}
        stream_class, kwargs = source.prepare_stream("Account", schema, {}, _sf_stub(), Mock(), CONFIG)
        assert stream_class is RestSalesforceStream
        assert "replication_key" not in kwargs

    def test_unsupported_filtering_forces_full_refresh(self):
        source = make_source()
        stream_class, _ = source.prepare_stream("LoginEvent", ACCOUNT_SCHEMA, {}, _sf_stub(), Mock(), CONFIG)
        assert stream_class is RestSalesforceStream

    def test_slice_step_from_config(self):
        source = make_source()
        _, kwargs = source.prepare_stream(
            "Account", ACCOUNT_SCHEMA, {}, _sf_stub(), Mock(), {**CONFIG, "salesforce_stream_slice_step": "P7D"}
        )
        assert kwargs["stream_slice_step"] == "P7D"


class TestGenerateStreams:
    def test_incremental_stream_wrapped_with_cursor(self):
        source = make_source()
        streams = source.generate_streams(CONFIG, {"Account": {"queryable": True}}, _sf_stub())
        assert [s.name for s in streams] == ["Account"]
        # Facade wraps the legacy stream; the slicer cursor must be attached.
        assert streams[0].cursor_field == "SystemModstamp"

    def test_substream_gets_parent_stream(self):
        sf = _sf_stub()
        source = make_source()
        streams = source.generate_streams(
            CONFIG, {"ContentDocumentLink": {}, "ContentDocument": {"queryable": True}}, sf
        )
        names = {s.name for s in streams}
        assert names == {"ContentDocumentLink", "ContentDocument"}

    def test_full_refresh_catalog_disables_cursor(self):
        source = make_source(catalog=make_catalog("Account", SyncMode.full_refresh))
        streams = source.generate_streams(CONFIG, {"Account": {}}, _sf_stub())
        assert len(streams) == 1
        # Full-refresh catalog entry -> FinalStateCursor -> facade has no state.
        assert streams[0].cursor_field == "SystemModstamp"

    def test_lookback_and_slice_step_from_config(self):
        source = make_source()
        config = {**CONFIG, "salesforce_lookback_window": "PT10M", "salesforce_stream_slice_step": "P7D"}
        streams = source.generate_streams(config, {"Account": {}}, _sf_stub())
        assert len(streams) == 1


class TestStreams:
    def test_streams_injects_default_start_date(self, monkeypatch):
        sf = Mock()
        sf.get_validated_streams.return_value = {}
        monkeypatch.setattr(SourceSalesforce, "_get_sf_object", staticmethod(lambda config: sf))
        seen = {}

        def fake_generate(config, stream_objects, sf_object):
            seen["config"] = config
            return []

        source = make_source()
        monkeypatch.setattr(source, "generate_streams", fake_generate)
        config = dict(CONFIG)
        del config["salesforce_start_date"]
        assert source.streams(config) == []
        # Two-year default lookback injected in DATETIME_FORMAT.
        injected = seen["config"]["salesforce_start_date"]
        assert injected.endswith("Z") and len(injected) == 20

    def test_streams_keeps_explicit_start_date(self, monkeypatch):
        sf = Mock()
        sf.get_validated_streams.return_value = {}
        monkeypatch.setattr(SourceSalesforce, "_get_sf_object", staticmethod(lambda config: sf))
        source = make_source()
        monkeypatch.setattr(source, "generate_streams", lambda config, objs, sf_obj: [])
        source.streams(CONFIG)
        sf.get_validated_streams.assert_called_once_with(catalog=None)


class TestCreateStreamSlicerCursor:
    def test_nested_cursor_field_rejected(self):
        source = make_source()
        stream = Mock()
        stream.cursor_field = ["Nested", "Cursor"]
        with pytest.raises(AssertionError, match="Nested cursor field are not supported"):
            source._create_stream_slicer_cursor(CONFIG, Mock(), stream)


class TestSyncModeFromCatalog:
    def test_no_catalog_returns_none(self):
        source = make_source()
        assert source._get_sync_mode_from_catalog(Mock(name="Account")) is None

    def test_matching_stream_returns_mode(self):
        source = make_source(catalog=make_catalog("Account", SyncMode.incremental))
        stream = Mock()
        stream.name = "Account"
        assert source._get_sync_mode_from_catalog(stream) == SyncMode.incremental

    def test_unmatched_stream_returns_none(self):
        source = make_source(catalog=make_catalog("Contact"))
        stream = Mock()
        stream.name = "Account"
        assert source._get_sync_mode_from_catalog(stream) is None


# ---------------------------------------------------------------------------
# read / _read_stream error decoration
# ---------------------------------------------------------------------------


class TestRead:
    def test_read_saves_catalog_and_delegates(self, monkeypatch):
        sentinel = object()
        monkeypatch.setattr(
            source_module.ConcurrentSourceAdapter,
            "read",
            lambda self, logger_, config, catalog, state=None: iter([sentinel]),
        )
        source = make_source()
        catalog = make_catalog("Account")
        out = list(source.read(logger, CONFIG, catalog))
        assert out == [sentinel]
        assert source.catalog is catalog


class TestReadStream:
    def _run(self, monkeypatch, response) -> tuple[Mock, Exception]:
        def raising(self, *args, **kwargs):
            raise exceptions.HTTPError("boom", response=response)
            yield  # pragma: no cover

        monkeypatch.setattr(source_module.ConcurrentSourceAdapter, "_read_stream", raising)
        source = make_source()
        mock_logger = Mock()
        with pytest.raises(exceptions.HTTPError) as exc_info:
            list(source._read_stream(mock_logger, Mock(), Mock(), Mock(), Mock()))
        return mock_logger, exc_info.value

    def test_request_limit_exceeded_logs_and_reraises(self, monkeypatch):
        response = make_http_response(403, [{"errorCode": "REQUEST_LIMIT_EXCEEDED", "message": "quota"}])
        mock_logger, _ = self._run(monkeypatch, response)
        assert mock_logger.warning.called
        assert "rate limit" in mock_logger.warning.call_args[0][0]

    def test_dict_payload_parsed(self, monkeypatch):
        response = make_http_response(403, {"error": "REQUEST_LIMIT_EXCEEDED", "error_description": "quota"})
        mock_logger, _ = self._run(monkeypatch, response)
        assert mock_logger.warning.called

    def test_other_http_error_reraised_without_warning(self, monkeypatch):
        response = make_http_response(400, [{"errorCode": "MALFORMED_QUERY", "message": "x"}])
        mock_logger, _ = self._run(monkeypatch, response)
        mock_logger.warning.assert_not_called()

    def test_non_json_body_reraised(self, monkeypatch):
        response = make_http_response(500, content=b"<html>oops</html>")
        mock_logger, _ = self._run(monkeypatch, response)
        mock_logger.warning.assert_not_called()

    def test_success_passthrough(self, monkeypatch):
        record = object()
        monkeypatch.setattr(
            source_module.ConcurrentSourceAdapter, "_read_stream", lambda self, *args, **kwargs: iter([record])
        )
        source = make_source()
        assert list(source._read_stream(Mock(), Mock(), Mock(), Mock(), Mock())) == [record]


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------


class TestMain:
    def test_main_wires_entrypoint(self, monkeypatch, tmp_path):
        config_path = tmp_path / "config.json"
        config_path.write_text(json.dumps(CONFIG))
        launched = {}

        import airbyte_cdk.entrypoint as entrypoint_module

        def fake_launch(source, args):
            launched["source"] = source
            launched["args"] = args

        monkeypatch.setattr(entrypoint_module, "launch", fake_launch)
        monkeypatch.setattr("sys.argv", ["source-salesforce-insight", "check", "--config", str(config_path)])
        source_module.main()
        assert isinstance(launched["source"], SourceSalesforce)
        assert launched["args"] == ["check", "--config", str(config_path)]
