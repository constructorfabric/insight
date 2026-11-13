"""LDAP values arrive in several shapes; each decodes to one canonical form."""

from __future__ import annotations

import ssl
import uuid
from datetime import datetime, timedelta, timezone

import pytest
from pydantic import ValidationError
from source_active_directory import ldap_client
from source_active_directory.config import ActiveDirectoryConfig

GUID = "3f2504e0-4f89-11d3-9a0c-0305e82c3301"
BASE = ActiveDirectoryConfig(
    insight_tenant_id="0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f",
    insight_source_id="active-directory-example",
    ad_server_host="dc1.example.test",
    ad_bind_dn="CN=reader,DC=example,DC=test",
    ad_bind_password="synthetic",
    ad_search_base="DC=example,DC=test",
)


def _config(**overrides: object) -> ActiveDirectoryConfig:
    return ActiveDirectoryConfig.model_validate({**BASE.model_dump(mode="json", exclude_unset=True), **overrides})


@pytest.mark.parametrize(
    ("value", "expected"),
    [(None, True), (True, True), (False, False), ("false", False), (" No ", False), ("1", True), (0, False)],
)
def test_a_tls_switch_reads_strings_as_the_operator_meant_them(value: bool | str | int | None, expected: bool) -> None:
    assert _config(ad_use_ssl=value).ad_use_ssl is expected, f"should read {value!r} as {expected}"


def test_an_unreadable_tls_switch_is_refused() -> None:
    with pytest.raises(ValidationError) as caught:
        _config(ad_use_ssl="maybe")

    assert [e["loc"] for e in caught.value.errors()] == [("ad_use_ssl",)]


@pytest.mark.parametrize(
    ("overrides", "expected"),
    [({}, 636), ({"ad_use_ssl": False}, 389), ({"ad_port": "3269"}, 3269), ({"ad_port": ""}, 636)],
)
def test_the_port_follows_tls_unless_one_is_named(overrides: dict[str, object], expected: int) -> None:
    assert _config(**overrides).port == expected


def test_a_tls_server_validates_its_certificate() -> None:
    server = ldap_client.build_server(BASE)

    assert server.ssl is True
    assert server.port == 636
    assert server.tls.validate == ssl.CERT_REQUIRED


def test_a_plain_server_carries_no_tls() -> None:
    server = ldap_client.build_server(_config(ad_use_ssl="false"))

    assert server.ssl is False
    assert server.port == 389


def test_a_connection_binds_with_the_configured_account(monkeypatch: pytest.MonkeyPatch) -> None:
    opened: dict[str, object] = {}

    def fake_connection(server: object, **kwargs: object) -> str:
        opened.update(kwargs, server=server)
        return "bound"

    monkeypatch.setattr(ldap_client, "Connection", fake_connection)
    assert ldap_client.connect(BASE) == "bound"
    assert opened["user"] == BASE.ad_bind_dn
    assert opened["password"] == "synthetic"
    assert opened["auto_bind"] is True
    assert opened["raise_exceptions"] is True


@pytest.mark.parametrize(
    ("raw", "expected"),
    [(None, None), (uuid.UUID(GUID).bytes_le, GUID), (f"{{{GUID.upper()}}}", GUID), (12345, "12345")],
    ids=["absent", "binary", "braced string", "anything else"],
)
def test_an_object_guid_reads_as_the_guid_directory_tools_show(
    raw: bytes | str | int | None, expected: str | None
) -> None:
    assert ldap_client.guid_to_str(raw) == expected


@pytest.mark.parametrize(
    ("value", "expected"),
    [
        (None, None),
        (datetime(2026, 1, 5, 12, 0, tzinfo=timezone(timedelta(hours=3))), "2026-01-05T09:00:00Z"),
        ("20260105120000.0Z", "2026-01-05T12:00:00Z"),
        (b"20260105120000.0Z", "2026-01-05T12:00:00Z"),
        ("20261399000000.0Z", "20261399000000.0Z"),
        ("not a time", "not a time"),
    ],
    ids=["absent", "parsed", "generalized", "bytes", "impossible date", "free text"],
)
def test_a_generalized_time_reads_as_utc_iso(value: datetime | str | bytes | None, expected: str | None) -> None:
    assert ldap_client.generalized_time_to_iso(value) == expected


@pytest.mark.parametrize(
    ("uac", "expected"), [(None, None), ("", None), (512, True), ("514", False), ("garbage", None)]
)
def test_the_disable_bit_decides_whether_an_account_is_enabled(uac: str | int | None, expected: bool | None) -> None:
    assert ldap_client.account_enabled(uac) is expected, f"should read {uac!r} as {expected}"
