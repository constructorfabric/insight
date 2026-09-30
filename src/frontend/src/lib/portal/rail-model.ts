import {
  Compass,
  FileText,
  House,
  LayoutGrid,
  Settings2,
  Users,
  type LucideIcon,
} from "lucide-react";

import { zoneIsPlanned, type Zone } from "./nav-model";
import { navPolicy, type InstanceNavPolicy } from "./nav-policy";

export type RailId = "home" | "dashboards" | "explore" | "people" | "reports" | "manage";

export interface RailItem {
  id: RailId;
  label: string;
  paneTitle?: string;
  icon: LucideIcon;
  zones: readonly string[];
}

export interface RailEntry extends RailItem {
  planned: boolean;
}

export const RAIL: readonly RailItem[] = [
  {
    id: "home",
    label: "Home",
    paneTitle: "Constructor Insight",
    icon: House,
    zones: ["overview"],
  },
  { id: "dashboards", label: "Dashboards", icon: LayoutGrid, zones: ["custom"] },
  { id: "explore", label: "Explore", icon: Compass, zones: ["directions", "aicost"] },
  { id: "people", label: "People", icon: Users, zones: ["person", "people"] },
  { id: "reports", label: "Reports", icon: FileText, zones: ["reports"] },
  { id: "manage", label: "Manage", icon: Settings2, zones: ["manage"] },
];

export function railItemFor(zoneId: string): RailItem | undefined {
  return RAIL.find((item) => item.zones.includes(zoneId));
}

export function visibleRailItems(
  visibleZones: readonly Zone[],
  policy: InstanceNavPolicy = navPolicy(),
): RailEntry[] {
  return RAIL.flatMap((item) => {
    const owned = visibleZones.filter((zone) => item.zones.includes(zone.id));
    if (!owned.length) return [];
    return [{ ...item, planned: owned.every((zone) => zoneIsPlanned(zone, policy)) }];
  });
}
