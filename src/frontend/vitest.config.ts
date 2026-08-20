/**
 * Test runner config. Kept separate from `vite.config.ts` so vite's build
 * pipeline doesn't pull in the testing-library deps in production bundles,
 * and so the test setup file can run jsdom + jest-dom matchers without
 * interfering with dev-mode HMR.
 *
 * Two projects:
 *   - `unit`      — existing RTL/jsdom unit & component tests (`*.test.tsx`).
 *   - `storybook` — stories tagged `["test"]` run as tests in a real browser
 *                   via @storybook/addon-vitest + @vitest/browser-playwright.
 *                   See docs/testing/storybook-component-tests.md.
 *
 * Run a single project with `--project=unit` / `--project=storybook`.
 */

import path from "path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { playwright } from "@vitest/browser-playwright";
import { storybookTest } from "@storybook/addon-vitest/vitest-plugin";

import { uiKitLayer } from "./vite.ui-kit-layer";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    // The kit's JS chunks import their own .css, which node's ESM loader
    // rejects by extension before `css: false` can neutralise it.
    server: { deps: { inline: ["@gears-frontx/ui-kit"] } },
    // A timezone with no UTC-coinciding offset, ever. CI runners live in UTC,
    // where "parse a zone-less timestamp as UTC" and "parse it as local" are
    // the same function — every zone-handling test passes vacuously. Pinning
    // a non-UTC zone makes those tests able to fail.
    env: { TZ: "Pacific/Kiritimati" },
    // Coverage is a GLOBAL option in Vitest — with `projects` it must live at
    // the root `test` level; a `coverage` block nested inside a project is
    // ignored (which silently dropped our `cobertura` reporter and left CI's
    // diff-coverage gate without its report). CI runs `vitest run --coverage`
    // (`pnpm test:coverage:ci`) with no `--project` filter, so BOTH the jsdom
    // `unit` project and the browser `storybook` project run in one pass and
    // their coverage merges into a single report — a component exercised only by
    // a story counts toward the diff-coverage gate. (`pnpm test:coverage` stays
    // unit-only for a fast local loop that needs no browser.)
    coverage: {
      provider: "v8",
      reporter: ["text", "html", "cobertura"],
      reportsDirectory: "./coverage",
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/**/*.test.{ts,tsx}",
        "src/**/*.stories.{ts,tsx}", // exercised by the browser storybook project, not unit
        "src/test/**", // setup + test utils
        "src/mocks/**", // MSW handlers + factories
        "src/**/*.d.ts",
        "src/routeTree.gen.ts", // TanStack Router generated file
        "src/main.tsx", // entry/bootstrap
        "src/components/ui/**", // vendored shadcn/ui primitives
        "src/routes/**", // thin TanStack Router wrappers around screens
      ],
    },
    projects: [
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
      {
        extends: true,
        plugins: [
          // `.storybook/preview.tsx` imports `@/index.css`, but without the
          // Tailwind plugin its `@import "tailwindcss"` yields no utilities:
          // every element collapses to content size and size-driven widgets
          // (recharts measures its container) never paint. Storybook's own
          // builder gets this from `vite.config.ts`; this project does not.
          tailwindcss(),
          uiKitLayer(),
          storybookTest({
            configDir: path.resolve(__dirname, ".storybook"),
            tags: { include: ["test"], exclude: [], skip: ["skip-test"] },
          }),
        ],
        // Dependencies vite's pre-bundling scan does not reach from the story
        // entry points, because a story renders them only after it mounts.
        // Discovering one mid-run makes vite re-optimize and RELOAD the page,
        // which aborts whatever the running test was doing — most visibly a
        // dynamic import, which fails with "Failed to fetch dynamically
        // imported module". The failure lands on an unrelated test and only
        // on a cold cache, so it reads as a flake and reproduces nowhere but
        // CI. Listing them here is what vite's own warning asks for.
        //
        // A component newly reached from a story can add to this list. The
        // symptom to match it to: "new dependencies optimized: …" followed by
        // "optimized dependencies changed. reloading" in the run output.
        //
        // Read the FIRST of those two lines, not the test that failed. The
        // reload kills whichever import was in flight, so the failure is
        // reported against a bystander — usually the slowest thing to load.
        // Listing the bystander changes nothing; the dependency named by
        // "new dependencies optimized" is the one to add.
        //
        // Every `@base-ui/react/*` entry point the source imports is listed,
        // whether or not it has misbehaved yet, because none of them is
        // reached until a story mounts. To check the set still matches:
        //   grep -rhoE '"@base-ui/react/[a-z-]+"' src | sort -u
        optimizeDeps: {
          include: [
            "@base-ui/react/avatar",
            "@base-ui/react/button",
            "@base-ui/react/checkbox",
            "@base-ui/react/collapsible",
            "@base-ui/react/dialog",
            "@base-ui/react/input",
            "@base-ui/react/menu",
            "@base-ui/react/merge-props",
            "@base-ui/react/popover",
            "@base-ui/react/preview-card",
            "@base-ui/react/scroll-area",
            "@base-ui/react/select",
            "@base-ui/react/separator",
            "@base-ui/react/switch",
            "@base-ui/react/tabs",
            "@base-ui/react/toggle",
            "@base-ui/react/toggle-group",
            "@base-ui/react/tooltip",
            "@base-ui/react/use-render",
            "@gears-frontx/telemetry",
            "@gears-frontx/ui-kit",
            "@sentry/react",
            "@tanstack/react-virtual",
            // `await import("exceljs")` inside the export path: the scan cannot
            // see it, so it is discovered while the export story is running and
            // takes the page down with it as vite reloads.
            "exceljs",
            "react-day-picker",
            "react-error-boundary",
          ],
        },
        test: {
          name: "storybook",
          browser: {
            enabled: true,
            headless: true,
            provider: playwright(),
            instances: [{ browser: "chromium" }],
          },
          // Preview annotations (decorators / MSW loader / parameters from
          // .storybook/preview.tsx) are applied automatically by
          // @storybook/addon-vitest since Storybook 10.3 — no setup file needed.
        },
      },
    ],
  },
});
