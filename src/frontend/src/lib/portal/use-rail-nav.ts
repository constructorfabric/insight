import { railItemFor, visibleRailItems, type RailEntry, type RailItem } from "@/lib/portal/rail-model";
import { useZoneNav } from "@/lib/portal/use-zone-nav";

export function useRailNav(): {
  items: RailEntry[];
  activeItem: string | null;
  selectItem: (item: RailItem) => void;
} {
  const { zones, activeZone, selectZone } = useZoneNav();

  function selectItem(item: RailItem) {
    const target = item.zones
      .map((id) => zones.find((zone) => zone.id === id))
      .find((zone) => zone != null);
    if (target) selectZone(target);
  }

  return {
    items: visibleRailItems(zones),
    activeItem: railItemFor(activeZone)?.id ?? null,
    selectItem,
  };
}
