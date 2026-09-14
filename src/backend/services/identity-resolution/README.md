# identity-resolution

The Insight identity service (Rust; epic #1602).
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

## Roster profiles and identity corrections

An account binding determines activity attribution. A person's selected roster
account determines their complete profile; attaching another account does not
replace or enrich it. The selected account's reference is stored with each
temporal `people` revision, not used as the person's identity key.

Corrections reconcile bindings, roster rows, and account-backed reporting lines
in one MariaDB transaction, coordinated with the seed lock. A failed evidence
read or an ambiguous profile source aborts the correction. Reporting changes
close and open intervals in `org_chart`; historical relationships are retained.
The hierarchy remains an authorization input, and a missing manager never
enables flat visibility.

Corrected person-based manager references retain the original source reference
and its destination, so unchanged source evidence cannot undo a merge. A changed
source reference or explicit clear replaces that correction. Account-based
manager references remain resolvable after exclusion and rebinding. Corrections
read source observations only for affected accounts and reporting dependencies;
bounded tenant binding and hierarchy snapshots remain available for resolution
and cycle checks.

The account listing reports `profile_source` as `selected`, `eligible`, or
`ineligible`. An administrator can use
`PUT /v1/resolution/persons/{person_id}/profile-source` with an `AccountRef` to
select an active roster account already held by the person. This explicitly
replaces the whole profile and its source reporting relationship. Move a selected
account only after choosing a replacement when another roster account remains.

Seeding uses the same selected account. New observations from it can update the
profile; explicit field clears remove values, while missing evidence retains
them. Existing ambiguous profiles are retained until their source is established
or explicitly selected. Source selection does not change metric computation or
transfer role and visibility grants between people.

Apply the normal service migrations and update both the identity service and
the toolbox that runs seeding. No connector configuration or resync is required.
The migration is additive, but older seed writers do not preserve the new
provenance: do not run old seed writers alongside the updated writers.

## Run locally against the dev cluster DB

The service reads MariaDB (the `persons` journal in the `identity`
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
- HTTP port **8082** (owned by the `api-gateway` host gear).
- `database_url` is left **empty** in `config/insight.yaml` — no credentials are
  committed. It is injected via the env override above (or, in a real deploy,
  from the umbrella Secret).
- Config env-override convention: `APP__gears__identity_resolution__config__<field>`
  (double underscore between path segments).
- If the service fails at init with `gear 'identity-resolution' not found`, the
  `gears.identity-resolution.config` section is missing from the config YAML.
