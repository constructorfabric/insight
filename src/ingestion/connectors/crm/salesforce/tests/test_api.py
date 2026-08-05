"""Tests for source_salesforce.api: token provider, authenticator, client."""

from __future__ import annotations

import time
from unittest.mock import Mock

import pytest
from airbyte_cdk.utils import AirbyteTracedException
from requests.exceptions import RequestException
from source_salesforce.api import Salesforce, SalesforceAuthenticator, SalesforceTokenProvider
from source_salesforce.constants import CRM_STREAMS, TOKEN_REFRESH_INTERVAL_SECONDS
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
    def test_first_use_authenticates(self, sf):
        sf.login = Mock(side_effect=lambda: setattr(sf, "access_token", "tok"))
        assert sf._token_provider.get_token() == "tok"
        sf.login.assert_called_once()

    def test_first_login_failure_surfaces(self, sf):
        sf.login = Mock(side_effect=RequestException("no creds"))
        with pytest.raises(RequestException):
            sf._token_provider.get_token()

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
# describe + field discovery
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
        sf.access_token = "tok"
        calls = _stub_send_request(sf, [FakeResponse({"sobjects": []})])
        assert sf.describe() == {"sobjects": []}
        assert calls[0]["url"] == (f"{INSTANCE_URL}/services/data/{sf.version}/sobjects")

    def test_describe_authenticates_when_no_token_yet(self, sf):
        """Describe is often a sync's first authenticated call."""
        sf.login = Mock(side_effect=lambda: setattr(sf, "access_token", "tok"))
        calls = _stub_send_request(sf, [FakeResponse(ACCOUNT_DESCRIBE)])
        sf.describe("Account")
        sf.login.assert_called_once()
        assert calls[0]["headers"]["Authorization"] == "Bearer tok"

    def test_sobject_describe_url_and_auth_header(self, sf):
        sf.access_token = "tok"
        calls = _stub_send_request(sf, [FakeResponse(ACCOUNT_DESCRIBE)])
        assert sf.describe("Account") == ACCOUNT_DESCRIBE
        assert calls[0]["url"].endswith("/sobjects/Account/describe")
        assert calls[0]["headers"]["Authorization"] == "Bearer tok"

    def test_404_for_named_sobject_is_config_error(self, sf):
        sf.access_token = "tok"
        _stub_send_request(sf, [FakeResponse({}, status_code=404)])
        with pytest.raises(AirbyteTracedException, match="'Missing' not found"):
            sf.describe("Missing")

    def test_other_error_is_system_error(self, sf):
        sf.access_token = "tok"
        _stub_send_request(sf, [FakeResponse({}, status_code=500)])
        with pytest.raises(AirbyteTracedException, match="describe\\('global'\\) failed"):
            sf.describe()


class TestFieldNames:
    def test_returns_standard_and_custom_fields(self, sf):
        sf.describe = Mock(return_value=ACCOUNT_DESCRIBE)
        names = sf.field_names("Account")
        assert "Id" in names and "Custom__c" in names

    def test_cached_describe_reused(self, sf):
        sf._sobject_describes["Account"] = ACCOUNT_DESCRIBE
        sf.describe = Mock()
        assert "Custom__c" in sf.field_names("Account")
        sf.describe.assert_not_called()

    def test_describe_fetched_once_per_sobject(self, sf):
        sf.describe = Mock(return_value=ACCOUNT_DESCRIBE)
        sf.field_names("Account")
        sf.field_names("Account")
        assert sf.describe.call_count == 1
        assert sf._sobject_describes["Account"] is ACCOUNT_DESCRIBE

    def test_absent_sobject_yields_no_fields(self, sf):
        sf.describe = Mock(return_value=None)
        assert sf.field_names("Ghost") == ()
        assert sf.is_queryable("Ghost") is False

    def test_non_queryable_sobject_reported_unavailable(self, sf):
        sf.describe = Mock(return_value={"fields": [{"name": "Id"}], "queryable": False})
        assert sf.is_queryable("Account") is False


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

    def test_syncable_streams_are_the_curated_set(self, sf):
        sf.describe = Mock(side_effect=AssertionError("describe must not be called"))
        assert sf.syncable_streams() == list(CRM_STREAMS)

    def test_syncable_streams_exclude_streams_needing_an_object_id(self, sf, monkeypatch):
        monkeypatch.setattr("source_salesforce.api.CRM_STREAMS", ["Account", "ActivityMetric"])
        assert sf.syncable_streams() == ["Account"]

    def test_unavailable_streams_reports_what_the_org_lacks(self, sf):
        sf.describe = Mock(return_value=_global_describe(["Account", "Contact"]))
        unavailable = sf.unavailable_streams()
        assert "Account" not in unavailable
        assert "Opportunity" in unavailable

    def test_unavailable_streams_counts_non_queryable_as_missing(self, sf):
        sf.describe = Mock(return_value=_global_describe(CRM_STREAMS, queryable=False))
        assert set(sf.unavailable_streams()) == set(CRM_STREAMS)


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
