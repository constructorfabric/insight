import { useRouterState } from "@tanstack/react-router";
import { useMemo } from "react";

import { useViewer } from "@/auth";
import { personIdFromPath } from "@/lib/metrics/entity";
import { bareLandingZone } from "@/lib/portal/landing-zone";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";

import { usePortalZone } from "./portal-nav";

/**
 * Resolves the active portal zone and the person the entity lenses point at.
 * The zone itself comes from the URL (`usePortalZone`: the path for
 * Person/People, `?zone=` for theme zones); this hook adds the person the
 * route names (its person id), so the rail and the context pane highlight in
 * sync.
 */
export function useActiveZone(): { activeZone: string; activePerson: string } {
  const zone = usePortalZone();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { personId } = useViewer();
  const showPlanned = usePortalShowPlanned();

  return useMemo(() => {
    // The path segment is a person id since the identity cutover, so this is
    // the id every portal surface keys on — not an email.
    const activePerson = personIdFromPath(pathname) ?? personId ?? "";
    // A bare /portal is Home, everyone's landing; an install that hides Home
    // lands on the viewer's own page instead.
    return { activeZone: zone ?? bareLandingZone(showPlanned), activePerson };
  }, [zone, pathname, personId, showPlanned]);
}
