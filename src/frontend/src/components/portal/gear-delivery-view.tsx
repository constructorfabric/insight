import { useState } from "react";
import { useTranslation } from "react-i18next";

import { GearsTable } from "@/components/portal/gear-delivery/gears-table";
import { RoadmapGrid } from "@/components/portal/gear-delivery/roadmap-grid";
import { GearSchedule } from "@/components/portal/gear-delivery/schedule";
import { GearSummary } from "@/components/portal/gear-delivery/summary";
import { Badge } from "@/components/ui/badge";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import type { GearRoadmap } from "@/api/gear-roadmap-client";
import type { BoardLensConfig } from "@/lib/portal/lens-configs";
import { usePortalGearOrder } from "@/lib/portal/portal-nav";
import { useGearBoards, useGearRoadmap } from "@/queries/gear-roadmap";

/**
 * A Development lens over the gear board. Each pane states the capacity the
 * schedule assumed, because a forecast drawn from an assumed number must not
 * read as a measured one.
 */
export function GearDeliveryView({ config }: { config: BoardLensConfig }) {
  const { t } = useTranslation();
  const order = usePortalGearOrder();
  const boards = useGearBoards();
  const [chosen, setChosen] = useState<number | null>(null);

  const project = chosen ?? boards.data?.[0]?.number ?? null;

  const { data, isPending, isError } = useGearRoadmap(
    project,
    order.sort
      ? {
          sort: order.sort,
          direction: order.direction === "desc" ? "desc" : "asc",
        }
      : null,
  );

  if (boards.isSuccess && boards.data.length === 0) {
    return (
      <div className="flex flex-col gap-6 p-6">
        <p role="status">{t("gear_roadmap.no_boards")}</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-6 p-6">
      <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h2 className="text-lg font-semibold">{config.title}</h2>
        {config.tagline ? (
          <p className="text-sm text-muted-foreground">{config.tagline}</p>
        ) : null}
        {boards.data && boards.data.length > 1 ? (
          <label className="ms-auto flex items-center gap-2 text-sm">
            <span className="text-muted-foreground">
              {t("gear_roadmap.board_label")}
            </span>
            <select
              className="rounded-md border bg-background px-2 py-1"
              value={project ?? ""}
              onChange={(event) => setChosen(Number(event.target.value))}
            >
              {boards.data.map((board) => (
                <option key={board.number} value={board.number}>
                  {t("gear_roadmap.board_option", {
                    number: board.number,
                    cards: board.cards,
                  })}
                </option>
              ))}
            </select>
          </label>
        ) : null}
        {data ? (
          <Badge
            variant="outline"
            className={
              boards.data && boards.data.length > 1
                ? "font-normal"
                : "ms-auto font-normal"
            }
          >
            {t("gear_roadmap.capacity_note", {
              capacity: data.capacity_man_days_per_person,
            })}
          </Badge>
        ) : null}
      </header>

      {isPending ? <CenteredSpinner /> : null}
      {isError || (!isPending && !data) ? (
        <p role="alert">{t("gear_roadmap.load_failed")}</p>
      ) : null}
      {data ? <Board board={config.board} roadmap={data} /> : null}
    </div>
  );
}

function Board({
  board,
  roadmap,
}: {
  board: BoardLensConfig["board"];
  roadmap: GearRoadmap;
}) {
  switch (board) {
    case "gear-table":
      return <GearsTable roadmap={roadmap} />;
    case "gear-roadmap":
      return <RoadmapGrid roadmap={roadmap} />;
    case "gear-schedule":
      return <GearSchedule roadmap={roadmap} />;
    case "gear-summary":
      return <GearSummary roadmap={roadmap} />;
  }
}
