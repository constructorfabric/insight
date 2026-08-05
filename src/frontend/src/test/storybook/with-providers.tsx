/**
 * Story decorator that mounts a story inside Insight's real provider stack.
 *
 * We keep the providers a widget actually needs in isolation:
 *   - QueryClientProvider  — fresh client per story (retry off, gcTime 0)
 *   - TanStack Router      — memory-history router (story = not-found route)
 *   - ThemeProvider        — light/dark classes; the theme toolbar drives it
 *   - I18nextProvider      — app i18n instance
 */

import type { Decorator } from "@storybook/react-vite";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { useEffect, useMemo } from "react";
import { I18nextProvider } from "react-i18next";

import { ThemeProvider } from "@/components/theme-provider";
import i18n from "@/i18n";
import { createTestingRouter } from "./test-router";

export const WithProviders: Decorator = (Story, context) => {
  // Drive our own ThemeProvider from the addon-themes toolbar global. Without
  // this, ThemeProvider reads (empty) localStorage → falls back to "system" and
  // rewrites <html>, clobbering the light/dark class withThemeByClassName set.
  const selectedTheme = context.globals.theme === "dark" ? "dark" : "light";
  const client = useMemo(
    () =>
      new QueryClient({
        defaultOptions: { queries: { retry: false, gcTime: 0 } },
      }),
    [],
  );
  const { router, reset } = useMemo(() => createTestingRouter(), []);

  // Render the story as the memory-router's not-found component. This gives
  // the story full router context (Link / useNavigate / useRouterState)
  // without the app route tree.
  // reset() first so a story that navigated doesn't leak its location into the
  // next story (the router is memoized for the decorator's lifetime).
  useEffect(() => {
    reset();
    router.update({ defaultNotFoundComponent: () => <Story /> });
  }, [router, reset, Story]);

  return (
    <QueryClientProvider client={client}>
      <ThemeProvider key={selectedTheme} defaultTheme={selectedTheme}>
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
        </I18nextProvider>
      </ThemeProvider>
    </QueryClientProvider>
  );
};
