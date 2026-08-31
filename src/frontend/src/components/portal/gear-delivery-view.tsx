import { useTranslation } from "react-i18next";

import { GearsTable } from "@/components/portal/gear-delivery/gears-table";
import { RoadmapGrid } from "@/components/portal/gear-delivery/roadmap-grid";
import { GearSchedule } from "@/components/portal/gear-delivery/schedule";
import { GearSummary } from "@/components/portal/gear-delivery/summary";
import { Badge } from "@/components/ui/badge";
import type { BoardLensConfig } from "@/lib/portal/lens-configs";
import { useGearRoadmap } from "@/queries/gear-roadmap";

/**
 * A Development lens over the gear board. Each pane states the capacity the
 * schedule assumed, because a forecast drawn from an assumed number must not
 * read as a measured one.
 */
export function GearDeliveryView({ config }: { config: BoardLensConfig }) {
  const { t } = useTranslation();
  const { data } = useGearRoadmap();

  return (
    <div className="flex flex-col gap-6 p-6">
      <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h2 className="text-lg font-semibold">{config.title}</h2>
        {config.tagline ? (
          <p className="text-sm text-muted-foreground">{config.tagline}</p>
        ) : null}
        {data ? (
          <Badge variant="outline" className="ms-auto font-normal">
            {t("gear_roadmap.capacity_note", {
              capacity: data.capacity_man_days_per_person,
            })}
          </Badge>
        ) : null}
      </header>

      <Board board={config.board} />
    </div>
  );
}

function Board({ board }: { board: BoardLensConfig["board"] }) {
  switch (board) {
    case "gear-table":
      return <GearsTable />;
    case "gear-roadmap":
      return <RoadmapGrid />;
    case "gear-schedule":
      return <GearSchedule />;
    case "gear-summary":
      return <GearSummary />;
  }
}
