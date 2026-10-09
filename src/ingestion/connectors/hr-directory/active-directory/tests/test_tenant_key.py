"""Records are stamped with the canonical tenant and keyed by the tenant key."""

from __future__ import annotations

from source_active_directory.source import SourceActiveDirectory

TENANT = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
CONFIG = {
    "insight_tenant_id": TENANT,
    "insight_source_id": "active-directory-example",
    "ad_host": "dc1.example.test",
    "ad_bind_dn": "CN=reader,DC=example,DC=test",
    "ad_bind_password": "synthetic",
    "ad_base_dn": "DC=example,DC=test",
}
GUID = "3f2504e0-4f89-11d3-9a0c-0305e82c3301"


def _stream(config):
    (stream,) = SourceActiveDirectory().streams(config)
    return stream


def test_a_fresh_install_keys_by_the_canonical_tenant() -> None:
    assert _stream(CONFIG)._make_unique_key(GUID) == f"{TENANT}-active-directory-example-{GUID}"


def test_a_warm_install_keys_by_its_legacy_tenant_key() -> None:
    stream = _stream({**CONFIG, "insight_tenant_key": "example-legacy"})
    assert stream._tenant_id == TENANT
    assert stream._make_unique_key(GUID) == f"example-legacy-active-directory-example-{GUID}"
