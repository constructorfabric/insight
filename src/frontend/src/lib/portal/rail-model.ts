import {
  Compass,
  FileText,
  House,
  LayoutGrid,
  Settings2,
  Users,
  type LucideIcon,
} from "lucide-react";

export interface RailItem {
  id: string;
  label: string;
  icon: LucideIcon;
  zones: readonly string[];
  planned?: boolean;
}

export const RAIL: readonly RailItem[] = [
  { id: "home", label: "Home", icon: House, zones: ["overview"] },
  { id: "dashboards", label: "Dashboards", icon: LayoutGrid, zones: ["custom"] },
  { id: "explore", label: "Explore", icon: Compass, zones: ["directions", "aicost"] },
  { id: "people", label: "People", icon: Users, zones: ["person", "people"] },
  { id: "reports", label: "Reports", icon: FileText, zones: ["reports"], planned: true },
  { id: "manage", label: "Manage", icon: Settings2, zones: ["manage"] },
];

export function railItemFor(zoneId: string): RailItem | undefined {
  return RAIL.find((item) => item.zones.includes(zoneId));
}

export function visibleRailItems(
  visibleZones: readonly string[],
  showPlanned: boolean,
): RailItem[] {
  return RAIL.filter(
    (item) =>
      (!item.planned || showPlanned) &&
      item.zones.some((zone) => visibleZones.includes(zone)),
  );
}
