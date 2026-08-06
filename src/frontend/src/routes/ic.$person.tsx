import { createFileRoute, Outlet, retainSearchParams } from "@tanstack/react-router";

import {
  PORTAL_SEARCH_KEYS,
  validatePortalSearch,
} from "@/lib/portal/portal-search";

export const Route = createFileRoute("/ic/$person")({
  // Person and People are portal zones too: the scope, slice and period a
  // reader picked must survive the drill into a person and the climb back out.
  validateSearch: validatePortalSearch,
  search: {
    // Retain the portal's own keys across every navigation, including the jump
    // between /portal and a person route. Passing `search` by hand at each call
    // site is how they got dropped in the first place — five links, one of them
    // missed, and the scope silently resets.
    middlewares: [retainSearchParams(PORTAL_SEARCH_KEYS)],
  },
  component: () => <Outlet />,
});
