# Contributing to Insight

This document covers how to get a working dev environment, the two
deployment paths (compose vs. k8s/helm), the daily edit-build-run loop,
and a few sharp edges to know about.

Open a PR after reading [AGENTS.md](AGENTS.md) and the relevant
spec files under `docs/components/<area>/specs/`.

---

## TL;DR — happy path, all defaults

Clone, run one command, answer four prompts, get a fully populated
stack:

```bash
git clone git@github.com:cyberantonz/insight.git
cd insight
./dev-compose.sh up
```

The first `up` runs an interactive wizard because `.env.compose`
doesn't exist yet. With defaults accepted everywhere, the answers are:

| Prompt                                | Default | Effect                                  |
| ------------------------------------- | ------- | --------------------------------------- |
| Use local MariaDB in docker compose?  | Y       | Compose starts mariadb on :3306         |
| Use local ClickHouse in docker compose? | Y     | Compose starts clickhouse on :8123      |
| `VITE_DEV_USER_EMAIL`                 | `dev@company.nonpresent` | Dev-team lead in the seed roster |
| Frontend choice                       | `1` (ghcr) | Pulls the published `insight-front:latest` image |

The wizard writes `.env.compose`, then the script:

1. Builds host artefacts (Rust + .NET; frontend is pulled if you picked
   ghcr). First run: 5–15 minutes (cold Rust compile).
2. Brings every service up (`docker compose up -d`).
3. Auto-seeds the demo dataset — 25 persons in MariaDB + ~24k rows
   across 16 ClickHouse silver tables.
4. Flips `SEEDED_LOCAL_MARIA` / `SEEDED_LOCAL_CH` in `.env.compose` so
   later `up` calls don't re-seed.

Open <http://localhost:3000>. The dashboards have data;
`dev@company.nonpresent` is the dev-team lead, CEO sees the whole org
tree, every team has 60 days of activity.

Daily workflow:

```bash
./dev-compose.sh build api-gateway     # rebuild one Rust service after a code edit
./dev-compose.sh build all             # rebuild everything
./dev-compose.sh seed silver           # refresh ClickHouse demo data only
./dev-compose.sh down                  # stop everything; data preserved
./dev-compose.sh down --volumes        # also wipe DB volumes + build/ artefacts
./dev-compose.sh prune                 # destructive wipe + remove .env.compose
```

> **First-run timing.** The cold Rust compile is the slow part — count
> on ~5–15 minutes depending on your machine (it's downloading the
> crates.io tree and compiling the whole workspace once). Subsequent
> runs reuse the Cargo cache volume and finish in seconds.

> **Re-running the wizard.** Delete `.env.compose` (or run
> `./dev-compose.sh prune`) and `up` again. Or hand-edit
> `.env.compose` — the wizard only runs when the file is missing.

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

## Using external MariaDB / ClickHouse

The wizard's defaults run both DBs in compose. If you already have a
populated MariaDB or ClickHouse elsewhere (shared dev box, staging
mirror, your own host install), answer **N** to the relevant wizard
prompt and you'll be asked for connection details:

```
Use the local MariaDB in docker compose? [Y/n]: n
  External MariaDB host: db.internal.example
  External MariaDB port [3306]:
  MariaDB user [insight]:
  MariaDB password:        ← hidden
  Probing MariaDB at db.internal.example:3306…
  MariaDB OK.
```

The wizard validates credentials before writing `.env.compose`:

- **MariaDB** is probed by spinning up a transient `mariadb:11.4`
  container and running `SELECT 1`. A bad host/user/password aborts
  the wizard with a clear error.
- **ClickHouse** is probed via the HTTP interface using host-side
  `curl`. Same fail-fast behavior.

When at least one DB is external, the wizard also asks for:

- **`TENANT_DEFAULT_ID`** — the UUID present in your
  `persons.insight_tenant_id`. Required because the canned demo UUID
  won't match your data.
- **Seed your external DB?** — defaults to **No**. If you accept,
  `./dev-compose.sh up` writes the demo dataset into your DB on first
  run (same content as the local-DB auto-seed). If you decline, the
  wizard pre-marks the DB as seeded so `up` leaves it alone.

What the wizard sets in `.env.compose`:

```env
MARIADB_EXTERNAL=true                 # don't start local-mariadb profile
MARIADB_HOST=db.internal.example      # used inside backend containers
MARIADB_INTERNAL_PORT=3306            # connect port, NOT the host-mapped one
MARIADB_USER=insight
MARIADB_PASSWORD=…
CLICKHOUSE_EXTERNAL=true
CLICKHOUSE_HOST=ch.internal.example
CLICKHOUSE_INTERNAL_HTTP_PORT=8123
CLICKHOUSE_DATABASE=insight
CLICKHOUSE_USER=insight
CLICKHOUSE_PASSWORD=…
TENANT_DEFAULT_ID=11111111-2222-3333-4444-555555555555
```

How it's wired:

- Backend services interpolate `${MARIADB_HOST}:${MARIADB_INTERNAL_PORT}`
  (and the ClickHouse equivalents) into their connection URLs. Defaults
  preserve the local docker behavior (`mariadb:3306`, `clickhouse:8123`).
- The `mariadb` and `clickhouse` services sit behind
  `profiles: ["local-mariadb"]` / `local-clickhouse`. `dev-compose.sh
  up` adds those profiles only when `*_EXTERNAL != true`.
- Backend `depends_on` entries use `required: false`, so compose skips
  the dependency when the profile isn't active.

> **`localhost` gotcha.** If your "external" DB is actually running on
> the docker host, **don't** type `localhost` — that resolves to the
> container itself. Use `host.docker.internal` (Mac/Windows) or your
> LAN IP. The wizard warns you when it sees `localhost`.

To switch later (e.g. start using a local DB after pointing at an
external one), the easiest path is `./dev-compose.sh prune` and re-run
the wizard. Or hand-edit `*_EXTERNAL` / `*_HOST` / `*_INTERNAL_PORT`
in `.env.compose` and `./dev-compose.sh down && ./dev-compose.sh up`.

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

- `FRONTEND_MODE=dev`: Vite is already watching — HMR delivers
  changes to the browser, no manual step.
- `FRONTEND_MODE=built`: run `./dev-compose.sh build frontend` after
  edits. nginx picks the new files up automatically.
- `FRONTEND_MODE=ghcr`: the published image is static — switch modes
  to pick up local changes.

See [Frontend modes](#frontend-modes) for how to switch between them
after the first-run wizard.

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

The wizard asks one question with three explicit choices:

```
--- Frontend ---
  How should the frontend run?
    1) ghcr   — pull the pre-built image (no source needed)
    2) local  — Vite + HMR against an existing insight-front checkout
    3) clone  — git clone insight-front, then run Vite + HMR
```

| Choice | Wizard does                                  | What runs                | Auto-reload? | When to use                              |
| ------ | -------------------------------------------- | ------------------------ | ------------ | ---------------------------------------- |
| 1 ghcr | Sets `FRONTEND_MODE=ghcr`                    | `ghcr.io/...` image      | No           | Backend-only work, save laptop CPU/RAM.  |
| 2 local| Sets `FRONTEND_MODE=dev` + `INSIGHT_FRONT_PATH`. Path must already exist. | `pnpm dev` in node:24    | Vite HMR     | Active frontend development on an existing checkout. |
| 3 clone| `git clone constructorfabric/insight-front` into the path you pick (refuses to clobber an existing dir), then same as local. | `pnpm dev` in node:24    | Vite HMR     | First-time setup, no checkout yet.       |

There's also a fourth, undocumented-in-wizard `built` mode (nginx +
host-built dist). To use it, hand-edit `.env.compose`:

```env
FRONTEND_MODE=built
```

…then `./dev-compose.sh build frontend` (refreshes `dist/`) and
`./dev-compose.sh down && ./dev-compose.sh up`. Useful for testing
the production build path.

### Switching modes after first run

The wizard runs only once. To change FE mode later, either:

- **Edit `.env.compose`** — flip `FRONTEND_MODE` to `dev`/`built`/`ghcr`,
  set `INSIGHT_FRONT_PATH` if switching to `dev`/`built`, then
  `./dev-compose.sh down && ./dev-compose.sh up --skip-build` (the
  `--skip-build` keeps an already-compiled Rust binary in place).
- **Or override per-run** without touching the file:

  ```bash
  ./dev-compose.sh up --frontend-mode=ghcr --skip-build
  ./dev-compose.sh up --no-frontend                # backend-only
  ```

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

## Seeding + building

### Auto-seed on first `up`

The first successful `./dev-compose.sh up` after the wizard
automatically populates the demo dataset, then writes
`SEEDED_LOCAL_MARIA=true` / `SEEDED_LOCAL_CH=true` into `.env.compose`
so subsequent `up` calls skip the seed step. You don't need to do
anything extra for the happy path.

For external DBs the wizard asks whether to seed; if you decline, the
two markers are pre-set to `true` and the stack leaves your DB alone.

### Manual seed / re-seed

`./dev-compose.sh seed` always runs regardless of `SEEDED_LOCAL_*`
state:

```bash
./dev-compose.sh seed            # identity + silver — everything
./dev-compose.sh seed identity   # just MariaDB: 25 persons + org chart + account map
./dev-compose.sh seed silver     # just ClickHouse: schema + gold views + ~24k rows
```

To force the next `up` to auto-seed again, set
`SEEDED_LOCAL_MARIA=` / `SEEDED_LOCAL_CH=` to empty in `.env.compose`
(or just run `./dev-compose.sh prune` and start fresh).

### What gets seeded

After **identity** runs, MariaDB has 25 persons:

* CEO (`email_ceo@company.nonpresent`) — apex of the org tree.
* Your `VITE_DEV_USER_EMAIL` person — leads the development team.
  Default `dev@company.nonpresent`; change it in the wizard or
  hand-edit before re-seeding to switch identities.
* 4 team leads (development = you, sales, HR, support).
* 20 ICs (5 per team, named `email_<team>_<NN>@company.nonpresent`).

Visibility is wired through the BambooHR org-chart source so the
gateway's per-caller `/v1/persons/{email}` lookups resolve correctly:
the dev lead sees their 5 direct reports; the CEO sees the whole tree.

After **silver** runs, ClickHouse has:

1. Bronze + silver placeholder tables (extracted from the k8s
   `create-bronze-placeholders.sh` workaround).
2. Every `src/ingestion/scripts/migrations/*.sql` applied to create the
   `insight.*` gold views the analytics-api reads.
3. ~24k rows across 16 silver tables, profile-typed per team (`class_git_*`
   only for devs, `class_crm_*` only for sales, etc.). The full
   per-team activity table lives in `compose/seed/profiles.py`.

The analytics-api's schema validator flips from "80 metrics error:
table_not_found" to "80 ok" and the FE dashboards have data to show.

The script source is in `insight/compose/seed/` — its README explains
the ruff / mypy / venv setup.

### Building

`./dev-compose.sh up` runs the build phase by default. The targets:

```bash
./dev-compose.sh build api-gateway     # Rust gateway only
./dev-compose.sh build analytics-api   # Rust analytics only
./dev-compose.sh build identity        # .NET 9 publish
./dev-compose.sh build frontend        # pnpm build → dist/
./dev-compose.sh build rust            # both Rust services
./dev-compose.sh build all             # everything
```

Skip the build when you just want to bounce the stack:

```bash
./dev-compose.sh up --skip-build
```

For one-off cargo work without building the binary into compose/build/:

```bash
docker compose --profile build run --rm build-rust \
  cargo test -p insight-api-gateway
```

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

Three escalating levels of destructive:

```bash
./dev-compose.sh down              # stop containers, keep volumes and .env.compose
./dev-compose.sh down --volumes    # also wipe named volumes + compose/build/
./dev-compose.sh prune             # the full nuke (see below)
```

### Prune — full reset including .env.compose

`./dev-compose.sh prune` is the only command that removes
`.env.compose`. Use it when you want the next `up` to re-run the
first-run wizard:

```bash
./dev-compose.sh prune
```

It's always interactive — there is no `--yes` switch. The flow:

1. Lists what's about to be destroyed (containers, named volumes,
   `compose/build/`, generated override, `.env.compose`) and asks
   `Proceed? [y/N]`.
2. Runs `docker compose down --volumes --remove-orphans` against every
   profile (so even services not currently active are cleaned up).
3. Removes `compose/build/`, `compose/override.generated.yml`, and
   `.env.compose`.
4. Asks **separately** whether to also remove pulled
   `ghcr.io/constructorfabric/insight-*` images — defaults to **No**
   because they're slow to re-pull and most resets don't need to throw
   them away.

After prune, the next `./dev-compose.sh up` re-runs the wizard.

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
- **Database mode** — `MARIADB_EXTERNAL`, `MARIADB_HOST`,
  `MARIADB_INTERNAL_PORT`, `CLICKHOUSE_EXTERNAL`, `CLICKHOUSE_HOST`,
  `CLICKHOUSE_INTERNAL_HTTP_PORT`, `CLICKHOUSE_DATABASE`. See
  [Using external MariaDB / ClickHouse](#using-external-mariadb--clickhouse).
- **DB credentials** — local-only by convention; in dotenv per project
  policy
- **Seed bookkeeping** — `SEEDED_LOCAL_MARIA`, `SEEDED_LOCAL_CH`
  (empty/false → auto-seed on next `up`; true → skip)
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
