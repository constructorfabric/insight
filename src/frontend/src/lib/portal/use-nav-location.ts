import { useMatches } from "@tanstack/react-router";
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
 */
export function useNavLocation(): NavLocation & { person: string } {
  const matches = useMatches();
  const { personId } = useViewer();
  const showPlanned = usePortalShowPlanned();

  const match = matches.findLast((candidate) => candidate.staticData.zone);
  const zone = match?.staticData.zone ?? bareLandingZone(showPlanned);
  const fixedItem = match?.staticData.item;
  const params = (match?.params ?? {}) as Partial<Record<string, string>>;
  const { view, surface, section, person, dir, lens } = params;

  return useMemo(() => {
    const location = locationOf(zone, fixedItem, { view, surface, section, person, dir, lens });
    return { ...location, person: normalizePersonId(location.person ?? personId ?? "") };
  }, [zone, fixedItem, view, surface, section, person, dir, lens, personId]);
}
