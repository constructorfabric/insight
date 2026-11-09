import { useViewer } from "@/auth";
import { zoneHidden } from "@/lib/portal/nav-policy";
import {
  ZONES,
  ZONES_FOR_EVERYONE,
  zoneIsPlanned,
  type Zone,
} from "@/lib/portal/nav-model";
import { useGo } from "@/lib/portal/portal-nav";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useNavLocation } from "@/lib/portal/use-nav-location";
import { useViewerReach } from "@/lib/portal/use-viewer-reach";
import { useIsAdmin } from "@/queries/identity-me";

export interface ZoneNav {
  zones: Zone[];
  activeZone: string;
  me: string;
  selectZone: (zone: Zone, item?: string) => void;
}

/**
 * The zone list the viewer may see plus the selection behaviour, shared by the
 * desktop icon rail and the mobile drawer so both offer exactly the same zones
 * and route the same way. Person and People open the viewer's own pages.
 */
export function useZoneNav(): ZoneNav {
  const go = useGo();
  const { zone: activeZone, person: activePerson } = useNavLocation();
  const { personId: viewerPersonId } = useViewer();
  const me = viewerPersonId ?? activePerson;
  const { canSeeOthers, isPending: reachPending } = useViewerReach();
  // A rail of scaffolds makes the built zones look unreliable.
  const showPlanned = usePortalShowPlanned();
  // A viewer with nobody to look at has nothing to roll up, so org zones are
  // hidden and the shell keeps only the zones every viewer has. While the answer is resolving,
  // assume they have a cohort so the nav doesn't flash a collapsed state.
  const orgZonesVisible = canSeeOthers || reachPending;
  // Manage is opened by the admin role, not by having reports: the operator
  // persona is an IC by design. Fail closed while the role check is pending —
  // a rail entry that vanishes is worse than one that appears a beat late.
  const { isAdmin } = useIsAdmin();

  const zones = ZONES.filter((z) => {
    if (zoneHidden(z.id) || (zoneIsPlanned(z) && !showPlanned)) return false;
    if (ZONES_FOR_EVERYONE.has(z.id)) return true;

    return orgZonesVisible || (z.id === "manage" && isAdmin);
  });

  function selectZone(zone: Zone, item?: string) {
    switch (zone.id) {
      case "person":
        if (me) go({ zone: "person", person: me, section: item });
        return;
      case "people":
        if (me) go({ zone: "people", person: me, item });
        return;
      case "directions":
        go({ zone: "directions" });
        return;
      case "custom":
        go({ zone: "custom" });
        return;
      case "overview":
      case "aicost":
      case "reports":
      case "manage":
        go({ zone: zone.id, item });
    }
  }

  return { zones, activeZone, me, selectZone };
}
