"""An AD user becomes one Bronze row in the same shape the Entra connector writes."""

from __future__ import annotations

import uuid
from collections.abc import Iterator

import pytest
from airbyte_cdk.models import SyncMode
from ldap3.core.exceptions import LDAPSocketOpenError
from source_active_directory import ldap_client
from source_active_directory.streams.users import ActiveDirectoryUsersStream

TENANT = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
SOURCE = "active-directory-example"
GUID = "3f2504e0-4f89-11d3-9a0c-0305e82c3301"
CONFIG = {"ad_search_base": "DC=example,DC=test", "ad_page_size": "50"}


def _entry(**attributes: object) -> dict[str, object]:
    return {
        "type": "searchResEntry",
        "dn": "CN=Alice Alpha,OU=People,DC=example,DC=test",
        "raw_attributes": {"objectGUID": [uuid.UUID(GUID).bytes_le]},
        "attributes": {
            "sAMAccountName": "alice",
            "userPrincipalName": "alice@example.com",
            "mail": ["alice@example.com"],
            "proxyAddresses": ["SMTP:alice@example.com", "", "smtp:a.alpha@example.com"],
            "displayName": "Alice Alpha",
            "sn": "Alpha",
            "userAccountControl": 512,
            "whenCreated": "20260105120000.0Z",
            "givenName": "",
            **attributes,
        },
    }


def _stream() -> ActiveDirectoryUsersStream:
    return ActiveDirectoryUsersStream(config=CONFIG, tenant_id=TENANT, source_id=SOURCE)


def test_a_user_maps_to_the_shared_directory_shape() -> None:
    record = _stream()._to_record(_entry())

    assert record["id"] == GUID
    assert record["unique_key"] == f"{TENANT}-{SOURCE}-{GUID}"
    assert record["tenant_id"] == TENANT
    assert record["mail"] == "alice@example.com"
    assert record["proxyAddresses"] == ["SMTP:alice@example.com", "smtp:a.alpha@example.com"]
    assert record["surname"] == "Alpha"
    assert record["givenName"] is None, "an empty attribute reads as absent"
    assert record["whenCreated"] == "2026-01-05T12:00:00Z"
    assert record["distinguishedName"] == "CN=Alice Alpha,OU=People,DC=example,DC=test"


@pytest.mark.parametrize(
    ("uac", "enabled", "status"), [(512, True, "Active"), (514, False, "Terminated"), (None, None, None)]
)
def test_a_disabled_account_reads_as_terminated(uac: int | None, enabled: bool | None, status: str | None) -> None:
    record = _stream()._to_record(_entry(userAccountControl=uac))

    assert (record["accountEnabled"], record["status"]) == (enabled, status), f"uac {uac!r}"


def test_a_guid_formatted_by_the_library_is_used_when_the_raw_one_is_missing() -> None:
    entry = {**_entry(objectGUID=f"{{{GUID}}}"), "raw_attributes": {"objectGUID": []}}

    assert _stream()._to_record(entry)["id"] == GUID


@pytest.mark.parametrize(
    ("value", "expected"), [("x", ["x"]), (["x", None, ""], ["x"]), ([""], None), ("", None), (None, None)]
)
def test_a_multi_valued_attribute_drops_empty_values(
    value: str | list[str | None] | None, expected: list[str] | None
) -> None:
    assert ActiveDirectoryUsersStream._multi({"k": value}, "k") == expected, f"should read {value!r}"


def test_an_entry_without_a_guid_is_skipped() -> None:
    entry = {**_entry(), "raw_attributes": {}}

    assert _stream()._to_record(entry) is None


class FakeConnection:
    def __init__(self, entries: list[dict[str, object]] | None = None, error: Exception | None = None) -> None:
        self.entries = entries or []
        self.error = error
        self.searched: dict[str, object] = {}
        self.unbound = False
        self.extend = self
        self.standard = self

    def paged_search(self, **kwargs: object) -> Iterator[dict[str, object]]:
        self.searched = kwargs
        if self.error:
            raise self.error
        return iter(self.entries)

    def unbind(self) -> None:
        self.unbound = True


def test_a_read_emits_users_and_skips_referrals(monkeypatch: pytest.MonkeyPatch) -> None:
    conn = FakeConnection([_entry(), {"type": "searchResRef"}, {**_entry(), "raw_attributes": {}}])
    monkeypatch.setattr(ldap_client, "connect", lambda _: conn)

    records = list(_stream().read_records(SyncMode.full_refresh))

    assert [r["id"] for r in records] == [GUID]
    assert conn.searched["paged_size"] == 50
    assert conn.searched["attributes"] == ldap_client.USER_ATTRIBUTES
    assert conn.unbound


def test_a_failed_search_propagates_and_unbinds(monkeypatch: pytest.MonkeyPatch) -> None:
    conn = FakeConnection(error=LDAPSocketOpenError("connection reset"))
    monkeypatch.setattr(ldap_client, "connect", lambda _: conn)

    with pytest.raises(LDAPSocketOpenError):
        list(_stream().read_records(SyncMode.full_refresh))
    assert conn.unbound


def test_the_schema_requires_the_key_and_the_guid() -> None:
    schema = _stream().get_json_schema()

    assert schema["required"] == ["unique_key", "id"]
    assert "tenant_id" in schema["properties"]
