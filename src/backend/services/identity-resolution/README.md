# identity-resolution

Rust port of the .NET `identity` service (epic #1602). **Iteration 1: read API.**
Built on the gears-rust framework — same host pattern as `services/analytics`
(the `api-gateway` system gear is the REST host; auth disabled — the platform
gateway authenticates upstream).

Current state: boots as a gears host, connects to MariaDB on startup, serves
`/health`, and implements the read API — `POST /v1/profiles` (attributes, `ids[]`,
org tree) plus the deprecated `GET /v1/persons/{email}`.

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
URL=$(kubectl -n insight get secret insight-identity-config \
  -o jsonpath='{.data.IDENTITY__mariadb__url}' | base64 -d \
  | sed 's#@[^/]*/#@127.0.0.1:3306/#')
# → mysql://insight:<password>@127.0.0.1:3306/identity
```

### 3. Run the service — from `src/backend`
Pass the DB URL as an env override. **Use `env "NAME=VALUE" …`, not `export`** —
the gear name contains a hyphen (`identity-resolution`), which a shell `export`
variable name cannot contain.
```bash
cd src/backend
env "APP__gears__identity-resolution__config__database_url=$URL" \
  cargo run -p identity-resolution -- -c services/identity-resolution/config/insight.yaml
```
Startup log should show `connected to MariaDB` and `HTTP server bound on 0.0.0.0:8083`.

### 4. Verify — terminal 3
```bash
curl -s localhost:8083/health     # {"status":"healthy", ...}
curl -s localhost:8083/healthz    # ok
open http://localhost:8083/docs   # OpenAPI docs page
```

## Notes
- HTTP port **8083** (owned by the `api-gateway` host gear).
- `database_url` is left **empty** in `config/insight.yaml` — no credentials are
  committed. It is injected via the env override above (or, in a real deploy,
  from the umbrella Secret).
- Config env-override convention: `APP__gears__identity-resolution__config__<field>`
  (double underscore between path segments).
- If the service fails at init with `gear 'identity-resolution' not found`, the
  `gears.identity-resolution.config` section is missing from the config YAML.
