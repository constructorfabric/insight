import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { buildGantt } from "@/lib/gears/gantt";
import { useGearRoadmap } from "@/queries/gear-roadmap";

export function GearSchedule() {
  const { t } = useTranslation();
  const { data, isPending, isError } = useGearRoadmap();

  const chart = useMemo(() => buildGantt(data?.lanes ?? []), [data]);
  const titles = useMemo(
    () => new Map((data?.gears ?? []).map((gear) => [gear.number, gear.title])),
    [data],
  );

  if (isPending) return <CenteredSpinner />;
  if (isError || !data) return <p role="alert">{t("gear_roadmap.load_failed")}</p>;
  if (chart.totalDays === 0) {
    return <p>{t("gear_roadmap.gantt.nothing_scheduled")}</p>;
  }

  return (
    <section className="flex flex-col gap-3">
      <p className="text-sm text-muted-foreground">
        {t("gear_roadmap.gantt.explainer", {
          capacity: data.capacity_man_days_per_person,
          start: chart.start,
          days: chart.totalDays,
        })}
      </p>

      <ul className="flex flex-col gap-2">
        {chart.lanes.map((lane, laneIndex) => (
          <li
            key={lane.assignee ?? `unassigned-${laneIndex}`}
            className="flex items-center gap-3"
          >
            <span className="w-40 shrink-0 truncate text-sm">
              {lane.assignee ?? t("gear_roadmap.gantt.unassigned")}
            </span>
            <div className="relative h-6 flex-1 rounded bg-muted">
              {lane.bars.map((bar) => (
                <div
                  key={bar.gearNumber}
                  title={titles.get(bar.gearNumber) ?? String(bar.gearNumber)}
                  className="absolute top-0 h-6 overflow-hidden rounded bg-primary/80 px-1 text-xs leading-6 text-primary-foreground"
                  style={{
                    left: `${(bar.offsetDays / chart.totalDays) * 100}%`,
                    width: `${(bar.lengthDays / chart.totalDays) * 100}%`,
                  }}
                >
                  {titles.get(bar.gearNumber) ?? bar.gearNumber}
                </div>
              ))}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
