import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { railItemFor, visibleRailItems, type RailItem } from "@/lib/portal/rail-model";
import { useZoneNav } from "@/lib/portal/use-zone-nav";

export function useRailNav(): {
  items: RailItem[];
  activeItem: string | null;
  selectItem: (item: RailItem) => void;
} {
  const { zones, activeZone, selectZone } = useZoneNav();
  const showPlanned = usePortalShowPlanned();
  const items = visibleRailItems(
    zones.map((zone) => zone.id),
    showPlanned,
  );

  function selectItem(item: RailItem) {
    const target = item.zones
      .map((id) => zones.find((zone) => zone.id === id))
      .find((zone) => zone != null);
    if (target) selectZone(target);
  }

  return {
    items,
    activeItem: railItemFor(activeZone)?.id ?? null,
    selectItem,
  };
}
