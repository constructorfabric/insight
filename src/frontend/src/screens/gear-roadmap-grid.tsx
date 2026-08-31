import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import type { Gear } from "@/api/gear-roadmap-client";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { buildRoadmap } from "@/lib/gears/roadmap-grid";
import { useGearRoadmap } from "@/queries/gear-roadmap";

const COMMITTED_GLYPH = "◆";
const PLANNED_GLYPH = "◇";

export function GearRoadmapGridScreen() {
  const { t } = useTranslation();
  const { data, isPending, isError } = useGearRoadmap();

  const rows = useMemo(
    () => buildRoadmap(data?.gears ?? [], data?.window_months ?? 0),
    [data],
  );

  if (isPending) return <CenteredSpinner />;
  if (isError || !data) return <p role="alert">{t("gear_roadmap.load_failed")}</p>;

  const months = monthLabels(data.window_start, data.window_months);

  return (
    <section className="flex flex-col gap-3 overflow-x-auto">
      <p className="text-sm text-muted-foreground">
        {t("gear_roadmap.grid.legend", {
          committed: COMMITTED_GLYPH,
          planned: PLANNED_GLYPH,
        })}
      </p>

      <table className="w-full min-w-max border-collapse text-sm">
        <thead>
          <tr>
            <th className="border-b p-2 text-start">
              {t("gear_roadmap.items.subsystem")}
            </th>
            <th className="border-b p-2 text-start text-destructive">
              {t("gear_roadmap.grid.overdue")}
            </th>
            {months.map((month) => (
              <th key={month} className="border-b p-2 text-start font-medium">
                {month}
              </th>
            ))}
            <th className="border-b p-2 text-start">
              {t("gear_roadmap.grid.later")}
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.subsystem} className="align-top">
              <th className="border-b p-2 text-start font-semibold">
                {row.subsystem}
              </th>
              <Cell gears={row.overdue} tone="overdue" />
              {row.slots.map((slot, index) => (
                <Cell key={months[index] ?? index} gears={slot} />
              ))}
              <Cell gears={row.later} />
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

function Cell({ gears, tone }: { gears: Gear[]; tone?: "overdue" }) {
  return (
    <td className="border-b p-2">
      <ul className="flex flex-col gap-1">
        {gears.map((gear) => (
          <li
            key={gear.number}
            className={
              tone === "overdue" ? "text-destructive" : "text-foreground"
            }
          >
            <span aria-hidden="true">
              {gear.commitment === "committed"
                ? COMMITTED_GLYPH
                : PLANNED_GLYPH}
            </span>{" "}
            {gear.title}
          </li>
        ))}
      </ul>
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
