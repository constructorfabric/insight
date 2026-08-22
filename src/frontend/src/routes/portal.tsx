import { Navigate, createFileRoute, retainSearchParams } from "@tanstack/react-router";

import { PortalLayout } from "@/components/portal/portal-layout";
import {
  PORTAL_SEARCH_KEYS,
  validatePortalSearch,
} from "@/lib/portal/portal-search";
import { readLegacyShell } from "@/lib/portal/portal-store";

/**
 * The portal's org zones (Overview / Directions / AI & Cost / Manage).
 *
 * Person and People are NOT here: they are about one person, so they keep
 * their own `/ic/$person/*` routes and carry the same search params. The zone
 * therefore comes from the route on those, and from `?zone=` here.
 */
export const Route = createFileRoute("/portal")({
  validateSearch: validatePortalSearch,
  search: {
    // Retain the portal's own keys across every navigation, including the jump
    // between /portal and a person route. Passing `search` by hand at each call
    // site is how they got dropped in the first place — five links, one of them
    // missed, and the scope silently resets.
    middlewares: [retainSearchParams(PORTAL_SEARCH_KEYS)],
  },
  component: PortalRoute,
});

function PortalRoute() {
  // The root shell swaps in PortalLayout for this route, so this component
  // only mounts under the legacy-shell hatch — a pasted /portal URL from a
  // document told to render the other shell. Send it the app it does have.
  if (readLegacyShell()) return <Navigate to="/" replace />;
  return <PortalLayout />;
}
