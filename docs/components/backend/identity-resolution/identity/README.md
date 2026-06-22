# Identity (.NET 9)

Person-lookup API over MariaDB `persons`. Read-only consumer of the
observation log written by
[seed-persons-from-identity-input.py](../../../../src/backend/services/identity/seed/seed-persons-from-identity-input.py)
and the (forthcoming) reconciliation service.

| Spec | Path |
|---|---|
| PRD | [specs/PRD.md](specs/PRD.md) |
| DESIGN | [specs/DESIGN.md](specs/DESIGN.md) |
| ADRs | [specs/ADR/](specs/ADR/) |

## Deployment

| Path | Command |
|---|---|
| Dev (Docker Compose, default) | `./dev-compose.sh up` runs the identity service in a container alongside MariaDB etc. Build the service image with `./dev-compose.sh build identity`. No Kind, no umbrella chart. |
| Dev (Kubernetes via gitops) | `cd deploy/gitops && make deploy ENV=local` on a local Kind/OrbStack cluster installs the umbrella chart, which includes identity-resolution when `identity.deploy=true`. |
| Production / staging | Standard umbrella install. Override `identity.deploy=true` and `identity.image.tag=<release>` in your values overlay. |
| Standalone (no umbrella) | `helm install identity ./src/backend/services/identity/helm` with a pre-created `insight-identity-config` Secret. |

The umbrella emits Secret `insight-identity-config` automatically when
`identity.deploy=true`. It carries `IDENTITY__mariadb__url` (derived
from auto-generated MariaDB credentials in `insight-db-creds`),
`IDENTITY__identity__tenant_default_id` (from `identity.tenantDefaultId`,
optional), and `IDENTITY__identity__org_chart_source_type` (from
`identity.orgChartSourceType`, optional — empty falls back to the
`appsettings.yaml` default `bamboohr`).

## API surface

| Endpoint | Description |
|---|---|
| `GET /v1/persons/{email}` | **Deprecated** — see PRD §7.1; new callers use `POST /v1/profiles`. Resolve person by email (case-insensitive). Returns 404 when no current observation matches. |
| `POST /v1/profiles` | Profile lookup by email or source-native id. Body-form replacement for the deprecated path-form. |
| `GET /health` | DB ping. 200 / 503. |
| `GET /healthz` | Process liveness. 200 `text/plain "ok"`. |

Tenant resolution: header `X-Insight-Tenant-Id` → JWT claim (Phase 1.5
stub) → config default. First non-null wins. Empty config default
forces every request to carry the header.

## Local run (VS F5 / `dotnet run`)

```sh
cp src/backend/services/identity/.env.local.example \
   src/backend/services/identity/.env.local
# Edit .env.local with real credentials. VS F5 reads
# Properties/launchSettings.json (gitignored) which mirrors the same
# vars; create your own from the example block in `.env.local`.
```

## Tests

```sh
dotnet test src/backend/services/identity/Insight.Identity.sln
```

Integration tests pull a MariaDB image via Testcontainers; Docker must
be running on the host.
