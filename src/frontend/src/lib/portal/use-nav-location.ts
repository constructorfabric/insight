import { useRouter, useRouterState } from "@tanstack/react-router";
import { useMemo } from "react";

import { useViewer } from "@/auth";
import { normalizePersonId } from "@/lib/metrics/entity";
import { bareLandingZone } from "@/lib/portal/landing-zone";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { locationOf, type NavLocation } from "@/lib/portal/portal-routes";

declare module "@tanstack/react-router" {
  interface StaticDataRouteOption {
    /** The nav-model zone this route renders. */
    zone?: string;
    /** The pane item a route renders when its path carries none. */
    item?: string;
  }
}

/**
 * Where the reader is, read from the deepest matched route that names a zone.
 * `person` falls back to the viewer, so the rail's "Me" has someone to mean.
 *
 * The page being opened counts as soon as the click lands: while its route
 * loads, the pane already shows where the reader is going rather than
 * lagging a page behind.
 */
export function useNavLocation(): NavLocation & { person: string } {
  const router = useRouter();
  // `location` moves the moment a navigation starts; the matches only once
  // the new route has loaded.
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const { personId } = useViewer();
  const showPlanned = usePortalShowPlanned();

  const [routes, rawParams] = useMemo(() => router.getMatchedRoutes(pathname), [router, pathname]);
  const named = routes.findLast((route) => route.options.staticData?.zone);
  const zone = named?.options.staticData?.zone ?? bareLandingZone(showPlanned);
  const fixedItem = named?.options.staticData?.item;
  const params = rawParams as Partial<Record<string, string>>;
  const { view, surface, section, person, dir, lens } = params;

  return useMemo(() => {
    const location = locationOf(zone, fixedItem, { view, surface, section, person, dir, lens });
    return { ...location, person: normalizePersonId(location.person ?? personId ?? "") };
  }, [zone, fixedItem, view, surface, section, person, dir, lens, personId]);
}
