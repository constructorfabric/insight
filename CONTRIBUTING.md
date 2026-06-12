# Contributing to Insight

This document covers how to get a working dev environment, the two
deployment paths (compose vs. k8s/helm), the daily edit-build-run loop,
and a few sharp edges to know about.

Open a PR after reading [AGENTS.md](AGENTS.md) and the relevant
spec files under `docs/components/<area>/specs/`.

---

## TL;DR — from scratch to a populated FE

Clone, configure, start the stack, seed the demo dataset:

```bash
# 1. Clone both repos side-by-side.
git clone git@github.com:cyberantonz/insight.git
git clone git@github.com:cyberantonz/insight-front.git   # sibling dir
cd insight

# 2. Copy the env template and set your dev-impersonation email
#    (becomes the development-team lead in the demo roster).
cp .env.compose.example .env.compose
sed -i.bak 's|^VITE_DEV_USER_EMAIL=$|VITE_DEV_USER_EMAIL=you@yourorg.com|' .env.compose && rm .env.compose.bak

# 3. Build host artefacts + start every service.
#    First run: 5–15 minutes (cold Rust compile + first image pulls).
./dev-compose.sh up

# 4. Populate the demo dataset (identity persons + 24k silver rows +
#    gold views). Idempotent; safe to re-run.
./dev-compose.sh seed all

# 5. Restart the frontend so it picks up the new VITE_DEV_USER_EMAIL.
docker compose -f docker-compose.yml --profile front-dev up -d insight-front-dev
```

Open <http://localhost:3000>. The dashboards now have data — `you@yourorg.com`
is the dev-team lead, the CEO sees the whole tree, every team has
60 days of activity.

Daily workflow:

```bash
./dev-compose.sh build api-gateway     # rebuild one Rust service after a code edit
./dev-compose.sh build all             # rebuild everything
./dev-compose.sh seed silver           # refresh ClickHouse data only
./dev-compose.sh down                  # stop everything; data preserved
./dev-compose.sh down --volumes        # also wipe DB volumes + build/ artefacts
```

> **First-run timing.** The cold Rust compile is the slow part — count
> on ~5–15 minutes depending on your machine (it's downloading the
> crates.io tree and compiling the whole workspace once). Subsequent
> runs reuse the Cargo cache volume and finish in seconds.

---

## Prerequisites

You need **only Docker** for the compose path:

| Tool                | Min version | Install                                       |
| ------------------- | ----------- | --------------------------------------------- |
| Docker Engine       | 24+         | Docker Desktop (Mac/Win), OrbStack (Mac), or  |
|                     |             | distro package (Linux)                        |
| docker compose v2   | 2.20+       | bundled with Docker Desktop/OrbStack          |
| git                 | any         | xcode-select / apt / winget                   |

You do **NOT** need Rust, .NET, Node, or pnpm on the host. Every build
runs inside a builder container so a fresh laptop with only Docker can
spin the whole stack.

**Repo layout.** The frontend lives in a sibling checkout:

```text
cf/
├── insight/         (this repo)
└── insight-front/   (frontend repo)
```

If you keep them elsewhere, set `INSIGHT_FRONT_PATH` in `.env.compose`.

---

## Two dev paths

We are currently in transition. Both paths exist; **the compose path is
the preferred one going forward**.

| Path        | Driver               | Use it when                                          |
| ----------- | -------------------- | ---------------------------------------------------- |
| **compose** | `dev-compose.sh up`  | Day-to-day backend / frontend work. Default.        |
| k8s/helm    | `dev-up.sh` (Kind)   | Testing helm charts; ingestion (Airbyte) work;      |
|             |                      | anything that needs Argo Workflows or a real        |
|             |                      | cluster shape.                                       |

The compose path does **not** ship Airbyte or Argo Workflows — see
[Beyond compose](#beyond-compose) below.

---

## What's in the compose stack

```text
┌──────────────────────────────────────────────────────────────────────┐
│  Frontend (one of three modes — FRONTEND_MODE=dev|built|ghcr)        │
│  ┌─────────────────┐ ┌─────────────────┐ ┌──────────────────┐        │
│  │ Vite dev (HMR)  │ │ nginx + dist    │ │ ghcr pulled img  │        │
│  │ port 3000       │ │ port 3000       │ │ port 3000        │        │
│  └─────────────────┘ └─────────────────┘ └──────────────────┘        │
├──────────────────────────────────────────────────────────────────────┤
│  Backend                                                              │
│  ┌─────────────────┐ ┌─────────────────┐ ┌──────────────────┐        │
│  │ api-gateway     │ │ analytics-api   │ │ identity (.NET 9)│        │
│  │ Rust :8080      │ │ Rust :8081      │ │ :8082            │        │
│  └─────────────────┘ └─────────────────┘ └──────────────────┘        │
├──────────────────────────────────────────────────────────────────────┤
│  Infra                                                                │
│  ┌────────┐ ┌────────────┐ ┌───────┐ ┌──────────┐                    │
│  │MariaDB │ │ ClickHouse │ │ Redis │ │ Redpanda │                    │
│  │ :3306  │ │ :8123/:9000│ │ :6379 │ │ :19092…  │                    │
│  └────────┘ └────────────┘ └───────┘ └──────────┘                    │
└──────────────────────────────────────────────────────────────────────┘
```

Every web service publishes a host port — change them in `.env.compose`
if you have conflicts.

---

## Daily workflow

### Editing backend code

1. Edit Rust or C# source.
2. Rebuild the affected service:

   ```bash
   ./dev-compose.sh build api-gateway     # or analytics-api / identity / rust / all
   ```

3. The running container picks up the new binary automatically because
   `ENABLE_AUTO_RELOAD=true` wraps the process in `watchexec` (see
   below). You should see `[Running: ...]` in
   `docker compose logs -f api-gateway`.

### Editing service YAML (no rebuild)

The api-gateway's `config/` directory is bind-mounted from
`src/backend/services/api-gateway/config/` into the container. Edit
`no-auth.yaml` (or `insight.yaml`) on the host and watchexec restarts
the gateway in ~1 second — no rebuild, no compose bounce.

The .NET identity and Rust analytics-api take their config from
environment variables defined in `docker-compose.yml`; edit those and
`docker compose up -d <service>` to apply.

### Editing the frontend

- `FRONTEND_MODE=dev` (default): Vite is already watching — HMR delivers
  changes to the browser, no manual step.
- `FRONTEND_MODE=built`: run `./dev-compose.sh build frontend` after
  edits. nginx picks the new files up automatically.
- `FRONTEND_MODE=ghcr`: the published image is static — switch modes
  to pick up local changes.

### Switching frontend mode without bouncing everything

```bash
./dev-compose.sh down
FRONTEND_MODE=built ./dev-compose.sh up --skip-build   # if dist/ is fresh
```

---

## Auto-reload: how it works

Each backend container's `ENTRYPOINT` is the shared
`src/backend/docker-entrypoint.sh`. Its contract:

```text
docker-entrypoint.sh <watched-path> -- <command> [args...]
```

- If `ENABLE_AUTO_RELOAD` is **unset** (production default): the script
  just `exec`s the command. No watcher, no restart logic.
- If `ENABLE_AUTO_RELOAD=true` (set in `.env.compose`): the script
  wraps the command in `watchexec --restart --watch <watched-path>` so
  any change to the watched file (the bind-mounted binary) triggers
  SIGTERM and respawn.

**Important** — this is the **only** mechanism that restarts a process.
Per project policy: **never set `ENABLE_AUTO_RELOAD` in a k8s manifest**.
Compose-only.

watchexec actually watches the parent **directory** (`/app`) — modern
watchexec requires a directory, and `/app` only contains the binary +
config so there's no false-positive surface. When
`./dev-compose.sh build` writes a new binary to the bind-mounted path,
mtime changes, watchexec fires, the container's process restarts in
~1 second.

The watchexec binary in each image is the **musl static build** (not
glibc) — bookworm-slim ships glibc 2.36 and stock watchexec 2.3 binaries
want glibc 2.39, so we pin the `-unknown-linux-musl` variant. The
service Dockerfiles also call `useradd -m` so the `appuser` actually has
a usable `$HOME` — watchexec dies during config resolution if not.

---

## Backend image fallback (pull from ghcr instead of building locally)

Three ways to mark a backend service as "pull from ghcr":

1. Set the per-service image var in `.env.compose`:
   ```env
   API_GATEWAY_IMAGE=ghcr.io/constructorfabric/insight-api-gateway:latest
   ```
2. Pass it on the CLI:
   ```bash
   ./dev-compose.sh up --from-ghcr=api-gateway,identity
   ```
3. Invert via `--build-only` — everything not listed comes from ghcr:
   ```bash
   ./dev-compose.sh up --build-only=analytics-api
   ```

The script generates `compose/override.generated.yml` (gitignored) that
drops the `build:` and bind-mount for the chosen services so the
published image runs as-is.

---

## Frontend modes

`FRONTEND_MODE` in `.env.compose` (or `--frontend-mode=...` on CLI):

| Mode    | What runs                | Auto-reload?    | When to use                              |
| ------- | ------------------------ | --------------- | ---------------------------------------- |
| `dev`   | `pnpm dev` in node:24    | Vite HMR        | Default. Active frontend development.    |
| `built` | nginx + host-built dist  | No — rebuild   | Testing the production build path.       |
| `ghcr`  | `ghcr.io/...` image      | No              | Backend-only work, save laptop CPU/RAM.  |

In `built` mode, run `./dev-compose.sh build frontend` to refresh the
dist before bringing the stack up (or whenever you change source).

---

## Dev auth chain (no-auth mode)

When `AUTH_DISABLED=true` and `VITE_DEV_USER_EMAIL=you@yourorg.com`,
here is what actually carries identity from the browser to the
downstream service. Knowing the path saves time when something
401s unexpectedly.

```text
1. Browser  → fetch-with-auth.ts builds an unsigned JWT
              ({alg:"none"}.{email, sub, preferred_username}.) and sets
              Authorization: Bearer <jwt> on every request.
2. Vite     → proxies /api/* to api-gateway via the compose network.
3. Gateway  → auth_disabled=true skips JWT validation, but the proxy
              module forwards the Authorization header end-to-end.
4. Service  → identity's HeaderCallerContext falls back to JWT claims
              when X-Insight-Person-Id is absent: reads `email` /
              `sub` / `oid`, looks the value up in `persons`
              (value_type='email'), returns the matching person_id.
              Tenant comes from X-Insight-Tenant-Id, or
              IDENTITY__identity__tenant_default_id when absent.
```

So three things must all be true for a dev call to succeed:

* `VITE_DEV_USER_EMAIL` is set (FE builds the bearer token).
* A row in `persons` has `value_type='email'` and `value_id` matching
  that address (run `./dev-compose.sh seed identity`).
* The gateway proxies `/api/{prefix}` to the right upstream (see
  `no-auth.yaml`).

If you bypass the FE (curl from the host) you must construct the
same fake bearer yourself, otherwise identity returns
`401 caller_unresolved`.

## Dev impersonation + demo dataset

A fresh stack has an empty `identity.persons` table. The frontend
detects this and shows the "Dev impersonation not configured" hint
until you wire it up.

```bash
# 1. Set yourself as the dev lead in the demo roster.
echo 'VITE_DEV_USER_EMAIL=you@yourorg.com' >> .env.compose

# 2. Populate the demo dataset (idempotent).
./dev-compose.sh seed             # identity + silver — everything
# or, more selectively:
./dev-compose.sh seed identity    # just MariaDB: 25 persons + org chart + account map
./dev-compose.sh seed silver      # just ClickHouse: schema + gold views + ~24k rows

# 3. Restart the frontend container so it picks up VITE_DEV_USER_EMAIL.
docker compose -f docker-compose.yml --profile front-dev up -d insight-front-dev
```

After `seed identity` runs, the MariaDB has 25 persons:

* CEO (`email_ceo@company.nonpresent`) — apex of the org tree.
* Your `VITE_DEV_USER_EMAIL` person — leads the development team.
* 4 team leads (development = you, sales, HR, support).
* 20 ICs (5 per team, named `email_<team>_<NN>@company.nonpresent`).

Visibility is wired through the BambooHR org-chart source so the
gateway's per-caller `/v1/persons/{email}` lookups resolve correctly:
the dev lead sees their 5 direct reports; the CEO sees the whole tree.

After `seed silver` runs, ClickHouse has:

1. Bronze + silver placeholder tables (extracted from the k8s
   `create-bronze-placeholders.sh` workaround).
2. Every `src/ingestion/scripts/migrations/*.sql` applied to create the
   `insight.*` gold views the analytics-api reads.
3. ~24k rows across 16 silver tables, profile-typed per team (`class_git_*`
   only for devs, `class_crm_*` only for sales, etc.). The full
   per-team activity table lives in `compose/seed/profiles.py`.

After `seed silver` runs, the analytics-api's schema validator flips
from "80 metrics error: table_not_found" to "80 ok" and the FE
dashboards have data to show.

The script source is in `insight/compose/seed/` — its README explains
the ruff / mypy / venv setup.

## Common tasks

### Tail logs

```bash
docker compose logs -f api-gateway analytics-api identity
```

### Inspect databases

```bash
# MariaDB (identity, analytics schemas)
docker compose exec mariadb mariadb -uinsight -pinsight-local identity

# ClickHouse (insight db)
docker compose exec clickhouse clickhouse-client --user insight --password insight-local
```

### Wipe everything and start fresh

```bash
./dev-compose.sh down --volumes
./dev-compose.sh up
```

### Run a one-off Rust build with custom args

```bash
docker compose --profile build run --rm build-rust \
  cargo test -p insight-api-gateway
```

### Switch the gateway to real OIDC

Edit `src/backend/services/api-gateway/config/no-auth.yaml` directly
(it's bind-mounted into the container):

1. Set `api-gateway.config.auth_disabled: false`.
2. Fill in `oidc-authn-plugin.config.issuer_url`,
   `audience`, and the `auth-info` section's `client_id` / `scopes`.
3. Save. watchexec restarts the gateway in ~1 second.

No rebuild, no compose bounce. To revert, undo the edits.

---

## Beyond compose

The compose stack ships **9-ish services** but **does NOT include**:

- **Airbyte** — needs k8s. Use `./dev-up.sh ingestion`.
- **Argo Workflows** — k8s controller; same deal.
- **dbt scheduling** that depends on Argo Workflows.

To run these, install **one** of:

- OrbStack with Kubernetes (recommended on Mac)
- k3d (`brew install k3d`)
- kind (`brew install kind`)
- minikube (`brew install minikube`)

…and use the existing `dev-up.sh` path. The k8s and compose stacks can
coexist — they use disjoint host ports by default.

---

## Settings reference (`.env.compose`)

Read `.env.compose.example` end-to-end. The blocks are:

- **Auto-reload** — `ENABLE_AUTO_RELOAD`
- **Frontend mode** — `FRONTEND_MODE`, `INSIGHT_FRONT_PATH`,
  `FRONTEND_IMAGE`
- **Backend image overrides** — `API_GATEWAY_IMAGE`,
  `ANALYTICS_API_IMAGE`, `IDENTITY_IMAGE` (any unset → built locally)
- **Host ports** — every published port is configurable
- **DB credentials** — local-only; in dotenv per project convention
- **Tenant / OIDC** — `TENANT_DEFAULT_ID`, OIDC client info
- **Log level** — `RUST_LOG`

---

## Troubleshooting

**`docker compose up` says a bind-mount path doesn't exist.**
You probably skipped the build phase. Re-run `./dev-compose.sh up`
without `--skip-build`, or run `./dev-compose.sh build all` first.

**Container exits immediately with "exec format error".**
The bind-mounted binary is the wrong architecture (e.g. you built
natively on Mac and the container is Linux). Always build via
`./dev-compose.sh build` — never `cargo build` from the host shell.

**`watchexec: GLIBC_2.39 not found` or `No such file or directory`.**
The Dockerfile pins the musl static build of watchexec and creates a
home dir for `appuser`. If you see either error, your image is
out-of-date — force a rebuild: `docker compose build --no-cache <service>`.

**api-gateway exits with `oidc-authn-plugin: issuer_url is required`.**
The `no-auth.yaml` ships with a placeholder issuer
(`https://no-auth.local/oauth2/default`) because the plugin module is
registered in the binary even when `auth_disabled=true`. If you wiped
that line, restore it — the URL is never actually called.

**Frontend dev mode hangs at "pnpm install".**
First-run installs all deps into the named volume; can take several
minutes. Subsequent starts are fast. Tail with
`docker compose logs -f insight-front-dev`.

**Port already in use.**
Edit the relevant `*_PORT` in `.env.compose` and `./dev-compose.sh up`
again.

**I need Airbyte / Argo Workflows.**
See [Beyond compose](#beyond-compose). The compose stack will tell you
so explicitly if you pass `--start-airbyte` or `--start-argo`.

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
