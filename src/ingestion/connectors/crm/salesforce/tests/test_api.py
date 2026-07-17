"""Tests for source_salesforce.api: token provider, authenticator, client."""

from __future__ import annotations

import time
from unittest.mock import Mock

import pytest
from airbyte_cdk.models import (
    AirbyteStream,
    ConfiguredAirbyteCatalog,
    ConfiguredAirbyteStream,
    DestinationSyncMode,
    SyncMode,
)
from airbyte_cdk.utils import AirbyteTracedException
from requests.exceptions import RequestException
from source_salesforce.api import Salesforce, SalesforceAuthenticator, SalesforceTokenProvider
from source_salesforce.constants import CRM_STREAMS, TOKEN_REFRESH_INTERVAL_SECONDS
from source_salesforce.exceptions import TypeSalesforceException
from tests.conftest import INSTANCE_URL, FakeResponse, make_sf


def _stub_send_request(sf: Salesforce, responses):
    """Replace the client's HttpClient with a stub recording every request."""
    calls = []
    seq = list(responses)

    def send_request(http_method, url, **kwargs):
        calls.append({"method": http_method, "url": url, **kwargs})
        return None, seq.pop(0)

    sf._http_client = Mock(send_request=send_request)
    return calls


# ---------------------------------------------------------------------------
# SalesforceTokenProvider / SalesforceAuthenticator
# ---------------------------------------------------------------------------


class TestTokenProvider:
    def test_fresh_token_not_refreshed(self, sf):
        sf.access_token = "tok"
        sf.login = Mock()
        assert sf._token_provider.get_token() == "tok"
        sf.login.assert_not_called()

    def test_stale_token_triggers_login(self, sf):
        sf.access_token = "old"
        sf.login = Mock(side_effect=lambda: setattr(sf, "access_token", "new"))
        provider = sf._token_provider
        provider._last_refresh_time = time.monotonic() - TOKEN_REFRESH_INTERVAL_SECONDS - 1
        assert provider.get_token() == "new"
        sf.login.assert_called_once()
        # Refresh timestamp advanced — the next call must not refresh again.
        assert provider.get_token() == "new"
        sf.login.assert_called_once()

    def test_refresh_failure_falls_back_to_existing_token(self, sf):
        sf.access_token = "old"
        sf.login = Mock(side_effect=RequestException("boom"))
        provider = sf._token_provider
        stale = time.monotonic() - TOKEN_REFRESH_INTERVAL_SECONDS - 1
        provider._last_refresh_time = stale
        assert provider.get_token() == "old"
        # Timestamp untouched on failure, so the next call retries the login.
        assert provider._last_refresh_time == stale

    def test_double_check_inside_lock(self, sf):
        # Another worker "refreshed" between the outer check and the lock:
        # simulate by resetting the timestamp from within login itself.
        provider = SalesforceTokenProvider(sf)
        sf.access_token = "tok"
        provider._last_refresh_time = time.monotonic() - TOKEN_REFRESH_INTERVAL_SECONDS - 1

        class ResettingLock:
            def __enter__(self_lock):
                provider._last_refresh_time = time.monotonic()

            def __exit__(self_lock, *args):
                return False

        provider._lock = ResettingLock()
        sf.login = Mock()
        assert provider.get_token() == "tok"
        sf.login.assert_not_called()

    def test_force_refresh_success(self, sf):
        sf.login = Mock(side_effect=lambda: setattr(sf, "access_token", "fresh"))
        provider = sf._token_provider
        provider._last_refresh_time = 0.0
        provider.force_refresh()
        sf.login.assert_called_once()
        assert provider._last_refresh_time > 0.0
        assert sf.access_token == "fresh"

    def test_force_refresh_failure_swallowed(self, sf):
        sf.login = Mock(side_effect=RequestException("down"))
        provider = sf._token_provider
        provider._last_refresh_time = 0.0
        provider.force_refresh()  # must not raise
        assert provider._last_refresh_time == 0.0


class TestAuthenticator:
    def test_bearer_header_reads_token_through_provider(self, sf):
        sf.access_token = "abc"
        auth = SalesforceAuthenticator(sf._token_provider)
        assert auth.auth_header == "Authorization"
        assert auth.token == "Bearer abc"
        # Token is re-read on every access (not frozen at construction).
        sf.access_token = "def"
        assert auth.token == "Bearer def"


# ---------------------------------------------------------------------------
# Salesforce client: init + login
# ---------------------------------------------------------------------------


class TestInit:
    def test_trailing_slash_stripped(self):
        assert make_sf(instance_url=INSTANCE_URL + "/").instance_url == INSTANCE_URL

    def test_missing_instance_url_raises(self):
        with pytest.raises(ValueError, match="instance_url is required"):
            make_sf(instance_url="")

    def test_extra_kwargs_ignored(self):
        sf = make_sf(unknown_key="x", start_date="2024-01-01")
        assert sf.start_date == "2024-01-01"


class TestLogin:
    def test_success_sets_access_token(self, sf):
        calls = _stub_send_request(sf, [FakeResponse({"access_token": "tok"})])
        sf.login()
        assert sf.access_token == "tok"
        assert calls[0]["method"] == "POST"
        assert calls[0]["url"] == f"{INSTANCE_URL}/services/oauth2/token"
        assert calls[0]["data"]["grant_type"] == "client_credentials"

    def test_http_error_is_config_error(self, sf):
        _stub_send_request(sf, [FakeResponse({"error": "invalid_client"}, status_code=400)])
        with pytest.raises(AirbyteTracedException, match="OAuth login failed"):
            sf.login()

    def test_non_json_body_raises(self, sf):
        _stub_send_request(sf, [FakeResponse(ValueError("not json"))])
        with pytest.raises(AirbyteTracedException, match="non-JSON response"):
            sf.login()

    def test_missing_access_token_raises(self, sf):
        _stub_send_request(sf, [FakeResponse({"token_type": "Bearer"})])
        with pytest.raises(AirbyteTracedException, match="missing access_token"):
            sf.login()


# ---------------------------------------------------------------------------
# describe + schema generation
# ---------------------------------------------------------------------------

ACCOUNT_DESCRIBE = {
    "name": "Account",
    "fields": [
        {"name": "Id", "type": "id", "custom": False},
        {"name": "Name", "type": "string", "custom": False},
        {"name": "AnnualRevenue", "type": "currency", "custom": False},
        {"name": "Custom__c", "type": "string", "custom": True},
    ],
}


class TestDescribe:
    def test_global_describe_url(self, sf):
        calls = _stub_send_request(sf, [FakeResponse({"sobjects": []})])
        assert sf.describe() == {"sobjects": []}
        assert calls[0]["url"] == (f"{INSTANCE_URL}/services/data/{sf.version}/sobjects")

    def test_sobject_describe_url_and_auth_header(self, sf):
        sf.access_token = "tok"
        calls = _stub_send_request(sf, [FakeResponse(ACCOUNT_DESCRIBE)])
        assert sf.describe("Account") == ACCOUNT_DESCRIBE
        assert calls[0]["url"].endswith("/sobjects/Account/describe")
        assert calls[0]["headers"]["Authorization"] == "Bearer tok"

    def test_404_for_named_sobject_is_config_error(self, sf):
        _stub_send_request(sf, [FakeResponse({}, status_code=404)])
        with pytest.raises(AirbyteTracedException, match="'Missing' not found"):
            sf.describe("Missing")

    def test_other_error_is_system_error(self, sf):
        _stub_send_request(sf, [FakeResponse({}, status_code=500)])
        with pytest.raises(AirbyteTracedException, match="describe\\('global'\\) failed"):
            sf.describe()


class TestGenerateSchema:
    def test_properties_built_from_fields(self, sf):
        sf.describe = Mock(return_value=ACCOUNT_DESCRIBE)
        schema = sf.generate_schema("Account")
        assert schema["type"] == "object"
        assert schema["properties"]["Id"] == {"type": ["string", "null"]}
        assert schema["properties"]["AnnualRevenue"] == {"type": ["number", "null"]}
        # Describe response cached for get_custom_field_names().
        assert sf._sobject_describes["Account"] is ACCOUNT_DESCRIBE

    def test_unnamed_schema_not_cached(self, sf):
        sf.describe = Mock(return_value=ACCOUNT_DESCRIBE)
        sf.generate_schema()
        assert sf._sobject_describes == {}


class TestGetCustomFieldNames:
    def test_from_cache_no_extra_describe(self, sf):
        sf._sobject_describes["Account"] = ACCOUNT_DESCRIBE
        sf.describe = Mock()
        assert sf.get_custom_field_names("Account") == frozenset({"Custom__c"})
        sf.describe.assert_not_called()

    def test_fallback_fetches_on_demand(self, sf):
        sf.describe = Mock(return_value=ACCOUNT_DESCRIBE)
        assert sf.get_custom_field_names("Account") == frozenset({"Custom__c"})
        sf.describe.assert_called_once_with("Account")
        assert sf._sobject_describes["Account"] is ACCOUNT_DESCRIBE


class TestGenerateSchemas:
    def test_parallel_success(self, sf):
        sf.describe = Mock(return_value=ACCOUNT_DESCRIBE)
        schemas = sf.generate_schemas({"Account": {}, "Contact": {}})
        assert set(schemas) == {"Account", "Contact"}
        assert schemas["Account"]["properties"]["Id"] == {"type": ["string", "null"]}

    def test_request_error_becomes_traced_exception(self, sf):
        sf.generate_schema = Mock(side_effect=RequestException("timeout"))
        with pytest.raises(AirbyteTracedException, match="Schema could not be extracted"):
            sf.generate_schemas({"Account": {}})

    def test_chunked_across_parallel_task_size(self, sf, monkeypatch):
        # Force two sequential batches to cover the outer chunking loop.
        monkeypatch.setattr(sf, "parallel_tasks_size", 1)
        sf.describe = Mock(return_value=ACCOUNT_DESCRIBE)
        schemas = sf.generate_schemas({"Account": {}, "Contact": {}})
        assert set(schemas) == {"Account", "Contact"}


# ---------------------------------------------------------------------------
# Stream discovery
# ---------------------------------------------------------------------------


def _global_describe(names, queryable=True):
    return {"sobjects": [{"name": n, "queryable": queryable} for n in names]}


class TestStreamDiscovery:
    def test_filter_streams(self, sf):
        assert sf.filter_streams("Account") is True
        assert sf.filter_streams("AccountChangeEvent") is False
        assert sf.filter_streams("Vote") is False  # QUERY_RESTRICTED
        assert sf.filter_streams("ContentBody") is False  # QUERY_INCOMPATIBLE

    def test_blacklist_is_union_of_both_lists(self, sf):
        blacklist = sf.get_streams_black_list()
        assert "Vote" in blacklist and "ContentBody" in blacklist

    def test_default_selection_uses_crm_streams(self, sf):
        sf.describe = Mock(return_value=_global_describe(CRM_STREAMS + ["Unrelated"]))
        validated = sf.get_validated_streams()
        assert set(validated) == set(CRM_STREAMS)

    def test_non_queryable_skipped(self, sf):
        sf.describe = Mock(return_value=_global_describe(["Account"], queryable=False))
        assert sf.get_validated_streams() == {}

    def test_unsupported_streams_skipped(self, sf):
        sf.describe = Mock(return_value=_global_describe(["ActivityMetric", "Account"]))
        assert set(sf.get_validated_streams()) == {"Account"}

    def test_missing_requested_streams_logged(self, sf, caplog):
        sf.describe = Mock(return_value=_global_describe(["Account"]))
        with caplog.at_level("WARNING", logger="airbyte"):
            validated = sf.get_validated_streams()
        assert set(validated) == {"Account"}
        assert "not queryable in this org" in caplog.text

    def test_catalog_intersection_wins(self, sf):
        sf.describe = Mock(return_value=_global_describe(["Account", "Contact"]))
        catalog = ConfiguredAirbyteCatalog(
            streams=[
                ConfiguredAirbyteStream(
                    stream=AirbyteStream(name="Account", json_schema={}, supported_sync_modes=[SyncMode.full_refresh]),
                    sync_mode=SyncMode.full_refresh,
                    destination_sync_mode=DestinationSyncMode.overwrite,
                ),
                ConfiguredAirbyteStream(
                    stream=AirbyteStream(name="Ghost", json_schema={}, supported_sync_modes=[SyncMode.full_refresh]),
                    sync_mode=SyncMode.full_refresh,
                    destination_sync_mode=DestinationSyncMode.overwrite,
                ),
            ]
        )
        validated = sf.get_validated_streams(catalog=catalog)
        assert set(validated) == {"Account"}


# ---------------------------------------------------------------------------
# Field-type mapping
# ---------------------------------------------------------------------------


class TestPkAndReplicationKey:
    def test_cursor_priority(self):
        schema = {"properties": {"Id": {}, "CreatedDate": {}, "SystemModstamp": {}}}
        assert Salesforce.get_pk_and_replication_key(schema) == ("Id", "SystemModstamp")

    def test_fallback_chain(self):
        assert Salesforce.get_pk_and_replication_key({"properties": {"Id": {}, "LastModifiedDate": {}}}) == (
            "Id",
            "LastModifiedDate",
        )
        assert Salesforce.get_pk_and_replication_key({"properties": {"LoginTime": {}}}) == (None, "LoginTime")

    def test_no_cursor_no_pk(self):
        assert Salesforce.get_pk_and_replication_key({"properties": {"Name": {}}}) == (None, None)
        assert Salesforce.get_pk_and_replication_key({}) == (None, None)


class TestFieldToPropertySchema:
    @pytest.mark.parametrize(
        "sf_type,expected",
        [
            ("string", {"type": ["string", "null"]}),
            ("picklist", {"type": ["string", "null"]}),
            ("datetime", {"type": ["string", "null"], "format": "date-time"}),
            ("date", {"type": ["string", "null"], "format": "date"}),
            ("currency", {"type": ["number", "null"]}),
            ("int", {"type": ["integer", "null"]}),
            ("boolean", {"type": ["boolean", "null"]}),
            ("base64", {"type": ["string", "null"], "format": "base64"}),
            ("anyType", {"type": ["string", "null"]}),
            ("calculated", {"type": ["string", "null"]}),
        ],
    )
    def test_scalar_types(self, sf_type, expected):
        assert Salesforce.field_to_property_schema({"type": sf_type}) == expected

    def test_address_and_location_are_objects(self):
        address = Salesforce.field_to_property_schema({"type": "address"})
        assert address["type"] == ["object", "null"]
        assert "street" in address["properties"]
        location = Salesforce.field_to_property_schema({"type": "location"})
        assert set(location["properties"]) == {"longitude", "latitude"}

    def test_unknown_type_raises(self):
        with pytest.raises(TypeSalesforceException, match="Unsupported Salesforce field type"):
            Salesforce.field_to_property_schema({"type": "hologram"})
