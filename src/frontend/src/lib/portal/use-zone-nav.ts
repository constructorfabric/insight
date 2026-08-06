import { useNavigate } from "@tanstack/react-router";

import { ZONES, type Zone } from "@/lib/portal/nav-model";
import {
  usePortalShowPlanned,
} from "@/lib/portal/portal-store";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { useViewerIsManager } from "@/lib/portal/use-viewer-is-manager";

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
  const { isManager, isPending: mgrPending } = useViewerIsManager();
  // Zones we have not built are hidden unless the viewer opted into seeing
  // planned work — a rail of scaffolds makes the built zones look unreliable.
  const showPlanned = usePortalShowPlanned();
  // An IC (no reports) has no subtree to roll up, so org zones are hidden — the
  // shell collapses to Person. While the viewer's identity is still resolving,
  // assume manager so the nav doesn't flash a collapsed state.
  const orgZonesVisible = isManager || mgrPending;

  const zones = ZONES.filter(
    (z) =>
      (orgZonesVisible || IC_ZONES.has(z.id)) &&
      (z.readiness !== "unbuilt" || showPlanned),
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
        ...(activeZone !== zone.id ? { item: undefined } : {}),
        zone: entity ? undefined : zone.id,
      }),
    });
  }

  return { zones, activeZone, selectZone };
}
