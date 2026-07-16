# Downstream-verification e2e

Proves the **R1 rule** as code (NGINX_BFF §6 / §D): every downstream service
verifies the gateway JWT itself — mandatory, fail-closed, no production disable
knob. Tenant identity comes only from the signed JWT; `X-Tenant-ID` is a
selector among the JWT's signed `tenants[]` (G2).

## What it stands up

The full chain with the **real** downstream services behind the OpenResty
gateway:

```
fakeidp ─▶ authenticator ─▶ gateway ─▶ {analytics (Rust), identity (.NET)}
                              │                     ▲
                    cookie ─▶ JWT (ES256)           │ verifies the JWT via JWKS
```

- `authenticator` resolves the login user via the **identity-stub** (a test seam
  so login works without seeding real identity).
- `analytics` and `identity` are the real services; each verifies the gateway
  JWT against the authenticator's JWKS and then maps the claims (analytics via
  the shared `authverify` layer; identity via its JwtBearer + `GatewayTenantContext`).
- `MariaDB` backs analytics (migrations run at boot) and identity.

## The five scenarios (`test_downstream.py`)

1. Browser-less login → cookie → `GET /api/analytics/...` → **200**.
2. Same request **directly to analytics' port** without a JWT → **401**
   (the R1 proof; identity is checked too).
3. Multi-tenant user (`carol`): correct `X-Tenant-ID` → 200; selector outside
   the signed set → 403; missing selector with >1 tenants → 400.
4. Step-06 **service token** → analytics accepts it; `roles: ["service"]`.
5. A request reaching analytics without a valid gateway JWT (models a gateway
   route shipped without `auth_request`, or a forged/browser token) → **401**.

## Running

Requires `docker`, `openssl`, `pytest`, and — for scenario 4 — `PyJWT` +
`cryptography`:

```
pip install pytest pyjwt cryptography
src/backend/services/gateway/tests/downstream-verify/run-e2e.sh
```

The suite builds the analytics / identity / authenticator / gateway / fakeidp
images, so the first run is slow; it is intended for CI and local verification,
never a production image.
