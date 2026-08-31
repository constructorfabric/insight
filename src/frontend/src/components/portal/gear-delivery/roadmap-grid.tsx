import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import type { Gear } from "@/api/gear-roadmap-client";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { buildRoadmap } from "@/lib/gears/roadmap-grid";
import { subsystemTone } from "@/lib/gears/subsystem-tone";
import { useGearRoadmap } from "@/queries/gear-roadmap";

const COMMITTED_GLYPH = "◆";
const PLANNED_GLYPH = "◇";

export function RoadmapGrid() {
  const { t } = useTranslation();
  const { data, isPending, isError } = useGearRoadmap();

  const rows = useMemo(
    () => buildRoadmap(data?.gears ?? [], data?.window_months ?? 0),
    [data],
  );

  if (isPending) return <CenteredSpinner />;
  if (isError || !data)
    return <p role="alert">{t("gear_roadmap.load_failed")}</p>;

  const months = monthLabels(data.window_start, data.window_months);

  return (
    <section className="flex flex-col gap-3">
      <p className="flex flex-wrap items-center gap-4 text-xs text-muted-foreground">
        <span>
          <span className="text-foreground">{COMMITTED_GLYPH}</span>{" "}
          {t("gear_roadmap.grid.committed_legend")}
        </span>
        <span>
          <span className="text-foreground">{PLANNED_GLYPH}</span>{" "}
          {t("gear_roadmap.grid.planned_legend")}
        </span>
      </p>

      <div className="overflow-x-auto rounded-lg border bg-card">
        <table className="w-full min-w-max border-collapse text-sm">
          <thead>
            <tr className="bg-muted/40">
              <th className="sticky left-0 z-10 bg-muted/40 px-3 py-2 text-start text-xs font-medium text-muted-foreground">
                {t("gear_roadmap.items.subsystem")}
              </th>
              <th className="min-w-56 border-s px-3 py-2 text-start text-xs font-medium text-destructive">
                {t("gear_roadmap.grid.overdue")}
              </th>
              {months.map((month) => (
                <th
                  key={month}
                  className="min-w-52 border-s px-3 py-2 text-start text-xs font-medium text-muted-foreground"
                >
                  {month}
                </th>
              ))}
              <th className="min-w-52 border-s px-3 py-2 text-start text-xs font-medium text-muted-foreground">
                {t("gear_roadmap.grid.later")}
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.subsystem} className="border-t align-top">
                <th
                  scope="row"
                  className="sticky left-0 z-10 bg-card px-3 py-2 text-start font-semibold"
                >
                  {row.subsystem}
                </th>
                <Cell gears={row.overdue} overdue />
                {row.slots.map((slot, index) => (
                  <Cell key={months[index] ?? index} gears={slot} />
                ))}
                <Cell gears={row.later} />
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function Cell({ gears, overdue }: { gears: Gear[]; overdue?: boolean }) {
  return (
    <td className={`border-s px-2 py-2 ${overdue ? "bg-destructive/5" : ""}`}>
      {gears.length === 0 ? (
        <span className="text-xs text-muted-foreground">·</span>
      ) : (
        <ul className="flex flex-col gap-1">
          {gears.map((gear) => (
            <li key={gear.number} className="flex">
              <span
                className={`inline-flex min-w-0 items-baseline gap-1 rounded px-1.5 py-0.5 text-xs ${
                  overdue
                    ? "bg-destructive/10 text-destructive"
                    : subsystemTone(gear.subsystem ?? null).chip
                }`}
                title={gear.title}
              >
                <span aria-hidden="true">
                  {gear.commitment === "committed"
                    ? COMMITTED_GLYPH
                    : PLANNED_GLYPH}
                </span>
                <span className="truncate">{gear.title}</span>
              </span>
            </li>
          ))}
        </ul>
      )}
    </td>
  );
}

function monthLabels(windowStart: string, months: number): string[] {
  const [year, month] = windowStart.split("-").map(Number);
  if (!year || !month) return [];

  return Array.from({ length: months }, (_, index) => {
    const date = new Date(Date.UTC(year, month - 1 + index, 1));
    return date.toISOString().slice(0, 7);
  });
}
