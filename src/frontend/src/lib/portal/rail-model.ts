import {
  Compass,
  FileText,
  House,
  LayoutDashboard,
  Settings2,
  Users,
  type LucideIcon,
} from "lucide-react";

import type { DisplayText } from "@/i18n/localized-text";

import { zoneIsPlanned, type Zone } from "./nav-model";
import { navPolicy, type InstanceNavPolicy } from "./nav-policy";

export type RailId = "home" | "dashboards" | "explore" | "people" | "reports" | "manage";

export interface RailItem {
  id: RailId;
  label: DisplayText;
  paneTitle?: DisplayText;
  icon: LucideIcon;
  zones: readonly string[];
}

export interface RailEntry extends RailItem {
  planned: boolean;
}

export const RAIL: readonly RailItem[] = [
  {
    id: "home",
    label: { key: "nav.rail.home" },
    paneTitle: { key: "nav.product" },
    icon: House,
    zones: ["overview"],
  },
  { id: "dashboards", label: { key: "nav.rail.dashboards" }, icon: LayoutDashboard, zones: ["custom"] },
  { id: "explore", label: { key: "nav.rail.explore" }, icon: Compass, zones: ["directions", "aicost"] },
  { id: "people", label: { key: "nav.rail.people" }, icon: Users, zones: ["person", "people"] },
  { id: "reports", label: { key: "nav.rail.reports" }, icon: FileText, zones: ["reports"] },
  { id: "manage", label: { key: "nav.rail.manage" }, icon: Settings2, zones: ["manage"] },
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
