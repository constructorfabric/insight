# Insight Frontend

Frontend application for **Insight** — a decision intelligence platform for engineering analytics, productivity insights, bottleneck detection, AI adoption tracking, and team health visibility.

Single-page application built on React 19 + TanStack Router + TanStack Query + shadcn/ui. Uses MSW for offline / demo mocking; talks to the Insight backend in production.

Part of the [Insight](https://github.com/constructorfabric/insight) monorepo — the backend, ingestion
and Helm charts live alongside this directory. Connector specs and API contracts are in
[insight-spec](https://github.com/constructorfabric/insight-spec).


## Tech Stack

| Layer | Technology |
|---|---|
| Routing | TanStack Router (file-based, auto-generated route tree) |
| Data | TanStack Query (per-query hooks under `src/queries/`) |
| Build | Vite 8 |
| Language | TypeScript 6 (strict) |
| Styling | Tailwind CSS 4 + shadcn/ui (`base-vega` style, CSS variables) |
| Charts | Recharts 3 |
| Auth | Server-side session (cookie/BFF through the gateway) |
| i18n | `i18next` + `react-i18next` (English only today) |
| Mocks | MSW (Mock Service Worker) |
| Linting | ESLint (flat config) |
| Package manager | pnpm 10 |
| Node | 24 (see `.nvmrc`) |

## Prerequisites

- Node.js 24 (`nvm use` picks up `.nvmrc`)
- pnpm 10+
- Docker (for container builds)

## Quick Start

```bash
git clone https://github.com/constructorfabric/insight.git
cd insight/src/frontend
pnpm install
pnpm dev
```

Open http://localhost:5173.

### Mock API

**Mocks are OFF by default** — `pnpm dev` talks to the Vite proxy (see `VITE_API_PROXY_TARGET`).

To enable synthetic data for an offline / demo session, copy `.env.example` to `.env.local` and set:

```
VITE_ENABLE_MOCKS=true
```

A yellow warning strip renders at the top of the page whenever mocks are active so synthetic values cannot be mistaken for real ones. Set `VITE_HIDE_MOCK_BANNER=true` to hide the strip during screenshots — mocks remain active. Prod builds (`pnpm build`) drop the mock subtree entirely.

Seeded mock people: `bob.park@example.com`, `carol.chen@example.com`, `alice.kim@example.com`, `frank.moss@example.com` (see [src/mocks/registry.ts](src/mocks/registry.ts)).

### Running against a deployed backend

`pnpm dev` proxies `/api` and `/auth` to `VITE_API_PROXY_TARGET` (default: the compose gateway). Pointing that at a deployed stand gets you its data, but **not** its login: the stand registers its OIDC `redirect_uri` on its own origin, so the IdP posts the code back to the stand and the dev server never receives a session.

Hand it a session instead:

1. Sign in to the stand in a browser.
2. Copy the `__Host-sid` value from DevTools → Application → Cookies (it is `HttpOnly`, so DevTools is the only way).
3. In `.env.local`:

```
VITE_API_PROXY_TARGET=https://<stand-host>
VITE_API_PROXY_SESSION=<the __Host-sid value>
```

The dev server holds the token and attaches it to every proxied request; the browser never stores it. `/auth/refresh` rotation is absorbed automatically, so the session stays alive as long as the SPA keeps refreshing it — no re-paste. Add `VITE_API_PROXY_INSECURE=true` for a self-signed upstream certificate.

Once the token is expired or revoked the SPA takes a `401` and heads for `/auth/login`, which the dev server answers with a `503` explaining how to refresh it rather than bouncing you onto the stand.

`VITE_API_PROXY_SESSION` is a live session on a real stand. `.env.local` is gitignored — keep it there.

## Scripts

| Script | Description |
|---|---|
| `pnpm dev` | Start Vite dev server |
| `pnpm build` | Production build (`tsc -b && vite build`) |
| `pnpm preview` | Serve production build locally |
| `pnpm typecheck` | TypeScript strict check (`tsc --noEmit`) |
| `pnpm lint` | ESLint (zero warnings) |
| `pnpm format` | Prettier write |

## Project Structure

```
src/
  auth/                  # Session probe + refresh driver, useAuth / useViewer hooks
  api/                   # Fetch clients (analytics, identity) + fetchWithAuth wrapper
  queries/               # React Query hooks (metric-results, member-grid, metric-definitions)
  routes/                # TanStack Router file-based routes (auto-discovered)
  routeTree.gen.ts       #   ← auto-generated, do not edit
  screens/               # Page components composed by routes
  components/
    ui/                  #   shadcn/ui primitives (button, card, dialog, alert, …)
    widgets/             #   Feature widgets (dashboard/, metric-views/, …)
    app-sidebar.tsx      #   Org-tree sidebar (recursive nav)
    theme-provider.tsx   #   Light/dark/system theme (localStorage-backed)
    mock-banner.tsx      #   Warning strip when mocks are on
    app-error-boundary.tsx
    error-fallback.tsx
  hooks/                 # Shared hooks (use-period, use-mobile)
  lib/                   # Domain helpers (format, status, scoring, peers, …)
  mocks/                 # MSW handlers, factories, registry (dev-only, tree-shaken in prod)
  locales/en/            # i18next translation files
  i18n/                  # i18next setup
  types/                 # Shared TypeScript types
  index.css              # Tailwind v4 inline config + theme tokens (light + dark)
  main.tsx               # Entry: consumeOverrideParam → enableMocking → loadSession → render
  router.ts              # createRouter(routeTree)
```

## Authentication

Server-side cookie/BFF flow — the SPA holds no tokens.

1. The browser hits `/auth/login`; the gateway and authenticator run the provider handshake and set a `__Host-sid` session cookie.
2. [src/main.tsx](src/main.tsx) probes `/auth/me` via `loadSession()` before the router mounts, so the root route reads a resolved auth store.
3. [src/api/fetch-with-auth.ts](src/api/fetch-with-auth.ts) sends `credentials: "include"` on every request — no `Authorization` header, no tenant header. The gateway injects the downstream JWT.
4. A 401 bounces the whole page into `/auth/login?return_to=…` ([src/auth/use-auth.ts](src/auth/use-auth.ts)); there is no client-side token to refresh.
5. The session is non-sliding: [src/auth/refresh.ts](src/auth/refresh.ts) drives `POST /auth/refresh` on the server-supplied `refresh_at`.

## Environment Variables

Build-time (Vite, `.env.local`):

| Variable | Description |
|---|---|
| `VITE_ENABLE_MOCKS` | `"true"` to enable MSW (dev only; stripped from prod). |
| `VITE_HIDE_MOCK_BANNER` | `"true"` to hide the warning strip while mocks are on (for screenshots). |
| `VITE_API_PROXY_TARGET` | Dev-only `/api` proxy target (e.g. `http://localhost:8080`). |
| `VITE_API_BASE` | Override analytics API base URL (default `/api/analytics/v1`). |
| `VITE_IDENTITY_BASE` | Override identity API base URL (default `/api/identity/v1`). |
| `VITE_SENTRY_DSN` | Sentry DSN for local runs only; a deployed stand uses `sentry.dsn`. Unset means Sentry never initializes. |
| `VITE_APP_RELEASE` | Release attached to events. Defaults to `local-<git sha>`; CI passes the image tag. |

## Error Reporting and Tracing

[src/sentry.ts](src/sentry.ts) initializes Sentry as the first statement in
[src/main.tsx](src/main.tsx). The module imports above it — i18n, the query
client, the router — have already run by then, so a throw while they initialize
goes unreported.

What leaves the browser:

- Render errors, via `onError` on [app-error-boundary.tsx](src/components/app-error-boundary.tsx).
- Unhandled exceptions and rejections, from the SDK's own handlers.
- Performance transactions for 10% of page loads and navigations. Browser
  tracing also instruments `fetch`/XHR and adds `sentry-trace` and `baggage`
  headers to same-origin requests.

Events carry the hostname as their `environment` (`local` on localhost) — one
image serves every stand, so nothing else tells them apart.

The image carries no DSN. A deployed stand takes two chart values, and they
must agree:

1. `sentry.dsn` — rendered into a ConfigMap that mounts over `/config.js`,
   which the SPA reads before it boots.
2. `sentry.connectSrc`, set to the same origin. The container's CSP is
   rendered from it at start; without it the browser blocks every event and
   nothing arrives.

Locally there is no chart, so `VITE_SENTRY_DSN` stands in for the first and
the CSP does not apply.

The SDK attaches no cookies, IP or headers to events. Session Replay is not
enabled.

The build emits no sourcemaps and uploads none, so a stack trace names the
minified bundle rather than your code.

## Routes

| Path | Screen | Notes |
|---|---|---|
| `/` | Dashboard for the signed-in viewer | Resolves the viewer from the session. |
| `/ic/$person` | (redirects to `/ic/$person/personal`) | |
| `/ic/$person/personal` | Dashboard | KPI row, attention list, metric group cards + drilldowns. |
| `/ic/$person/team` | Team view | Members heatmap, attention list, metric group drilldowns. |
| `/metrics` | Metric catalog | Metric definitions browser. |
| `/whats-new` | Release notes | |

## Theming

`light` / `dark` / `system`. Theme tokens are CSS variables defined in [src/index.css](src/index.css); use the semantic Tailwind utilities (`bg-background`, `text-muted-foreground`, `border-border`, `text-destructive`, `bg-warning/10`, etc.). The shadcn theme is `base-vega` with `cssVariables: true` (see [components.json](components.json)).

## i18n

`i18next` + `react-i18next`. English-only today (`supportedLngs: ["en"]`). Translations live in [src/locales/en/translation.json](src/locales/en/translation.json); component code uses `const { t } = useTranslation()` + `t("key")`.

## Docker

### Build

```bash
docker build -t insight-frontend:local .
```

### Run without a backend (mock mode)

```bash
VITE_ENABLE_MOCKS=true pnpm build
docker run -d -p 8080:80 insight-frontend:local
```

All screens render synthetic data and the warning strip stays visible.

### Docker Compose

```bash
cp docker-compose.yml docker-compose.override.yml
docker compose up -d --build
```

### With Insight Backend (Kind cluster)

From the repository root:

```bash
./up.sh frontend    # builds image + deploys to Kind via Helm
./up.sh app         # backend + frontend together
./up.sh             # full stack (ingestion + backend + frontend)
```

## License

See [LICENSE](LICENSE) and [NOTICE](NOTICE).
