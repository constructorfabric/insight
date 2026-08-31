import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import type { Gear } from "@/api/gear-roadmap-client";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { buildGantt, monthTicks } from "@/lib/gears/gantt";
import { subsystemTone } from "@/lib/gears/subsystem-tone";
import { useGearRoadmap } from "@/queries/gear-roadmap";

const DAY_WIDTH_PX = 4;
const LANE_LABEL_WIDTH = "11rem";
const MIN_BAR_LABEL_DAYS = 14;

export function GearSchedule() {
  const { t } = useTranslation();
  const { data, isPending, isError } = useGearRoadmap();

  const chart = useMemo(() => buildGantt(data?.lanes ?? []), [data]);
  const gears = useMemo(
    () => new Map((data?.gears ?? []).map((gear) => [gear.number, gear])),
    [data],
  );
  const ticks = useMemo(() => monthTicks(chart.start, chart.totalDays), [chart]);

  if (isPending) return <CenteredSpinner />;
  if (isError || !data)
    return <p role="alert">{t("gear_roadmap.load_failed")}</p>;
  if (chart.totalDays === 0)
    return (
      <p className="text-sm text-muted-foreground">
        {t("gear_roadmap.gantt.nothing_scheduled")}
      </p>
    );

  const trackWidth = chart.totalDays * DAY_WIDTH_PX;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-sm text-muted-foreground">
        {t("gear_roadmap.gantt.explainer", {
          capacity: data.capacity_man_days_per_person,
          start: chart.start,
          days: chart.totalDays,
        })}
      </p>

      <div className="overflow-x-auto rounded-lg border bg-card">
        <div style={{ minWidth: `calc(${LANE_LABEL_WIDTH} + ${trackWidth}px)` }}>
          <div className="flex items-end border-b bg-muted/40">
            <div
              className="sticky left-0 z-10 shrink-0 bg-muted/40 px-3 py-2 text-xs font-medium text-muted-foreground"
              style={{ width: LANE_LABEL_WIDTH }}
            >
              {t("gear_roadmap.gantt.lane")}
            </div>
            <div
              className="relative h-8 shrink-0"
              style={{ width: `${trackWidth}px` }}
            >
              {ticks.map((tick) => (
                <span
                  key={tick.label}
                  className="absolute top-2 border-s ps-1 text-xs text-muted-foreground"
                  style={{ left: `${tick.offsetDays * DAY_WIDTH_PX}px` }}
                >
                  {tick.label.slice(2)}
                </span>
              ))}
            </div>
          </div>

          {chart.lanes.map((lane, laneIndex) => (
            <div
              key={lane.assignee ?? `unassigned-${laneIndex}`}
              className="flex items-center border-b last:border-b-0 hover:bg-accent/30"
            >
              <div
                className="sticky left-0 z-10 shrink-0 truncate bg-card px-3 py-2 text-sm"
                style={{ width: LANE_LABEL_WIDTH }}
                title={lane.assignee ?? t("gear_roadmap.gantt.unassigned")}
              >
                {lane.assignee ?? (
                  <span className="text-muted-foreground italic">
                    {t("gear_roadmap.gantt.unassigned")}
                  </span>
                )}
              </div>

              <div
                className="relative h-9 shrink-0"
                style={{ width: `${trackWidth}px` }}
              >
                {ticks.map((tick) => (
                  <div
                    key={tick.label}
                    className="absolute inset-y-0 border-s border-border/60"
                    style={{ left: `${tick.offsetDays * DAY_WIDTH_PX}px` }}
                  />
                ))}

                {lane.bars.map((bar) => (
                  <Bar
                    key={bar.gearNumber}
                    gear={gears.get(bar.gearNumber)}
                    offsetDays={bar.offsetDays}
                    lengthDays={bar.lengthDays}
                  />
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function Bar({
  gear,
  offsetDays,
  lengthDays,
}: {
  gear: Gear | undefined;
  offsetDays: number;
  lengthDays: number;
}) {
  const tone = subsystemTone(gear?.subsystem ?? null);
  const label = gear?.title ?? "";

  return (
    <div
      className={`absolute top-1.5 flex h-6 items-center overflow-hidden rounded-sm px-1.5 text-xs ${tone.bar}`}
      style={{
        left: `${offsetDays * DAY_WIDTH_PX}px`,
        width: `${Math.max(lengthDays * DAY_WIDTH_PX, 6)}px`,
      }}
      title={`${label}${gear?.milestone ? ` · ${gear.milestone}` : ""}`}
    >
      {lengthDays >= MIN_BAR_LABEL_DAYS ? (
        <span className="truncate">{shortTitle(label)}</span>
      ) : null}
    </div>
  );
}

function shortTitle(title: string): string {
  const [, tail] = title.split(" - ");
  return tail ?? title;
}
