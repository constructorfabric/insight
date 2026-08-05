# Auth and tenancy

## How the rig authenticates

analytics runs **auth-enabled** in this rig: `lib/analytics.py` writes a
per-spawn gears config with `auth_disabled: false` and wires the
`oidc-authn-plugin` to a self-signed TLS discovery front that `lib/gateway_jwt.py`
serves. Every request carries a signed ES256 gateway JWT, and the request tenant
is the JWT's `tenant_id` claim.

There is **no tenant header**. `X-Insight-Tenant-Id` was removed from the
product; the bearer is the only tenant authority. (The `OTHER_TENANT` comment in
`api/endpoint_helpers.py` still describes a tenant-override *header* — that text
is stale; the mechanism is the bearer, as `other_tenant_headers` in
`api/conftest.py` correctly describes.)

`/health` is a host-gear route and is public — it is the only unauthenticated
path in the suite.

## The client fixtures

```python
@pytest.fixture
def api(analytics: AnalyticsProcess):
    """Recording httpx client (the coverage chokepoint), one per test."""
    with analytics.client() as c:
        yield c
```

`analytics.client()` sets `Authorization: Bearer <jwt for TEST_TENANT_ID>` and
installs the `api_coverage.record_response` hook. **Every request in the suite
must go through it** — a hand-built `httpx.Client` is invisible to the gate.

For a different principal, mint a bearer and override the header per request:

```python
@pytest.fixture
def other_tenant_headers(analytics) -> dict:
    return {"Authorization": f"Bearer {analytics.bearer(OTHER_TENANT)}"}


def test_update_403_cross_tenant(api, admin_threshold_row: dict, other_tenant_headers: dict) -> None:
    r = api.put(f"/v1/admin/metric-thresholds/{admin_threshold_row['id']}", json={...},
                headers=other_tenant_headers)
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"
```

Overriding the header on the call keeps the recording hook (and therefore the
ledger) intact — that is why this is a headers dict rather than a second client.

`analytics.bearer(tenant_id)` is the only bearer factory. Do not construct or
sign a JWT in a test; if a new principal shape is needed (missing claim, expired
token, wrong audience), add a minting helper to `lib/gateway_jwt.py` so the
signing rules stay in one place.

## Which codes tenancy can produce

| Code | Reachable how |
| --- | --- |
| 403 | A write against a row owned by another tenant (`not_tenant_admin`), or a create shadowed by a locked broader scope (`threshold_locked`) |
| 404 | A *read* of another tenant's row — cross-tenant reads answer by opacity, not by 403. Assert 404, not 403, for reads |
| 401 | No bearer, or one the plugin rejects |

The 403-vs-404 split is a contract, not an accident: writes reveal ownership,
reads do not. Getting it backwards in a test hides a real regression.

## The 401 gap

`api/` has **no 401 case today**, and `lib/api_coverage.py` subtracts 401 from
every analytics operation via `UNIVERSAL_BOILERPLATE` with the reason "auth
disabled at the gateway" — which no longer matches the rig (`auth_disabled:
false` since the gateway-JWT change). The `identity/` suite, which runs the same
plugin, treats 401 as required and proves it per route.

So: when adding auth coverage here, expect to **drop 401 from
`UNIVERSAL_BOILERPLATE`** and add a no-bearer case per route group, modelled on
`identity/conftest.py`'s `anon_api`:

```python
@pytest.fixture
def anon_api(analytics):
    """Client with no Authorization header — the 401 gate."""
    with analytics.client() as c:
        c.headers.pop("Authorization", None)
        yield c
```

Verify against the running rig before changing the gate: if a route answers
something other than 401 unauthenticated, that is a finding to file, not an
exclusion to keep.

## See Also

- [api-coverage-gate.md](./api-coverage-gate.md) — where 401/403 exclusions live
- [api-fixtures.md](./api-fixtures.md) — the rest of the fixture inventory
- `lib/gateway_jwt.py` — bearer minting and the JWKS/discovery front
