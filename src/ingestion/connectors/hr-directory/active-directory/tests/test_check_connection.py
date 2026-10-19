"""A check fails with the reason an operator can act on, and never leaves a bind open."""

from __future__ import annotations

from collections.abc import Mapping

import pytest
from ldap3.core.exceptions import LDAPBindError, LDAPSocketOpenError
from source_active_directory import ldap_client
from source_active_directory.source import SourceActiveDirectory

CONFIG = {
    "insight_tenant_id": "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f",
    "insight_source_id": "active-directory-example",
    "ad_server_host": "dc1.example.test",
    "ad_bind_dn": "CN=reader,DC=example,DC=test",
    "ad_bind_password": "synthetic",
    "ad_search_base": "DC=example,DC=test",
}


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
