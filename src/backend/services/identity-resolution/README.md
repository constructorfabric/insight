# identity-resolution

The Insight identity service (Rust; epic #1602 — port of the retired .NET `identity` service).
Built on the gears-rust framework — same host pattern as `services/analytics`
(the `api-gateway` system gear is the REST host; auth ENABLED — the
`oidc-authn-plugin` verifies the gateway JWT and maps its claims into the
`SecurityContext`).

Current state: boots as a gears host, connects to MariaDB on startup, and
implements the full ported surface — `POST /v1/profiles` (attributes, `ids[]`,
org tree), persons-seed, roles / person-roles / visibility, org subchart, and
three internal service-only S2S lookups kept as SEPARATE routes, one question
each: `GET /internal/persons/by-external-id` (source-type-scoped external id —
the authenticator's login bootstrap), `GET /internal/persons/by-roster-email`
(the login bootstrap of an install that resolves by address — tenant-scoped,
confined to `roster_source_type`, and only for a person still holding a live
account under it) and `GET /internal/persons/by-email-override` (any source, any
tenant — the admin `__override` view-as feature only). (The deprecated legacy
`GET /v1/persons/{email}` is intentionally not carried.)

## Run locally against the dev cluster DB

The service reads MariaDB (`persons`, `account_person_map` in the `identity`
database). For local dev, point it at the dev cluster's MariaDB via
`kubectl port-forward` (requires cluster access / VPN).

### 1. Port-forward MariaDB — terminal 1, keep open
```bash
kubectl -n insight-infra port-forward svc/mariadb 3306:3306
```

### 2. Build the DB URL — terminal 2
Reuse the exact connection string the deployed identity service uses, rewriting
the host to localhost:
```bash
URL=$(kubectl -n insight get secret insight-identity-resolution-config \
  -o jsonpath='{.data.APP__gears__identity_resolution__config__database_url}' | base64 -d \
  | sed 's#@[^/]*/#@127.0.0.1:3306/#')
# → mysql://insight:<password>@127.0.0.1:3306/identity
```

### 3. Run the service — from `src/backend`
Pass the DB URL as an env override. The toolkit maps the underscored
`identity_resolution` environment-key segment to the hyphenated
`identity-resolution` YAML gear name.
```bash
cd src/backend
export APP__gears__identity_resolution__config__database_url="$URL"
cargo run -p identity-resolution -- -c services/identity-resolution/config/insight.yaml
```
Startup log should show `connected to MariaDB` and `HTTP server bound on 0.0.0.0:8082`.

### 4. Verify — terminal 3
```bash
curl -s localhost:8082/health     # {"status":"healthy", ...}
curl -s localhost:8082/healthz    # ok
open http://localhost:8082/docs   # OpenAPI docs page
```

## Notes
- HTTP port **8082** (owned by the `api-gateway` host gear) — same port as the
  retired .NET identity service it replaced, so the cutover flipped only the hostname.
- `database_url` is left **empty** in `config/insight.yaml` — no credentials are
  committed. It is injected via the env override above (or, in a real deploy,
  from the umbrella Secret).
- Config env-override convention: `APP__gears__identity_resolution__config__<field>`
  (double underscore between path segments).
- If the service fails at init with `gear 'identity-resolution' not found`, the
  `gears.identity-resolution.config` section is missing from the config YAML.
