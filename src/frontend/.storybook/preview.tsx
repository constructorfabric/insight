/**
 * Storybook preview: global decorators, MSW, and app styles.
 *
 * Wires stories to Insight's own theme/i18n/router providers and `index.css`
 * (Tailwind v4 + theme tokens).
 */

import type { Preview } from "@storybook/react-vite";
import { initialize, mswLoader } from "msw-storybook-addon";

import "@/index.css";
import { authStore } from "@/auth/auth-store";
import { themeDecorators } from "@/test/storybook/theme-decorators";
import { WithProviders } from "@/test/storybook/with-providers";

// Start the MSW worker once. Unhandled requests pass through so a story
// without explicit handlers still renders (it just won't see mock data).
initialize({ onUnhandledRequest: "bypass" });

const preview: Preview = {
  // Opt-in test model: strip the `test` tag Storybook auto-applies to every
  // story, so a story runs as a Vitest browser test ONLY when it re-adds
  // `tags: ["test"]` itself. Without this, `tags.include: ["test"]` in
  // vitest.config.ts would run every story (demo stories included) as a test.
  tags: ["!test"],
  parameters: {
    layout: "centered",
    // Keep `Test*` stories (component tests) out of the docs/autodocs pages —
    // they exist to run as Vitest browser tests, not as documentation.
    docs: {
      stories: {
        filter: (story: { name: string }) => !story.name.startsWith("Test"),
      },
    },
  },
  // Global reset of app-level singletons before every story. Per-story state
  // (QueryClient, router)
  // is already fresh via `WithProviders`' `useMemo`, and MSW handlers are reset
  // by the addon; this covers the module-level singletons a story might mutate
  // (auth) and any persisted UI prefs (theme / locale / settings in
  // localStorage) so nothing leaks between stories even if an author forgets.
  // A story that needs a signed-in tenant sets it in its own `beforeEach`,
  // which runs after this one (project → meta → story order).
  beforeEach: () => {
    authStore.reset();
    try {
      window.localStorage.clear();
    } catch {
      // localStorage may be unavailable; nothing to reset.
    }
  },
  loaders: [mswLoader],
  decorators: [...themeDecorators, WithProviders],
};

export default preview;
