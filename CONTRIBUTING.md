# Contributing to Insight

Two deployment paths — Docker Compose for day-to-day dev, Kubernetes
when you need Airbyte / Argo Workflows. Both share a single first-run
wizard at [`deploy/compose/insight-init.sh`](deploy/compose/insight-init.sh).

Open a PR after reading [AGENTS.md](AGENTS.md) and the relevant spec
files under `docs/components/<area>/specs/`.

## Contents

1. [Quick start](#quick-start)
2. [Prerequisites](#prerequisites)
3. [Deployment paths](#deployment-paths)
   - [Docker Compose (default)](#docker-compose-default)
   - [Kubernetes — interactive](#kubernetes--interactive)
   - [Kubernetes — non-interactive (CI)](#kubernetes--non-interactive-ci)
4. [What's in the compose stack](#whats-in-the-compose-stack)
5. [Compose configuration](#compose-configuration)
   - [First-run wizard + re-runs](#first-run-wizard--re-runs)
   - [External MariaDB / ClickHouse](#external-mariadb--clickhouse)
   - [Frontend modes](#frontend-modes)
   - [Local dev auth backend (fakeidp / Keycloak)](#local-dev-auth-backend-fakeidp--keycloak)
   - [Backend image fallback (ghcr)](#backend-image-fallback-ghcr)
   - [Settings reference (`.env.compose`)](#settings-reference-envcompose)
6. [Daily workflow](#daily-workflow)
   - [Edit code](#edit-code)
   - [Auto-reload mechanic](#auto-reload-mechanic)
   - [Common operations](#common-operations)
7. [Seeding](#seeding)
   - [Compose](#compose)
   - [Kubernetes](#kubernetes)
8. [Dev auth chain (fakeidp)](#dev-auth-chain-fakeidp)
9. [Troubleshooting](#troubleshooting)
10. [Code style and reviews](#code-style-and-reviews)

---

## Quick start

Clone, run one command, answer four prompts, get a fully populated
stack:

```bash
git clone https://github.com/constructorfabric/insight.git
cd insight
./dev-compose.sh up
```

First-run wizard prompts (Enter accepts defaults):

| Prompt | Default | Effect |
| --- | --- | --- |
| Use local MariaDB? | Y | Compose starts mariadb on :3306 |
| Use local ClickHouse? | Y | Compose starts clickhouse on :8123 |
| `VITE_DEV_USER_EMAIL` | `dev@company.nonpresent` | Dev-team lead in the seed roster |
| Frontend mode | `1` (ghcr) | Pulls the published `insight-front:latest` image |

Then the script builds host artefacts, brings up the stack, auto-seeds
the demo dataset (25 persons + ~24k ClickHouse rows across 16 silver
tables). First run: ~5–15 min cold Rust compile; subsequent runs reuse
the Cargo cache and finish in seconds.

Open <http://localhost:3000>. `dev@company.nonpresent` leads the dev
team; CEO sees the whole org tree. To use CEO more set email to `email_ceo@company.nonpresent`.

> **Stuck after pulling an update?** The compose stack runs full auth
> (fakeidp → authenticator → nginx gateway → downstream JWT verification;
> no `auth_disabled`). If a stale local config trips it up, wipe and
> regenerate: `rm -f .env.compose && ./dev-compose.sh up` re-runs the
> first-run wizard, and `./dev-compose.sh prune` additionally clears the
> containers + volumes for a clean slate.

---

## Prerequisites

**Compose path** — only Docker:

| Tool | Min | Install |
| --- | --- | --- |
| Docker Engine | 24+ | Docker Desktop / OrbStack / distro package |
| docker compose v2 | 2.20+ | bundled with Docker Desktop/OrbStack |
| git | any | xcode-select / apt / winget |

No Rust / .NET / Node / pnpm on the host — every build runs in a
builder container.

**K8s path** — also `kubectl`, `helm`, `kubeseal`, `yq`, `jq`, plus a
local cluster (OrbStack with Kubernetes / k3d / kind / minikube). No
frontend checkout needed — the umbrella chart pulls
`ghcr.io/constructorfabric/insight-front:<tag>` from GHCR.

**Frontend checkout** — only needed for compose with
`FRONTEND_MODE=dev` (Vite HMR) or `built` (host-built dist). The
default mode (`ghcr`) pulls the published image, so a fresh laptop with
only Docker can run the full compose stack. When you do need the
checkout, the wizard's "clone" option offers to git-clone it for you;
otherwise it expects a sibling repo (override `INSIGHT_FRONT_PATH` in
`.env.compose` to point elsewhere):

```text
cf/
├── insight/         (this repo)
└── insight-front/   (only for FRONTEND_MODE=dev or built)
```

---

## Deployment paths

| Path | Driver | Use when |
| --- | --- | --- |
| **compose** | `./dev-compose.sh up` | Day-to-day backend / frontend work. Default. |
| **k8s** | `cd deploy/gitops && make deploy ENV=local` | Testing the published umbrella; Airbyte / Argo work; real cluster shape. |

Both share the same wizard so the MariaDB / ClickHouse / tenant /
dev-email answers are identical across them.

### Docker Compose (default)

Covered by the [Quick start](#quick-start). See also
[Compose configuration](#compose-configuration) for the post-wizard
knobs and [Daily workflow](#daily-workflow) for the edit-build loop.

### Kubernetes — interactive

```bash
cd deploy/gitops
make deploy ENV=local
# or, if your kubeconfig lives elsewhere:
KUBECONFIG=/path/to/config.yaml make deploy ENV=local
```

`kubectl` / `helm` / `kubeseal` all honour `$KUBECONFIG`; the wizard
prints which file it's reading at startup so you can abort and retry
with a different one if the context list looks wrong.

On first run the wizard generates (and gitignores) the local artifacts:

- `environments/local/inventory.yaml` (cluster topology + toggles)
- `environments/local/values.yaml` (umbrella overlay)
- `secrets-store.yaml` (cleartext for the seal step)
- `environments/local/.env.local` (Airbyte setup creds — only when
  `system.airbyte=true`)

Then the chain runs: `bootstrap → fetch-cert → seal → system →
deploy-app`. Subsequent runs skip the wizard and reconcile the stack.

K8s and compose can coexist — disjoint host ports by default. Demo-data
seeding on k8s is manual (wizard output prints the port-forward +
`deploy/seed/` recipe).

### Kubernetes — non-interactive (CI)

The wizard refuses without a TTY. For CI / scripted runs, pre-populate
the four files the wizard would have generated; `make deploy ENV=local`
skips the wizard whenever `environments/local/inventory.yaml` exists.

```bash
cd deploy/gitops

# 1. Inventory: cluster topology + bootstrap/system toggles.
cp environments/local/inventory.yaml.template environments/local/inventory.yaml
# Edit:
#   kubeContext: <ctx>                       # required
#   bootstrap.{ingressNginx,certManager,sealedSecrets}: true|false
#   system.{airbyte,argoWorkflows,redpandaConsole,loki,alloy,grafana}: true|false

# 2. Umbrella overlay: image tags / OIDC / tenant id / L2 hosts.
cp environments/local/values.yaml.template environments/local/values.yaml
# Edit:
#   global.tenantDefaultId: <UUID>           # required for external DBs with seeded persons
#   fakeidp.deploy: true                     # local sandbox IdP; set false + point
#   authenticator.oidc.issuerUrl: <idp>      #   authenticator.oidc.* at a real IdP
#   <l2>.host / <l2>.port                    # only when <l2>.deploy=false
# The `__INGRESS_LB_IP__` placeholders (authenticator.oidc.issuerUrl,
# fakeidp.issuer) must be replaced with your ingress-nginx LoadBalancer IP
# (`kubectl -n ingress-nginx get svc ingress-nginx-controller`).

# 3. Cleartext secret store (read by `make seal`, never committed).
cp secrets-store.yaml.template secrets-store.yaml
# Edit each `insight-local-*-creds:` block, replacing REPLACE_* with real passwords.

# 4. Airbyte setup creds — only when inventory.system.airbyte=true.
cat > environments/local/.env.local <<'EOF'
AIRBYTE_SETUP_EMAIL=admin@example.com
AIRBYTE_SETUP_ORG=Insight
EOF

# 5. Run the chain.
KUBECONFIG=/path/to/config.yaml make deploy ENV=local
```

Idempotent — re-running on a converged cluster is near-noop. For CI
"exit 0 on a fresh cluster" is the smoke check; helm's `--wait` ensures
every Deployment is Ready before the chain returns.

---

## What's in the compose stack

```text
┌──────────────────────────────────────────────────────────────────────┐
│  Frontend (FRONTEND_MODE=dev|built|ghcr)                             │
│  Vite dev (HMR) / nginx+dist / ghcr image — port 3000                │
├──────────────────────────────────────────────────────────────────────┤
│  Backend                                                              │
│  gateway (nginx :8080)  analytics (Rust :8081)                       │
│  identity (.NET 9 :8082)                                              │
│  authenticator (Rust :8083/:8093)  fakeidp (Rust :8084, dev-only)    │
├──────────────────────────────────────────────────────────────────────┤
│  Infra                                                                │
│  MariaDB :3306  ClickHouse :8123/:9000  Redis :6379  Redpanda :19092…│
└──────────────────────────────────────────────────────────────────────┘
```

Every web service publishes a host port; override `*_PORT` in
`.env.compose` if you have conflicts.

Does **not** ship Airbyte or Argo Workflows — those need k8s. Use the
[Kubernetes path](#kubernetes--interactive).

---

## Compose configuration

### First-run wizard + re-runs

`.env.compose` is generated by the wizard on first `up`. Re-run by
deleting it (or `./dev-compose.sh prune`). Hand-edit afterwards — the
wizard only runs when the file is missing.

### External MariaDB / ClickHouse

Answer **N** to the relevant wizard prompt; the wizard asks for host /
port / user / password, then probes connectivity:

- **MariaDB** — spins up a transient `mariadb:11.4` container, runs
  `SELECT 1`. Bad credentials abort the wizard.
- **ClickHouse** — host-side `curl` against the HTTP interface. Same
  fail-fast.

When at least one DB is external, the wizard also asks for
`TENANT_DEFAULT_ID` (UUID in your `persons.insight_tenant_id`) and
whether to seed the external DB (defaults to **No** — pre-marks
`SEEDED_LOCAL_*=true` so `up` leaves your DB alone).

> **`localhost` gotcha.** Inside the container, `localhost` is the
> container itself. Use `host.docker.internal` (Mac/Windows) or your
> LAN IP. The wizard warns when it sees `localhost`.

To switch later: `./dev-compose.sh prune` and re-run the wizard, or
hand-edit `*_EXTERNAL` / `*_HOST` / `*_INTERNAL_PORT` in `.env.compose`
and bounce the stack.

### Frontend modes

| Mode | Wizard does | What runs | Auto-reload? | When |
| --- | --- | --- | --- | --- |
| `ghcr` | `FRONTEND_MODE=ghcr` | published image | no | Backend-only work; save laptop CPU. |
| `dev` (local) | `FRONTEND_MODE=dev` + checks `INSIGHT_FRONT_PATH` exists | `pnpm dev` in node:24 | Vite HMR | Active FE work on an existing checkout. |
| `dev` (clone) | `git clone insight-front` then same as above | `pnpm dev` in node:24 | Vite HMR | First-time setup, no checkout yet. |

A fourth `built` mode (nginx + host-built dist) is undocumented in the
wizard. To use it, hand-edit `FRONTEND_MODE=built` in `.env.compose`,
`./dev-compose.sh build frontend`, then bounce.

**Switching modes later:** edit `.env.compose` and `down && up
--skip-build`, or override per-run:

```bash
./dev-compose.sh up --frontend-mode=ghcr --skip-build
./dev-compose.sh up --no-frontend                  # backend-only
```

### Local dev auth backend (fakeidp / Keycloak)

The `authenticator` service's login always runs the same BFF code path — only
the IdP behind it changes. Select it with **`AUTH_MODE` in `.env.compose`**
(a persisted setting like `FRONTEND_MODE`):

- `fakeidp` (default) — a tiny in-repo test double, no login screen, no setup.
- `keycloak` — a real Keycloak container with an actual login form, for
  exercising the genuine OIDC code path.

```dotenv
# .env.compose
AUTH_MODE=keycloak
```

```bash
./dev-compose.sh up            # reads AUTH_MODE from .env.compose
./dev-compose.sh up --auth=keycloak   # optional per-run override
```

In keycloak mode the authenticator logs in server-side against the pre-seeded
realm's `insight-authenticator` confidential client; the SPA stays cookie/BFF (no
special frontend build), and dev-compose points both the browser and the
authenticator at a host-IP Keycloak issuer so the id_token `iss` validates. See
[`deploy/compose/keycloak/README.md`](deploy/compose/keycloak/README.md) for
login creds, admin console, enforcement, and the custom-claims contract. This is
distinct from [switching the gateway to real
OIDC](#switch-the-gateway-to-real-oidc) below (a production-style toggle against
an external IdP).

### Backend image fallback (ghcr)

Skip the local Rust/dotnet build for one or more services:

```bash
# Per-run flags (recognised services: analytics, identity)
./dev-compose.sh up --from-ghcr=analytics,identity
./dev-compose.sh up --build-only=analytics     # invert: build only this

# Or pin in .env.compose
ANALYTICS_IMAGE=ghcr.io/constructorfabric/insight-analytics:latest
```

The script writes `deploy/compose/override.generated.yml` (gitignored) that
drops the `build:` + bind-mount for the chosen services.

### Settings reference (`.env.compose`)

`.env.compose.example` documents every knob. Blocks:

- **Auto-reload** — `ENABLE_AUTO_RELOAD` (compose-only, never in k8s)
- **Frontend** — `FRONTEND_MODE`, `INSIGHT_FRONT_PATH`, `FRONTEND_IMAGE`
- **Backend image overrides** — `ANALYTICS_IMAGE`, `IDENTITY_IMAGE`
- **Host ports** — every published port is configurable
- **Database mode** — `MARIADB_EXTERNAL`/`_HOST`/`_INTERNAL_PORT`/…, ClickHouse equivalents (see [External DBs](#external-mariadb--clickhouse))
- **Credentials** — local-only, kept in dotenv per project policy
- **Seed bookkeeping** — `SEEDED_LOCAL_MARIA`, `SEEDED_LOCAL_CH`
- **Tenant / OIDC** — `TENANT_DEFAULT_ID`, OIDC client info
- **Log level** — `RUST_LOG`

---

## Daily workflow

### Edit code

| Edit | Then | Picked up by |
| --- | --- | --- |
| Rust / C# source | `./dev-compose.sh build <service>` | watchexec → ~1s restart |
| `deploy/compose/gateway/routes.yaml` (gateway route table) | `docker compose restart gateway` | nginx.conf regenerated at startup |
| `src/backend/services/authenticator/config/*.yaml` | save | watchexec → ~1s restart (bind-mounted) |
| `deploy/compose/analytics-fullauth.yaml` (analytics plugin config) | save | watchexec → ~1s restart (bind-mounted) |
| identity / analytics env | edit `docker-compose.yml`, `up -d <svc>` | container respawn |
| Frontend (`dev` mode) | save | Vite HMR |
| Frontend (`built` mode) | `./dev-compose.sh build frontend` | nginx auto |
| Frontend (`ghcr` mode) | switch modes | — |

Build targets:

```bash
./dev-compose.sh build analytics       # Rust analytics
./dev-compose.sh build authenticator   # Rust authenticator
./dev-compose.sh build identity        # .NET 9 publish
./dev-compose.sh build frontend        # pnpm build → dist/
./dev-compose.sh build rust            # all Rust services (analytics + authenticator)
./dev-compose.sh build all             # everything
./dev-compose.sh up --skip-build       # bounce without rebuilding
```

### Auto-reload mechanic

Each backend container's `ENTRYPOINT` is
`src/backend/docker-entrypoint.sh`:

```text
docker-entrypoint.sh <watched-path> -- <command> [args...]
```

- `ENABLE_AUTO_RELOAD` unset (prod) → `exec`s the command bare.
- `ENABLE_AUTO_RELOAD=true` (set in `.env.compose`) → wraps in
  `watchexec --restart --watch <watched-path>`. Any change to the
  bind-mounted binary triggers SIGTERM + respawn.

**Never set `ENABLE_AUTO_RELOAD` in a k8s manifest** — compose-only.

watchexec watches the parent **directory** (`/app`), not the file —
modern watchexec needs a dir. The image pins the musl static build of
watchexec because bookworm-slim's glibc is older than what stock
watchexec wants, and `useradd -m` ensures `appuser` has a usable
`$HOME` (watchexec dies during config resolution without one).

### Common operations

```bash
# Tail logs
docker compose logs -f gateway authenticator analytics identity fakeidp

# Inspect databases
docker compose exec mariadb mariadb -uinsight -pinsight-local identity
docker compose exec clickhouse clickhouse-client --user insight --password insight-local

# Stop / wipe (escalating)
./dev-compose.sh down                  # stop containers; keep volumes + .env.compose
./dev-compose.sh down --volumes        # also wipe named volumes + deploy/compose/build/
./dev-compose.sh prune                 # interactive nuke — see below

# One-off cargo work
docker compose --profile build run --rm build-rust cargo test -p analytics
```

`prune` is the only command that removes `.env.compose`. Always
interactive — no `--yes` switch. Asks separately whether to also remove
pulled `ghcr.io/constructorfabric/insight-*` images (defaults to no —
they're slow to re-pull). After prune, next `up` re-runs the wizard.

### Point the authenticator at a real IdP

Auth is **always on** — there is no bypass. Local dev logs in against the
in-repo `fakeidp` OIDC provider by default. The authenticator is
IdP-agnostic, so switching to a real IdP (Entra, Keycloak, …) is a
config change, not a mode flip:

- **Compose** — set `AUTHENTICATOR_OIDC_ISSUER` (plus `OIDC_CLIENT_ID` /
  `OIDC_CLIENT_SECRET` and `AUTHENTICATOR_REDIRECT_URI`) in `.env.compose`
  and bounce the `authenticator`. Leaving them unset falls back to
  `http://fakeidp:8084`.
- **K8s** — set `authenticator.oidc.issuerUrl` (+ `clientId` /
  `redirectUri`) in the values overlay and set `fakeidp.deploy: false`.

> **redirect/issuer: local uses `localhost`, remote needs a real host.** On local
> k8s `issuerUrl` is the ingress LB IP (e.g. `http://192.168.139.2/kc/realms/insight`)
> because it must be reachable by the browser **and** the in-cluster authenticator
> pod (`localhost` inside the pod is the pod). But `redirectUri` is
> `http://localhost/auth/callback`: it's where the `__Host-sid` cookie is set, and
> that cookie needs a *secure context* — `http://localhost` qualifies (OrbStack
> binds the ingress to `localhost:80`), a plain-HTTP IP does not. This is
> **single-machine only**: on a *remote* cluster (deploy over a kube-context) your
> browser's `localhost` is your laptop, not the ingress, so it won't work. Remote /
> shared envs give the ingress a real DNS host + TLS (cert-manager) and set **both**
> `issuerUrl` and `redirectUri` to that `https://…` host — over HTTPS `__Host-`
> cookies work on any hostname, so no `localhost` hack — as the `dev` / `virtuozzo`
> overlays do. To poke a remote cluster quickly: `kubectl port-forward
> svc/insight-gateway 8080:80` and use `http://localhost:8080`.

See ADR
[`docs/components/backend/authenticator/specs/ADR/0001-per-environment-idp-selection.md`](docs/components/backend/authenticator/specs/ADR/0001-per-environment-idp-selection.md)
for the per-environment IdP selection rationale.

---

## Seeding

The seed package lives in [`deploy/seed/`](deploy/seed/) — its
README documents the ruff / mypy / venv setup. Both deploy paths use
the same package; only how it's invoked differs.

**Identity content (after `seed identity`):** CEO, your
`VITE_DEV_USER_EMAIL` person (leads the dev team), 4 team leads (dev /
sales / HR / support), 20 ICs (5/team). Visibility is wired through
the BambooHR org-chart source so per-caller `/v1/persons/{email}`
lookups resolve correctly — dev lead sees their 5 reports, CEO sees
the whole tree.

**Silver content (after `seed silver`):** bronze + silver placeholder
tables, every `src/ingestion/scripts/migrations/*.sql` applied
(produces the `insight.*` gold views), ~24k rows across 16 silver
tables profile-typed per team (`class_git_*` for devs, `class_crm_*`
for sales, …). The full per-team activity table is in
[`deploy/seed/profiles.py`](deploy/seed/profiles.py). analytics's
schema validator flips from "80 metrics error" to "80 ok".

### Compose

`./dev-compose.sh up` auto-seeds on first run after the wizard, then
flips `SEEDED_LOCAL_MARIA` / `SEEDED_LOCAL_CH` to `true` so subsequent
`up`s skip it. Re-seed manually:

```bash
./dev-compose.sh seed            # identity + silver (everything)
./dev-compose.sh seed identity   # MariaDB only
./dev-compose.sh seed silver     # ClickHouse only
```

To force auto-seed on next `up`, clear the `SEEDED_LOCAL_*` markers in
`.env.compose` or `./dev-compose.sh prune`.

### Kubernetes

No auto-seed. The chart doesn't ship a `seed` Job, so you point the
same Python package at port-forwarded L2 services from the host. One
recipe per re-seed:

```bash
# 1. Port-forward MariaDB + ClickHouse in the background.
KUBECONFIG=/path/to/config.yaml kubectl -n insight-infra \
  port-forward svc/mariadb 3306:3306 &
KUBECONFIG=/path/to/config.yaml kubectl -n insight-infra \
  port-forward svc/clickhouse 8123:8123 &

# 2. Run the seed package against them. First time only: bootstrap a venv.
cd deploy/seed
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt

# Identity + silver. Drop `all` and pass `identity` / `silver` for partial.
# Schema inputs (placeholders script + gold-view migrations) are
# auto-located: the container bind-mount when present, otherwise
# repo-relative to deploy/seed. No path env vars needed.
MARIADB_HOST=127.0.0.1     MARIADB_PORT=3306 \
MARIADB_USER=insight       MARIADB_PASSWORD=insight-local \
CLICKHOUSE_HOST=127.0.0.1  CLICKHOUSE_HTTP_PORT=8123 \
CLICKHOUSE_USER=insight    CLICKHOUSE_PASSWORD=insight-local \
VITE_DEV_USER_EMAIL=dev@company.nonpresent \
  .venv/bin/python seed.py all

# 3. Kick analytics so its schema validator re-runs against the
#    now-populated silver tables. Without this, schema_status stays
#    cached at boot-time 'table_not_found' and the FE shows "no peer
#    data" everywhere (cf/insight#1307).
KUBECONFIG=/path/to/config.yaml kubectl -n insight \
  rollout restart deploy/insight-analytics

# 4. Stop the port-forwards.
kill %1 %2
```

Use the real cluster credentials in place of `insight-local` if you
switched to external DBs at wizard time — the values are whatever the
operator stored in `secrets-store.yaml` and `make seal` baked into the
cluster's `mariadb-creds` / `clickhouse-creds` Secrets.

When `frontend.devUserEmail` (set by the wizard / values overlay) and
the seeded `VITE_DEV_USER_EMAIL` match, the FE's dev impersonation
resolves to a real person row and dashboards populate.

---

## Dev auth chain (fakeidp)

Auth is **always on** (NGINX_BFF EPIC #1583) — there is no no-auth mode.
Every request that reaches a backend carries an ES256 gateway JWT that
the `gateway` (nginx / OpenResty) injects after the `authenticator`
confirms a valid session. Local dev logs in against `fakeidp`, an
in-repo dev-only OIDC provider, so the real login code path runs with no
external IdP.

```text
1. Browser   → GET /auth/login on the gateway (:8080). The authenticator
               starts an OIDC authorization-code + PKCE flow and 302s to
               fakeidp's /authorize.
2. fakeidp   → no login screen: mints a one-time code for the default
               user (VITE_DEV_USER_EMAIL) and 302s back to /auth/callback.
3. Gateway   → /auth/callback → authenticator exchanges the code for
               tokens, resolves the person in identity, opens an opaque
               session in Redis, and sets the `__Host-sid` cookie.
               (`__Host-` requires a secure context — HTTPS or
               http://localhost.)
4. Gateway   → on every subsequent /api/* request it runs auth_request
               against the authenticator, mints/attaches the ES256
               gateway JWT, and proxies to analytics / identity + the SPA.
5. Downstream→ analytics verifies the JWT via cf-gears-oidc-authn-plugin
               (JWKS over the authn-tls discovery front); identity
               verifies it with .NET JwtBearer against the authenticator's
               JWKS endpoint. Both resolve the caller person from the
               token claims.
```

**Driving it from a real browser (not just curl)** works out of the box — two
things a browser needs that curl doesn't are handled automatically:

- **The callback rides the SPA's own origin** so the `__Host-sid` cookie lands
  where the SPA runs: `AUTHENTICATOR_REDIRECT_URI` defaults to
  `http://localhost:3000/auth/callback` (the Vite origin, which proxies `/auth` +
  `/api` to the gateway) — never the authenticator's own `:8083`, where the
  cookie would strand.
- **The fakeidp issuer is a host IP, not a hostname.** `./dev-compose.sh up`
  auto-detects your host IP and sets `FAKEIDP_ISSUER` + `AUTHENTICATOR_OIDC_ISSUER`
  to `http://<host-ip>:8084`. A hostname (`fakeidp:8084`) gets HTTPS-upgraded by
  the browser and fails (fakeidp is http-only); `localhost` means the container
  itself. An IP literal is reached un-upgraded by the browser and by the
  containers alike. (curl/e2e flows run inside the compose network, so when no
  issuer is set they fall back to `fakeidp:8084` and don't need this.)

So a dev call succeeds when:

- The stack is up with `fakeidp` (default profile) and the authenticator
  dev signing key + authn-tls cert exist (generated by `dev-compose.sh up`).
- A row in `persons` has `value_type='email'` and `value_id` matching
  `VITE_DEV_USER_EMAIL` (run `./dev-compose.sh seed identity` — fakeidp's
  default login resolves to that seeded person).
- The gateway's `routes.yaml` proxies `/api/{prefix}` to the right
  upstream (`deploy/compose/gateway/routes.yaml`).

To drive it from the host with `curl` (or a browser), start at
`http://localhost:8080/auth/login` and follow the redirects with a cookie
jar so the `__Host-sid` cookie is captured; subsequent `/api/*` calls
reuse that session. `fakeidp` itself exposes a copy-paste code+PKCE flow
in `src/backend/services/fakeidp/README.md` for exercising it directly.

---

## Troubleshooting

**`docker compose up` says a bind-mount path doesn't exist.**
You probably skipped the build phase. Re-run `./dev-compose.sh up`
without `--skip-build`, or `./dev-compose.sh build all` first.

**Container exits immediately with "exec format error".**
The bind-mounted binary is the wrong architecture (e.g. host-built on
Apple Silicon, container is linux/amd64). Always build via
`./dev-compose.sh build` — never `cargo build` from the host shell.

**`watchexec: GLIBC_2.39 not found` or `No such file or directory`.**
Image out-of-date (the Dockerfile pins the musl static build of
watchexec and creates a home dir for `appuser`). Force a rebuild:
`docker compose build --no-cache <service>`.

**`authenticator` exits at startup / login fails.**
The authenticator needs its dev ES256 signing key
(`deploy/compose/authenticator-dev-keys/current.pem`) and the analytics
plugin needs the authn-tls discovery cert
(`deploy/compose/authn-tls-certs/`). Both are generated by
`./dev-compose.sh up` (never committed). If they're missing or stale,
re-run `up` (or `prune` then `up`) so the key/cert are regenerated.

**Login returns 403 / "person not found".**
`fakeidp`'s default login identity (`VITE_DEV_USER_EMAIL`) must resolve
to a seeded person in identity. Run `./dev-compose.sh seed identity`
first — an unknown person is denied.

**Frontend dev mode hangs at "pnpm install".**
First-run installs all deps into the named volume; can take several
minutes. Subsequent starts are fast. Tail with
`docker compose logs -f insight-front-dev`.

**Port already in use.**
Edit the relevant `*_PORT` in `.env.compose` and `up` again.

**`./dev-compose.sh --start-airbyte` errors out.**
Compose stack doesn't ship Airbyte / Argo. Use the
[Kubernetes path](#kubernetes--interactive).

---

## Code style and reviews

- **Enable the git hooks once** (needs [pre-commit](https://pre-commit.com):
  `pipx install pre-commit` or `brew install pre-commit`), then in the repo:
  `pre-commit install`. Hooks run only on the files you stage: `cargo-fmt` +
  `cargo-clippy` on backend Rust (the same checks CI's Rust job gates on),
  `ruff` + `ruff-format` on Python, `yamlfmt` on YAML — so a formatting/lint
  slip fails locally instead of in CI. Skip one with `SKIP=<hook-id> git
  commit`, or all with `git commit --no-verify`. `pre-commit install` only
  manages the `pre-commit` hook, so any personal `prepare-commit-msg` (e.g. DCO
  sign-off) is left untouched.
- Rust: `cargo fmt` + `cargo clippy --all-targets -- -D warnings`
- C#: `dotnet format`
- Frontend: `pnpm lint` + `pnpm tsc --noEmit`
- Always sign your commits: `git commit -s ...`
- Push to your fork (`origin`), not to `cf` upstream
- PR description should link the relevant spec under
  `docs/components/<area>/specs/`

CI runs the same checks on every PR.
