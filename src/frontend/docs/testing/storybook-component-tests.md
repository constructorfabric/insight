# Storybook + component tests in Insight Frontend

> Status: **PoC implemented** (scaffold + one example test). The setup is wired
> end-to-end; a sample `KpiTile` test is green in a real browser.

## 1. Context and decision

We want component tests that run **against a real browser** rather than jsdom, so
that layout, CSS, and real DOM behaviour are exercised. Storybook stories are a
natural fit: a story is an isolated render of a component, and its `play`
function is a place to put interaction assertions.

### Decision

1. **Use public packages only.** Insight is a public, Apache-2.0 repository
   (`github.com/constructorfabric/insight-front`). Every test dependency must be
   installable from the public npm registry so `pnpm install` works for anyone,
   including external contributors and public CI.
2. **Run stories as tests in a real browser** via `@storybook/addon-vitest` +
   `@vitest/browser-playwright` (headless chromium). `play` functions are the
   tests.
3. **Reuse Insight's own provider stack** (QueryClient, TanStack Router, theme,
   i18n) and the existing `src/mocks/` MSW handlers, rather than pulling in any
   framework-specific test harness.

## 2. Building blocks

| Layer | What it is |
|---|---|
| Storybook config (`.storybook/main.ts`) | Stories glob, addons, static dirs |
| Browser test project (`vitest.config.ts`) | `storybookTest` plugin + Playwright chromium |
| Theme preview decorator | `withThemeByClassName` mapped to our `light`/`dark` classes |
| Manager UI (`.storybook/manager.ts`) | Storybook UI theme |
| Memory router for stories | Plain TanStack Router with memory history |
| Provider wrapper (`WithProviders`) | QueryClient + router + theme + i18n around each story |
| MSW in stories | Reuses `src/mocks/`; handlers can be overridden per story |

Stories mount a component in isolation, so the router does not use the app's
generated `routeTree.gen` — a plain **TanStack Router with memory history** is
enough to provide `<Link>` / `useNavigate` / router context.

## 3. File layout (as implemented)

```
cyber-insight-front/
├─ .storybook/
│   ├─ main.ts                 # stories, addons, staticDirs
│   ├─ preview.tsx             # global decorators (theme + WithProviders), MSW init, global beforeEach reset
│   └─ manager.ts              # Storybook UI theme (dark)
├─ src/test/storybook/
│   ├─ test-router.ts          # createTestingRouter() — memory-history TanStack Router
│   ├─ with-providers.tsx      # QueryClient + router + theme + i18n decorator
│   └─ theme-decorators.ts     # addon-themes adapted to light/dark
├─ vitest.config.ts            # test.projects = [unit (jsdom), storybook (browser)]
├─ public/mockServiceWorker.js # already present (msw workerDirectory: public)
└─ src/components/widgets/dashboard/kpi-tile.stories.tsx  # PoC story + play tests
```

> Existing `*.test.tsx` (RTL/jsdom) are untouched and run in the `unit` project.
> Storybook tests are added as a second, independent `storybook` project.
>
> Note: `@storybook/addon-vitest` ≥ Storybook 10.3 auto-applies preview
> annotations, so **no `.storybook/vitest.setup.ts` / `setProjectAnnotations` is
> needed** — see §13.

## 4. Dependencies (devDependencies, public)

Versions align with our `vitest@4.1.7`, `vite@8`, `react@19.2.6`.

```jsonc
{
  "devDependencies": {
    "storybook": "10.3.5",
    "@storybook/react-vite": "10.3.5",
    "@storybook/addon-vitest": "10.3.5",
    "@storybook/addon-themes": "10.3.5",
    "@storybook/addon-a11y": "10.3.5",
    "@storybook/addon-docs": "10.3.5",
    "@vitest/browser-playwright": "4.1.7", // aligned with vitest
    "playwright": "1.57.0",                 // browser binaries for browser-mode
    "msw-storybook-addon": "2.0.7"          // 2.0.7, not 2.0.6 — see §13
  }
}
```

One-time browser install: `pnpm exec playwright install chromium`.

**Run on Node 24** (`.nvmrc`): vite 8 / Storybook 10 require Node ≥ 20.19;
`nvm use` picks up 24.

## 5. Configuration

### 5.1 `.storybook/main.ts`

```ts
import type { StorybookConfig } from "@storybook/react-vite";

const config: StorybookConfig = {
  stories: ["../src/**/*.stories.@(ts|tsx)"],
  addons: [
    "@storybook/addon-themes",
    "@storybook/addon-vitest",
    "@storybook/addon-a11y",
    "@storybook/addon-docs",
  ],
  framework: { name: "@storybook/react-vite", options: {} },
  core: { disableTelemetry: true, enableCrashReports: false },
  typescript: { reactDocgen: "react-docgen-typescript" },
  staticDirs: ["../public"], // serves public/mockServiceWorker.js
};

export default config;
```

### 5.2 `vitest.config.ts` (two projects)

We keep a dedicated `vitest.config.ts` (so test deps don't leak into the prod
`vite.config.ts`):

```ts
import path from "path";
import react from "@vitejs/plugin-react";
import { playwright } from "@vitest/browser-playwright";
import { storybookTest } from "@storybook/addon-vitest/vitest-plugin";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  test: {
    projects: [
      // project 1: existing RTL/jsdom unit & component tests (unchanged)
      {
        extends: true,
        test: {
          name: "unit",
          environment: "jsdom",
          globals: false,
          css: false,
          setupFiles: ["./src/test/setup.ts"],
          include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
        },
      },
      // project 2: stories tagged `test` run as tests in a real browser
      {
        extends: true,
        plugins: [
          storybookTest({
            configDir: path.resolve(__dirname, ".storybook"),
            tags: { include: ["test"], exclude: [], skip: ["skip-test"] },
          }),
        ],
        test: {
          name: "storybook",
          browser: {
            enabled: true,
            headless: true,
            provider: playwright(),
            instances: [{ browser: "chromium" }],
          },
          // No setupFiles: addon-vitest auto-applies .storybook/preview.tsx (≥10.3).
        },
      },
    ],
  },
});
```

### 5.3 `.storybook/manager.ts`

```ts
import { addons } from "storybook/manager-api";
import { themes } from "storybook/theming";

addons.setConfig({ theme: themes.dark });
```

### 5.4 `package.json` scripts

```jsonc
{
  "scripts": {
    "storybook:dev": "storybook dev -p 6006",
    "storybook:build": "storybook build",
    "test": "vitest run --project=unit",          // keep current CI behaviour
    "test:watch": "vitest --project=unit",
    "test:storybook": "vitest --project=storybook",
    "test:storybook:ci": "vitest run --project=storybook"
  }
}
```

## 6. The `WithProviders` decorator

This is the core piece: it mounts a story inside Insight's own provider stack
from `src/main.tsx` (QueryClient + memory router + theme + i18n).

`src/test/storybook/test-router.ts`:

```ts
import {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  Outlet,
} from "@tanstack/react-router";

export function createTestingRouter() {
  const rootRoute = createRootRoute({ component: Outlet });
  const history = createMemoryHistory({ initialEntries: ["/"] });
  const router = createRouter({ routeTree: rootRoute, history });
  return { router, history, reset: () => history.replace("/") };
}
```

`src/test/storybook/with-providers.tsx`:

```tsx
import type { Decorator } from "@storybook/react-vite";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { I18nextProvider } from "react-i18next";
import { useEffect, useMemo } from "react";

import i18n from "@/i18n";
import { ThemeProvider } from "@/components/theme-provider";
import { createTestingRouter } from "./test-router";

export const WithProviders: Decorator = (Story) => {
  // Fresh QueryClient + router per story (isolation).
  const client = useMemo(
    () => new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } }),
    [],
  );
  const { router } = useMemo(() => createTestingRouter(), []);

  // Mount the story as the memory-router's not-found component so it gets full
  // router context.
  useEffect(() => {
    router.update({ defaultNotFoundComponent: () => <Story /> });
  }, [router, Story]);

  return (
    <QueryClientProvider client={client}>
      <ThemeProvider>
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
        </I18nextProvider>
      </ThemeProvider>
    </QueryClientProvider>
  );
};
```

> `CatalogProvider` / `AppErrorBoundary` from `main.tsx` are intentionally NOT in
> the shared decorator: catalog-driven widgets mock `POST /catalog/get_metrics`
> via MSW per story, so a story exercises the real `useCatalog()` wiring instead
> of a stubbed provider.

## 7. Theme decorator

Our `ThemeProvider` sets `light`/`dark` classes on `<html>` (Tailwind / shadcn
convention, `src/components/theme-provider.tsx`):

```ts
// src/test/storybook/theme-decorators.ts
import { withThemeByClassName } from "@storybook/addon-themes";
import type { Decorator } from "@storybook/react-vite";

export const themeDecorators: Decorator[] = [
  withThemeByClassName({
    themes: { light: "light", dark: "dark" },
    defaultTheme: "light",
  }),
];
```

## 8. `.storybook/preview.tsx` (decorators + MSW + global reset)

```tsx
import type { Preview } from "@storybook/react-vite";
import { initialize, mswLoader } from "msw-storybook-addon";

import "@/index.css"; // Tailwind v4 + theme tokens
import { authStore } from "@/auth/auth-store";
import { themeDecorators } from "@/test/storybook/theme-decorators";
import { WithProviders } from "@/test/storybook/with-providers";

initialize({ onUnhandledRequest: "bypass" });

const preview: Preview = {
  parameters: {
    layout: "centered",
    docs: { stories: { filter: (s: { name: string }) => !s.name.startsWith("Test") } },
  },
  // Reset app-level singletons before every story. QueryClient/router are
  // already fresh (WithProviders), MSW handlers are reset by the addon; this
  // covers module singletons (auth) and persisted UI prefs (theme / locale /
  // settings in localStorage). A story needing a tenant sets it in its own
  // beforeEach (runs after this one).
  beforeEach: () => {
    authStore.reset();
    try {
      window.localStorage.clear();
    } catch {
      /* localStorage unavailable */
    }
  },
  loaders: [mswLoader],
  decorators: [...themeDecorators, WithProviders],
};

export default preview;
```

> The MSW worker (`public/mockServiceWorker.js`) is already generated
> (`"msw": { "workerDirectory": ["public"] }` in `package.json`). Handlers come
> from the existing `src/mocks/` or are defined per story (see §9).

## 9. Example story + play test (`KpiTile`)

Covers the same rules as the existing `kpi-tile.test.tsx`, but as browser tests.
The catalog is mocked via MSW (`/api/analytics/v1/catalog/get_metrics`).

```tsx
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, waitFor } from "storybook/test";

import { KpiTile } from "./kpi-tile";

const meta: Meta<typeof KpiTile> = {
  title: "Widgets/Dashboard/KpiTile",
  component: KpiTile,
  beforeEach: () => { authStore.setTenantId("t-1"); }, // global beforeEach handles cleanup
};
export default meta;
type Story = StoryObj<typeof KpiTile>;

// Demo story for the Storybook UI. Untagged, so it is NOT run as a test (see §13).
export const Default: Story = { args: { kpi: /* ... */ } };

// Component test: opt in with the `test` tag. Ok catalog row drives the
// peer-median label.
export const TestOkRow: Story = {
  tags: ["test"],
  args: { kpi: /* ... peer_median 6 ... */ },
  parameters: {
    msw: { handlers: [http.post("/api/analytics/v1/catalog/get_metrics",
      () => HttpResponse.json(okCatalog))] },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Bugs Fixed")).toBeInTheDocument();
    await waitFor(() => expect(canvas.getByText(/median 6/i)).toBeInTheDocument());
  },
};
```

Pattern: a story becomes a test by carrying the `test` tag (see §13); `play(...)`
is the test body, `expect`/`waitFor`/`userEvent` come from `storybook/test`, and
assertions run against `canvas`.

## 10. Migration of existing tests

- Current `src/**/*.test.tsx` (RTL/jsdom, `vi.mock`) — **untouched**, run in `unit`.
- `pnpm test` stays scoped to `unit` (CI behaviour unchanged).
- New component tests are written story-style (`storybook` project).
- Over time, visual / interaction cases move to stories; pure-logic tests
  (`lib/`, transforms) stay as plain vitest units.

## 11. CI

Implemented in `.github/workflows/ci.yml` (the `test` job):

- The job runs **both** Vitest projects in one pass via `pnpm test:coverage:ci`
  (`vitest run --coverage`, no `--project` filter). Coverage is a global option,
  so the jsdom `unit` and browser `storybook` projects merge into a single
  Cobertura report — a component exercised only by a story counts toward the
  diff-coverage gate. The existing `coverage` job (summary + 80% diff gate)
  consumes that report unchanged.
- Browser-mode needs headless chromium: the job caches `~/.cache/ms-playwright`
  (keyed on `pnpm-lock.yaml`, where the Playwright version is pinned) and runs
  `pnpm exec playwright install --with-deps chromium` (a cache hit skips the
  download and only re-runs the fast apt-get OS-deps step).
- Component tests are **blocking**: a failing story fails the `test` job exactly
  like a unit test does.
- `pnpm test:coverage` stays unit-only for a fast local loop that needs no
  browser; `pnpm test:coverage:ci` is what CI runs.
- Prod build (`pnpm build`) is unaffected — Storybook/test deps are dev-only.

## 12. Rollout phases

1. **Phase 0 — scaffold (done):** deps, `.storybook/*`, `src/test/storybook/*`,
   `storybook` project in `vitest.config.ts`, scripts.
2. **Phase 1 — first widget (done):** stories + play tests for `KpiTile`; green
   `test:storybook`. MSW catalog-mock pattern established.
3. **Phase 2 — CI (done):** the `test` job caches + installs chromium and runs
   `pnpm test:coverage:ci` (both projects, merged coverage). Story tests are
   blocking and feed the diff-coverage gate. See §11.
4. **Phase 3 — rollout:** stories for key widgets (`bullet-chart`,
   `members-heatmap`, `attention-needed`, `metric-card`, `members-table`).

## 13. Notes and decisions

- **Tag model: a story runs as a test only when it opts in with `tags: ["test"]`.**
  This takes two coordinated settings, because Storybook auto-applies the `test`
  tag to *every* story by default:
  1. `.storybook/preview.tsx` sets `tags: ["!test"]`, which **strips** that
     auto-applied `test` tag from all stories.
  2. `vitest.config.ts` sets `tags: { include: ["test"] }`, so the `storybook`
     project runs only stories that (re-)carry the `test` tag.

  A story opts in by adding `tags: ["test"]` (a tagged story with no `play` is a
  smoke test; one with `play` adds assertions). `Default`-style demo stories stay
  untagged and are **not** run as tests. Without step 1, `include: ["test"]`
  would run *every* story, demo stories included. Verify what actually runs with
  `vitest list --project=storybook`.
- **`setProjectAnnotations` is deliberately NOT used.** Since Storybook 10.3,
  `@storybook/addon-vitest` auto-applies preview annotations from `configDir`.
  Adding a `.storybook/vitest.setup.ts` with `setProjectAnnotations([preview])`
  would **double-apply decorators** (a known footgun — the plugin even logs a
  warning and skips its own provisioning when it detects the call). Verified:
  the singular `canvas.getByText(...)` queries in the PoC pass, which would throw
  on a doubled render — so decorators apply exactly once.
- **`msw-storybook-addon` 2.0.6 → 2.0.7.** 2.0.6's `initialize()` writes to
  `worker.context.activationPromise`, but in our `msw@2.14.6` `worker.context`
  is `undefined`. 2.0.7 stores the promise in a module variable and works.
- **`tanstackRouter` plugin in `vite.config.ts`** (autoCodeSplitting, scans
  `src/routes`) is picked up by Storybook. Both `test:storybook` and
  `storybook build` are green, so it does not interfere. Stories use the
  memory router, not `routeTree.gen`.
- **`kpi-tile.test.tsx` (RTL) is kept** alongside the story as a deliberate PoC
  duplicate. Decide its fate later (keep "logic in unit, render in stories", or
  migrate).
- **Apache-2.0 hygiene:** every dependency above is public (npmjs); no private
  registry or `.npmrc` is required.
</content>
</invoke>
