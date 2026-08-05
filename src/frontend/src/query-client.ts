/**
 * Single shared TanStack Query client for the app.
 *
 * Defaults are chosen for analytics-heavy, read-only screens:
 * - `staleTime: 1 h` — analytics-api refreshes silver/gold tables on a
 *   daily cadence, so the same dashboard payload is byte-identical for
 *   long stretches. An hour is a safe ceiling that survives a normal work
 *   session without redundant fetches.
 * - `refetchOnWindowFocus` / `refetchOnReconnect: false` — the data changes
 *   at most daily, so automatic refetches on tab-focus or network reconnect
 *   only add noise to a dashboard with 6+ panels. Data still refreshes on an
 *   explicit navigation or period change once past `staleTime`.
 * - `retry: 1` — analytics-api 5xx is occasional; one retry without a
 *   backoff multiplier is cheap and saves the user a manual refresh.
 *
 * Mutations stay on RQ defaults (no automatic retry) — there are none in
 * the sales dashboard today, but the client is shared app-wide.
 */

import { QueryClient } from "@tanstack/react-query";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 60 * 60_000,
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
      retry: 1,
    },
  },
});
