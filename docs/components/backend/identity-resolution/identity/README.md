# Identity Resolution

Person-lookup API over MariaDB `persons`, served by the Rust
`identity-resolution` service (`src/backend/services/identity-resolution/`).
Read-only consumer of the observation log written by the persons-seed
(the service's own `seed` subcommand) and the (forthcoming)
reconciliation service.

The specs below define the API contract the service serves (minus the
deprecated legacy lookup,
see below).

| Spec | Path |
|---|---|
| PRD | [specs/PRD.md](specs/PRD.md) |
| DESIGN | [specs/DESIGN.md](specs/DESIGN.md) |
| ADRs | [specs/ADR/](specs/ADR/) |

## Deployment

| Path | Command |
|---|---|
| Dev (Docker Compose, default) | `./dev-compose.sh up` runs the identity-resolution service in a container alongside MariaDB etc. Build the service image with `./dev-compose.sh build identity-resolution`. No Kind, no umbrella chart. |
| Dev (Kubernetes via gitops) | `cd deploy/gitops && make deploy ENV=local` on a local Kind/OrbStack cluster installs the umbrella chart, which includes identity-resolution when `identityResolution.deploy=true`. |
| Production / staging | Standard umbrella install. Override `identityResolution.deploy=true` and `identityResolution.image.tag=<release>` in your values overlay. |
| Standalone (no umbrella) | `helm install identity-resolution ./src/backend/services/identity-resolution/helm` with a pre-created `insight-identity-resolution-config` Secret. |

The umbrella emits Secret `insight-identity-resolution-config`
automatically when `identityResolution.deploy=true`. It carries
`APP__gears__identity-resolution__config__database_url` (derived from
auto-generated MariaDB credentials in `insight-db-creds`), the
ClickHouse coordinates for the persons-seed reader, and — when set —
`tenant_default_id` (from `global.tenantDefaultId`) and
`bootstrap_admin_person_id` (from
`identityResolution.bootstrapAdminPersonId`).

## API surface

| Endpoint | Description |
|---|---|
| `POST /v1/profiles` | Profile lookup by `value_type`: `email` (tenant-wide), `id` (source-native account id, needs both source fields), or `person_id` (the canonical UUID — the key the metrics runtime and the SPA routes use since the identity cutover). Body-form replacement for the retired path-form `GET /v1/persons/{email}` (removed — zero callers). |
| `POST /v1/visible-persons` | Filters a list of canonical person ids (UUIDs) to the ones the caller may see. Authenticated, not admin-gated — the caller comes from the gateway JWT, so the answer is always their own visible set (ADR-0015). |
| `GET /health` | DB ping. 200 / 503. |
| `GET /healthz` | Process liveness. 200 `text/plain "ok"`. |

Tenant resolution comes from the signed gateway JWT's `tenant_id` claim
(NGINX_BFF R1); the config default feeds only the first-admin bootstrap.

## Local run

```sh
cargo run -p identity-resolution -- -c <config.yaml> migrate   # schema first
cargo run -p identity-resolution -- -c <config.yaml>           # server
```

## Tests

```sh
cd src/backend && cargo test -p identity-resolution
```

DB-backed live tests run when `INTEGRATION_TESTS_MARIADB_URL` is set and
skip cleanly otherwise; see the service README.
