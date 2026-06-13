# OIDC AuthN Plugin

A cyberfabric `authn-resolver` plugin that validates JWT bearer tokens against an OIDC provider (Okta, Keycloak, Auth0, or any OIDC-compliant IdP).

## How it works

1. On startup, fetches JWKS keys from the provider's keys endpoint
2. On each request, validates the JWT signature against cached keys
3. Validates standard claims (issuer, audience, expiry, not-before)
4. Extracts `sub`, scopes, and tenant ID from claims
5. Builds a `SecurityContext` and returns it to the authn-resolver gateway
6. Background task refreshes JWKS keys periodically

## Configuration

```yaml
gears:
  oidc-authn-plugin:
    config:
      vendor: "hyperspot"
      priority: 50
      issuer_url: "https://dev-12345.okta.com/oauth2/default"
      audience: "api://insight"
      # jwks_url: ""                       # Override if JWKS URL differs from {issuer}/v1/keys
      jwks_refresh_interval_seconds: 300   # 5 minutes
      tenant_claim: "tenant_id"            # JWT claim containing tenant UUID
      subject_type: "user"
      leeway_seconds: 60                   # Clock skew tolerance
```

### Configuration reference

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `vendor` | No | `hyperspot` | Vendor name for GTS plugin registration |
| `priority` | No | `50` | Plugin priority (lower = higher) |
| `issuer_url` | **Yes** | — | OIDC issuer URL. Used for `iss` claim validation and JWKS URL derivation |
| `audience` | No | — | Expected `aud` claim. If empty, audience is not validated |
| `jwks_url` | No | `{issuer_url}/v1/keys` | JWKS endpoint override |
| `jwks_refresh_interval_seconds` | No | `300` | How often to refresh JWKS keys |
| `tenant_claim` | No | `tenant_id` | JWT claim name containing the tenant UUID |
| `subject_type` | No | `user` | Subject type in `SecurityContext` |
| `leeway_seconds` | No | `60` | Clock skew tolerance for exp/nbf validation |

### Environment variable overrides

```bash
export APP__gears__oidc-authn-plugin__config__issuer_url=https://dev-12345.okta.com/oauth2/default
export APP__gears__oidc-authn-plugin__config__audience=api://insight
```

## Claim mapping

| JWT claim | SecurityContext field | Notes |
|-----------|---------------------|-------|
| `sub` | `subject_id` | Hashed to UUID v5 (OIDC subs are often not UUIDs) |
| `scp` (array) or `scope` (string) | `token_scopes` | Okta uses `scp`, standard OIDC uses `scope`. Empty if neither present (authz layer decides) |
| `{tenant_claim}` | `subject_tenant_id` | Configurable claim name. Parsed as UUID. Falls back to nil UUID |

## Provider setup

### Okta

1. Create an API application in Okta Admin Console
2. Note the **Issuer URI**: `https://dev-XXXXX.okta.com/oauth2/default`
3. Set the **Audience** in your authorization server settings
4. JWKS URL is auto-derived: `{issuer}/v1/keys`
5. For tenant isolation, add a custom claim `tenant_id` to the authorization server

### Keycloak

1. Create a realm and client
2. Issuer URI: `https://keycloak.example.com/realms/{realm}`
3. JWKS URL: `{issuer}/protocol/openid-connect/certs` — set `jwks_url` explicitly
4. Audience: the client ID

### Auth0

1. Create an API in Auth0 Dashboard
2. Issuer URI: `https://{tenant}.auth0.com/`
3. JWKS URL: `{issuer}/.well-known/jwks.json` — set `jwks_url` explicitly
4. Audience: the API identifier

## Limitations

- **Client credentials flow** is not supported. Use `static-authn-plugin` for S2S authentication.
- **Token refresh** is not handled by this plugin — clients must refresh tokens with the IdP directly.
- **Custom claim extraction** is limited to the configured `tenant_claim`. Additional claims require code changes.

## Architecture

This plugin implements the `AuthNResolverPluginClient` trait from `authn-resolver-sdk`. It is discovered by the `authn-resolver` gateway module via the GTS types-registry at runtime.

```text
Request with Bearer token
    → API Gateway extracts token
    → authn-resolver delegates to this plugin
    → Plugin validates JWT (signature + claims)
    → Returns SecurityContext
    → API Gateway injects SecurityContext into request
```

Uses `modkit-auth` crate for:
- `JwksKeyProvider` — JWKS key fetching and caching
- `validate_claims` — standard JWT claim validation
