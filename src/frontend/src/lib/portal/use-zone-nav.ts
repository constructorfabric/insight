import { useNavigate } from "@tanstack/react-router";

import { zoneHidden, zonePlanned } from "@/lib/portal/nav-policy";
import { ZONES, type Zone } from "@/lib/portal/nav-model";
import {
  usePortalShowPlanned,
} from "@/lib/portal/portal-store";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { useViewerReach } from "@/lib/portal/use-viewer-reach";
import { useIsAdmin } from "@/queries/identity-me";

/**
 * Zones that still make sense when the viewer manages no one — everything else
 * rolls up a (non-existent) subtree. An IC's portal collapses to these.
 */
const IC_ZONES = new Set(["person"]);

/**
 * The zone list the viewer may see plus the selection behaviour, shared by the
 * desktop icon rail and the mobile drawer so both offer exactly the same zones
 * and route the same way. Two zones are route-driven (Person / People): they
 * navigate and clear the pinned zone, which is what lets a person-name click
 * inside a roster drill straight into Person; the rest pin the zone.
 */
export function useZoneNav(): {
  zones: Zone[];
  activeZone: string;
  selectZone: (zone: Zone) => void;
} {
  const navigate = useNavigate();
  const { activeZone, activePerson } = useActiveZone();
  const { canSeeOthers, isPending: reachPending } = useViewerReach();
  // A rail of scaffolds makes the built zones look unreliable.
  const showPlanned = usePortalShowPlanned();
  // A viewer with nobody to look at has nothing to roll up, so org zones are
  // hidden and the shell collapses to Person. While the answer is resolving,
  // assume they have a cohort so the nav doesn't flash a collapsed state.
  const orgZonesVisible = canSeeOthers || reachPending;
  // Manage is opened by the admin role, not by having reports: the operator
  // persona is an IC by design. Fail closed while the role check is pending —
  // a rail entry that vanishes is worse than one that appears a beat late.
  const { isAdmin } = useIsAdmin();

  const zones = ZONES.filter(
    (z) =>
      !zoneHidden(z.id) &&
      (orgZonesVisible || IC_ZONES.has(z.id) || (z.id === "manage" && isAdmin)) &&
      (!zonePlanned(z.id) || showPlanned),
  );

  function selectZone(zone: Zone) {
    // ONE navigation per click. Three separate writes (clear item, clear zone,
    // change path) meant three history entries, so Back walked through
    // half-states nobody chose.
    const entity = zone.kind === "person" || zone.kind === "people";
    if (entity && !activePerson) return;
    void navigate({
      ...(entity
        ? {
            to: zone.kind === "person" ? "/ic/$person/personal" : "/ic/$person/team",
            params: { person: activePerson },
          }
        : { to: "/portal" }),
      // `item` is per-zone: carrying it over renders a fallback view while the
      // pane highlights nothing. The path carries the zone for entity zones, so
      // a lingering `?zone=` there would only contradict it.
      search: (prev: Record<string, unknown>) => ({
        ...prev,
        ...(activeZone !== zone.id ? { item: undefined, acct: undefined } : {}),
        zone: entity ? undefined : zone.id,
      }),
    });
  }

  return { zones, activeZone, selectZone };
}
