import { Card, CardContent } from "@/components/ui/card";
import { fmtCompact } from "@/lib/portal/metric-stats";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { dayHourMatrix, HOUR_BLOCKS, WEEKDAY_LABELS } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

/** Day-of-week × two-hour block magnitude ramp — when CI actually runs. */
export function HeatmapHoursSection({
  section,
  resolve,
}: {
  section: Extract<TenantSectionSpec, { kind: "heatmap-hours" }>;
  resolve: ResolveView;
}) {
  // The need is day-bucketed by construction (`sectionNeeds`), so the lens
  // bucket is irrelevant here.
  const r = resolve(sectionNeeds(section, "day")[0]);
  if (!r) return null;
  const { cells, max, total } = dayHourMatrix(tenantData(r).series);
  if (total <= 0 || max <= 0) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {section.title}
      </p>
      <Card>
        <CardContent className="p-4">
          <div
            className="grid gap-px text-xs"
            style={{ gridTemplateColumns: `3rem repeat(${HOUR_BLOCKS.length}, 1fr)` }}
          >
            <div />
            {HOUR_BLOCKS.map((block, index) => (
              <div
                key={block}
                className="text-center text-xs text-muted-foreground"
              >
                {index % 2 === 0 ? block : ""}
              </div>
            ))}
            {WEEKDAY_LABELS.map((day, dayIndex) => (
              <HeatmapRow
                key={day}
                day={day}
                values={cells[dayIndex]}
                max={max}
              />
            ))}
          </div>
          <p className="mt-2 text-xs text-muted-foreground">
            Two-hour blocks of the run's start, UTC · darker is more ·{" "}
            {fmtCompact(total)} total
          </p>
        </CardContent>
      </Card>
    </section>
  );
}

function HeatmapRow({
  day,
  values,
  max,
}: {
  day: string;
  values: number[];
  max: number;
}) {
  return (
    <>
      <div className="pr-2 text-right text-xs leading-5 text-muted-foreground">
        {day}
      </div>
      {values.map((value, index) => (
        <div
          key={HOUR_BLOCKS[index]}
          className="h-5 rounded-[2px]"
          title={`${day} ${HOUR_BLOCKS[index]}:00 — ${fmtCompact(value)}`}
          style={{
            // A nonzero cell never fades to invisible: 12% floor, then ramp.
            background:
              value <= 0
                ? "var(--muted)"
                : `color-mix(in srgb, var(--chart-1) ${Math.round(
                    12 + (value / max) * 78
                  )}%, transparent)`,
          }}
        />
      ))}
    </>
  );
}
