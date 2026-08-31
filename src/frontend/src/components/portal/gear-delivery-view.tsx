import { useTranslation } from "react-i18next";

import { GearsTable } from "@/components/portal/gear-delivery/gears-table";
import { RoadmapGrid } from "@/components/portal/gear-delivery/roadmap-grid";
import { GearSchedule } from "@/components/portal/gear-delivery/schedule";
import { GearSummary } from "@/components/portal/gear-delivery/summary";
import { Badge } from "@/components/ui/badge";
import { useGearRoadmap } from "@/queries/gear-roadmap";

/**
 * Gear delivery zone: the board rolled up, listed, placed on months, and
 * scheduled. Every pane states the capacity the schedule assumed, because a
 * forecast drawn from an assumed number must not read as a measured one.
 */
export function GearDeliveryView({ item }: { item: string | null }) {
  const { t } = useTranslation();
  const { data } = useGearRoadmap();

  return (
    <div className="flex flex-1 flex-col gap-4 p-4">
      <header className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold tracking-tight">
          {t("gear_roadmap.title")}
        </h1>
        {data ? (
          <Badge variant="outline" className="font-normal">
            {t("gear_roadmap.capacity_note", {
              capacity: data.capacity_man_days_per_person,
            })}
          </Badge>
        ) : null}
      </header>

      <Pane item={item} />
    </div>
  );
}

function Pane({ item }: { item: string | null }) {
  switch (item) {
    case "gears":
      return <GearsTable />;
    case "roadmap":
      return <RoadmapGrid />;
    case "schedule":
      return <GearSchedule />;
    default:
      return <GearSummary />;
  }
}
