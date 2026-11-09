import { createFileRoute, retainSearchParams } from "@tanstack/react-router";

import { PortalLayout } from "@/components/portal/portal-layout";
import { GLOBAL_SEARCH_KEYS, validateGlobalSearch } from "@/lib/portal/portal-search";

/**
 * The portal shell, mounted once for every page under it: the rail, the pane
 * and the top bar stay put while the outlet swaps the page.
 */
export const Route = createFileRoute("/_shell")({
  validateSearch: validateGlobalSearch,
  search: {
    // Scope, cohort and period frame every page, so every navigation carries
    // them; a page's own keys stay with that page.
    middlewares: [retainSearchParams(GLOBAL_SEARCH_KEYS)],
  },
  component: PortalLayout,
});
