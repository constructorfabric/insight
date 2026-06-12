# Insight API Gateway

HTTP entry point for all Insight backend services. Built on cyberfabric-core ModKit with OIDC/JWT authentication.

## What it does

- Routes HTTP requests to backend service gears
- Validates JWT bearer tokens against an OIDC provider (Okta, Keycloak, Auth0, etc.)
- Enforces RBAC and tenant isolation via authz-resolver
- Provides OpenAPI documentation, CORS, rate limiting, request tracing
- Returns RFC 9457 Problem Details for all errors

## Quick start (local dev, no auth)

```bash
cd src/backend
cargo run --bin insight-api-gateway -- run -c services/api-gateway/config/no-auth.yaml
```

Open `http://localhost:8080/api/v1/docs` for OpenAPI UI.

## Quick start (with OIDC)

```bash
export OIDC_ISSUER_URL=https://dev-12345.okta.com/oauth2/default
export OIDC_AUDIENCE=api://insight

cd src/backend
cargo run --bin insight-api-gateway -- run -c services/api-gateway/config/insight.yaml
```

Test with a valid Okta token:

```bash
curl -H "Authorization: Bearer <your-jwt-token>" http://localhost:8080/api/v1/docs
```

## Configuration

Configuration is loaded in layers (highest priority last):

1. **Defaults** — hardcoded in code
2. **YAML file** — `-c config/insight.yaml`
3. **Environment variables** — `APP__gears__<gear>__config__<key>`
4. **CLI flags** — `--verbose`, `--print-config`

### Key environment variables

| Variable | Description | Example |
|----------|-------------|---------|
| `OIDC_ISSUER_URL` | OIDC issuer URL | `https://dev-12345.okta.com/oauth2/default` |
| `OIDC_AUDIENCE` | Expected JWT audience | `api://insight` |
| `APP__gears__api-gateway__config__bind_addr` | Listen address | `0.0.0.0:8080` |
| `APP__gears__api-gateway__config__auth_disabled` | Disable auth | `true` |

### Okta setup

1. Create an Okta application (Web or SPA)
2. Note the **Issuer URI** (e.g., `https://dev-12345.okta.com/oauth2/default`)
3. Note the **Audience** (e.g., `api://insight`)
4. Set `OIDC_ISSUER_URL` and `OIDC_AUDIENCE` environment variables
5. The plugin auto-discovers JWKS keys from `{issuer}/v1/keys`

### Custom claims

The plugin extracts these claims from the JWT:

| Claim | Maps to | Default behavior |
|-------|---------|-----------------|
| `sub` | `subject_id` (as UUID v5 hash) | Required |
| `scp` or `scope` | `token_scopes` | Empty if neither present (authz layer decides) |
| `{tenant_claim}` (configurable) | `subject_tenant_id` | Warning logged if missing. Rejected if `require_tenant_claim: true`, nil UUID if `false` (default). |

To use a different claim for tenant ID, set `tenant_claim` in config. Set `require_tenant_claim: true` to reject tokens without a valid tenant UUID.

## Deploy to Kubernetes

### Helm install

```bash
helm install insight-gw ./helm \
  --set oidc.issuerUrl=https://dev-12345.okta.com/oauth2/default \
  --set oidc.audience=api://insight \
  --set ingress.host=insight.example.com
```

### Helm values

See `helm/values.yaml` for all configurable values. Key ones:

```yaml
oidc:
  issuerUrl: "https://dev-12345.okta.com/oauth2/default"
  audience: "api://insight"

authDisabled: false  # Set true for dev without IdP

ingress:
  enabled: true
  host: insight.example.com
  tls:
    enabled: true
    secretName: insight-tls
```

### Local dev without auth

```bash
helm install insight-gw ./helm --set authDisabled=true
```

## Architecture

```text
Client → Ingress → API Gateway → [Auth Middleware] → Service Gears
                                       │
                                       ├── OIDC Plugin (JWT validation via JWKS)
                                       ├── AuthZ Resolver (RBAC + org scoping)
                                       └── Tenant Resolver (workspace isolation)
```

The gateway is a gears-rust Toolkit server binary that links:

| Gear | Purpose |
|------|---------|
| `api-gateway` | Axum HTTP server, routing, OpenAPI, CORS, rate limiting |
| `oidc-authn-plugin` | JWT validation against OIDC provider |
| `authn-resolver` | Authentication gateway (delegates to OIDC plugin) |
| `authz-resolver` | Authorization (static plugin now, org-tree plugin later) |
| `tenant-resolver` | Multi-tenant workspace isolation |
| `grpc-hub` | Internal gRPC communication |
| `gear-orchestrator` | Gear lifecycle management |
| `types-registry` | GTS type/plugin discovery |

## Public endpoints (no auth required)

| Endpoint | Response | Purpose |
|----------|----------|---------|
| `GET /health` | `{"status":"healthy","timestamp":"..."}` | K8s liveness + readiness probe |
| `GET /healthz` | `ok` | Simple liveness check |
| `GET /auth/config` | OIDC configuration JSON (see below) | Frontend reads this to initiate OIDC login |

### `GET /auth/config`

Returns the OIDC provider details the frontend needs to start the Authorization Code flow with PKCE. No token required.

```json
{
  "issuer_url": "https://dev-12345.okta.com/oauth2/default",
  "client_id": "0oa1b2c3d4e5f6g7h8i9",
  "redirect_uri": "http://localhost:3000/callback",
  "scopes": ["openid", "profile", "email"],
  "response_type": "code"
}
```

Frontend flow:
1. Fetch `/auth/config` on startup
2. Construct Okta authorize URL from `issuer_url` + `client_id` + `redirect_uri` + `scopes`
3. Redirect user to Okta login
4. Okta redirects back with auth code
5. Frontend exchanges code for tokens
6. Frontend sends `Authorization: Bearer <access_token>` on every API call
7. If API returns 401 → redirect to Okta again (token expired)

Configured via `OIDC_CLIENT_ID`, `OIDC_REDIRECT_URI` env vars (or Helm values `oidc.clientId`, `oidc.redirectUri`).

## Building

```bash
cd src/backend
cargo build --release --bin insight-api-gateway
```

Docker (build context is `cf/` parent directory containing both repos):

```bash
cd /path/to/cf
docker build -f insight/src/backend/services/api-gateway/Dockerfile -t insight-api-gateway:dev .
```

See `Dockerfile` for the full multi-stage build (protoc, non-root user, bookworm-slim runtime).
