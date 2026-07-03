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
   - [Backend image fallback (ghcr)](#backend-image-fallback-ghcr)
   - [Settings reference (`.env.compose`)](#settings-reference-envcompose)
6. [Daily workflow](#daily-workflow)
   - [Edit code](#edit-code)
   - [Auto-reload mechanic](#auto-reload-mechanic)
   - [Common operations](#common-operations)
7. [Seeding](#seeding)
   - [Compose](#compose)
   - [Kubernetes](#kubernetes)
8. [Dev auth chain (no-auth mode)](#dev-auth-chain-no-auth-mode)
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
#   apiGateway.authDisabled: true            # local sandbox; flip for real OIDC
#   <l2>.host / <l2>.port                    # only when <l2>.deploy=false

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
│  api-gateway (Rust :8080)  analytics (Rust :8081)                    │
│  identity (.NET 9 :8082)                                              │
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

### Backend image fallback (ghcr)

Skip the local Rust/dotnet build for one or more services:

```bash
# Per-run flags
./dev-compose.sh up --from-ghcr=api-gateway,identity
./dev-compose.sh up --build-only=analytics     # invert: build only this

# Or pin in .env.compose
API_GATEWAY_IMAGE=ghcr.io/constructorfabric/insight-api-gateway:latest
```

The script writes `deploy/compose/override.generated.yml` (gitignored) that
drops the `build:` + bind-mount for the chosen services.

### Settings reference (`.env.compose`)

`.env.compose.example` documents every knob. Blocks:

- **Auto-reload** — `ENABLE_AUTO_RELOAD` (compose-only, never in k8s)
- **Frontend** — `FRONTEND_MODE`, `INSIGHT_FRONT_PATH`, `FRONTEND_IMAGE`
- **Backend image overrides** — `API_GATEWAY_IMAGE`, `ANALYTICS_IMAGE`, `IDENTITY_IMAGE`
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
| `src/backend/services/api-gateway/config/*.yaml` | save | watchexec → ~1s restart (bind-mounted) |
| identity / analytics env | edit `docker-compose.yml`, `up -d <svc>` | container respawn |
| Frontend (`dev` mode) | save | Vite HMR |
| Frontend (`built` mode) | `./dev-compose.sh build frontend` | nginx auto |
| Frontend (`ghcr` mode) | switch modes | — |

Build targets:

```bash
./dev-compose.sh build api-gateway     # Rust gateway
./dev-compose.sh build analytics   # Rust analytics
./dev-compose.sh build identity        # .NET 9 publish
./dev-compose.sh build frontend        # pnpm build → dist/
./dev-compose.sh build rust            # both Rust services
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
docker compose logs -f api-gateway analytics identity

# Inspect databases
docker compose exec mariadb mariadb -uinsight -pinsight-local identity
docker compose exec clickhouse clickhouse-client --user insight --password insight-local

# Stop / wipe (escalating)
./dev-compose.sh down                  # stop containers; keep volumes + .env.compose
./dev-compose.sh down --volumes        # also wipe named volumes + deploy/compose/build/
./dev-compose.sh prune                 # interactive nuke — see below

# One-off cargo work
docker compose --profile build run --rm build-rust cargo test -p insight-api-gateway
```

`prune` is the only command that removes `.env.compose`. Always
interactive — no `--yes` switch. Asks separately whether to also remove
pulled `ghcr.io/constructorfabric/insight-*` images (defaults to no —
they're slow to re-pull). After prune, next `up` re-runs the wizard.

### Switch the gateway to real OIDC

Edit `src/backend/services/api-gateway/config/no-auth.yaml` directly
(bind-mounted):

1. Set `api-gateway.config.auth_disabled: false`.
2. Fill in `oidc-authn-plugin.config.issuer_url`, `audience`, and the
   `auth-info` section's `client_id` / `scopes`.
3. Save — watchexec restarts in ~1 second.

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

## Dev auth chain (no-auth mode)

When `AUTH_DISABLED=true` and `VITE_DEV_USER_EMAIL=you@yourorg.com`:

```text
1. Browser   → fetch-with-auth.ts builds an unsigned JWT
               ({alg:"none"}.{email, sub, preferred_username}.) and sets
               Authorization: Bearer <jwt> on every request.
2. Vite      → proxies /api/* to api-gateway via the compose network.
3. Gateway   → auth_disabled=true skips JWT validation but forwards
               the Authorization header end-to-end.
4. Service   → identity's HeaderCallerContext falls back to JWT claims
               when X-Insight-Person-Id is absent: reads email/sub/oid,
               looks the value up in persons (value_type='email'),
               returns the matching person_id. Tenant comes from
               X-Insight-Tenant-Id or
               IDENTITY__identity__tenant_default_id.
```

So three things must all be true for a dev call to succeed:

- `VITE_DEV_USER_EMAIL` is set (FE builds the bearer token).
- A row in `persons` has `value_type='email'` and `value_id` matching
  that address (run `./dev-compose.sh seed identity`).
- The gateway proxies `/api/{prefix}` to the right upstream (see
  `no-auth.yaml`).

If you bypass the FE (curl from the host), you must construct the same
fake bearer yourself; otherwise identity returns
`401 caller_unresolved`.

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

**api-gateway exits with `oidc-authn-plugin: issuer_url is required`.**
`no-auth.yaml` ships with a placeholder issuer because the plugin
module is registered even when `auth_disabled=true`. Restore the
`issuer_url: https://no-auth.local/oauth2/default` line — the URL is
never actually called.

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

- Rust: `cargo fmt` + `cargo clippy --all-targets -- -D warnings`
- C#: `dotnet format`
- Frontend: `pnpm lint` + `pnpm tsc --noEmit`
- Always sign your commits: `git commit -s ...`
- Push to your fork (`origin`), not to `cf` upstream
- PR description should link the relevant spec under
  `docs/components/<area>/specs/`

CI runs the same checks on every PR.
