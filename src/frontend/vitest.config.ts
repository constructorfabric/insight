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
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
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
          // Preview annotations (decorators / MSW loader / parameters from
          // .storybook/preview.tsx) are applied automatically by
          // @storybook/addon-vitest since Storybook 10.3 — no setup file needed.
        },
      },
    ],
  },
});
