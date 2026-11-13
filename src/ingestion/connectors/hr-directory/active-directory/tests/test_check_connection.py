"""A check fails with the reason an operator can act on, and never leaves a bind open."""

from __future__ import annotations

from collections.abc import Mapping

import pytest
from ldap3.core.exceptions import LDAPBindError, LDAPSocketOpenError
from source_active_directory import ldap_client
from source_active_directory.config import ActiveDirectoryConfig
from source_active_directory.source import SourceActiveDirectory

CONFIG = ActiveDirectoryConfig(
    insight_tenant_id="0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f",
    insight_source_id="active-directory-example",
    ad_server_host="dc1.example.test",
    ad_bind_dn="CN=reader,DC=example,DC=test",
    ad_bind_password="synthetic",
    ad_search_base="DC=example,DC=test",
).model_dump(mode="json", exclude_unset=True)


class FakeConnection:
    def __init__(
        self, ok: bool = True, result: dict[str, object] | None = None, error: Exception | None = None
    ) -> None:
        self.ok = ok
        self.result = result
        self.error = error
        self.unbound = False

    def search(self, **_: object) -> bool:
        if self.error:
            raise self.error
        return self.ok

    def unbind(self) -> None:
        self.unbound = True


def _check(
    monkeypatch: pytest.MonkeyPatch, conn: FakeConnection | Exception, config: Mapping[str, object] = CONFIG
) -> tuple[bool, str | None]:
    def connect(_: Mapping[str, object]) -> FakeConnection:
        if isinstance(conn, Exception):
            raise conn
        return conn

    monkeypatch.setattr(ldap_client, "connect", connect)
    return SourceActiveDirectory().check_connection(None, config)


@pytest.mark.parametrize("field", ["insight_source_id", "insight_tenant_id"])
def test_a_missing_identity_field_is_refused_before_any_bind(monkeypatch: pytest.MonkeyPatch, field: str) -> None:
    ok, reason = _check(monkeypatch, AssertionError("must not bind"), {**CONFIG, field: "  "})

    assert ok is False
    assert field in str(reason)


@pytest.mark.parametrize(
    ("overrides", "field", "error_type"),
    [
        ({"ad_page_size": "secret-page-size"}, "ad_page_size", "int_parsing"),
        ({"ad_bind_password": None}, "ad_bind_password", "string_type"),
        ({"ad_use_ssl": "secret-switch"}, "ad_use_ssl", "value_error"),
    ],
    ids=["wrong type", "null", "unreadable switch"],
)
def test_a_malformed_config_is_refused_without_quoting_it(
    monkeypatch: pytest.MonkeyPatch, overrides: dict[str, object], field: str, error_type: str
) -> None:
    ok, reason = _check(monkeypatch, AssertionError("must not bind"), {**CONFIG, **overrides})

    assert ok is False
    assert f"{field}: {error_type}" in str(reason)
    assert "secret" not in str(reason), "the input value must not reach the message"


def test_a_config_without_a_password_is_refused_before_any_bind(monkeypatch: pytest.MonkeyPatch) -> None:
    config = {k: v for k, v in CONFIG.items() if k != "ad_bind_password"}

    ok, reason = _check(monkeypatch, AssertionError("must not bind"), config)

    assert ok is False
    assert "ad_bind_password: missing" in str(reason)


def test_a_read_with_a_malformed_config_fails_without_quoting_it() -> None:
    with pytest.raises(ValueError, match="ad_page_size: int_parsing") as caught:
        SourceActiveDirectory().streams({**CONFIG, "ad_page_size": "secret-page-size"})

    assert "secret" not in str(caught.value)
    assert caught.value.__cause__ is None and caught.value.__suppress_context__


def test_a_readable_search_base_passes(monkeypatch: pytest.MonkeyPatch) -> None:
    conn = FakeConnection()

    assert _check(monkeypatch, conn) == (True, None)
    assert conn.unbound


def test_a_failed_bind_names_the_server(monkeypatch: pytest.MonkeyPatch) -> None:
    ok, reason = _check(monkeypatch, LDAPBindError("invalid credentials"))

    assert ok is False
    assert "dc1.example.test" in str(reason)


def test_an_unreadable_search_base_reports_the_directory_answer(monkeypatch: pytest.MonkeyPatch) -> None:
    conn = FakeConnection(ok=False, result={"result": 32, "description": "noSuchObject", "message": "base gone"})

    ok, reason = _check(monkeypatch, conn)

    assert ok is False
    assert "noSuchObject" in str(reason)
    assert conn.unbound


def test_a_search_that_raises_is_a_failed_check(monkeypatch: pytest.MonkeyPatch) -> None:
    conn = FakeConnection(error=LDAPSocketOpenError("connection reset"))

    ok, reason = _check(monkeypatch, conn)

    assert ok is False
    assert "connection reset" in str(reason)
    assert conn.unbound


def test_the_spec_is_the_packaged_one() -> None:
    spec = SourceActiveDirectory().spec(None)

    assert "ad_server_host" in spec.connectionSpecification["properties"]
