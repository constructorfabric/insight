import type { HeatmapWidget, MetricResult } from "@/api/custom-client";
import { groupedNumber, unitFor } from "@/components/custom/chart-format";

import {
  calendarCells,
  calendarGrain,
  type CalendarCell,
} from "../adapters/calendar";
import { KindFigure } from "../chrome";

const LEVEL_FILL: Record<CalendarCell["level"], string> = {
  0: "var(--muted)",
  1: "color-mix(in srgb, var(--chart-1) 28%, var(--card))",
  2: "color-mix(in srgb, var(--chart-1) 50%, var(--card))",
  3: "color-mix(in srgb, var(--chart-1) 75%, var(--card))",
  4: "var(--chart-1)",
};

const MONTH = new Intl.DateTimeFormat("en", {
  month: "short",
  timeZone: "UTC",
});
const WEEKDAYS = ["Sun", "", "Tue", "", "Thu", "", "Sat"];
const CELL_PX = 14;

export function HeatmapKind({
  widget,
  result,
}: {
  widget: HeatmapWidget;
  result: MetricResult;
}) {
  if (calendarGrain(result, widget.x) !== "day") {
    return (
      <KindFigure kind="heatmap">
        <p className="m-auto max-w-72 text-center text-xs text-muted-foreground">
          This calendar needs one row per day. Pick a window of 31 days or less,
          or group the metric by a day column.
        </p>
      </KindFigure>
    );
  }

  const unit = unitFor(result.percents, widget.value);
  const cells = calendarCells(result, widget.x, widget.value);
  const weeks = (cells.at(-1)?.week ?? 0) + 1;
  const months = monthStarts(cells);
  const columns = `repeat(${weeks}, minmax(0, ${CELL_PX}px))`;

  return (
    <KindFigure kind="heatmap">
      <div className="flex h-full min-h-0 flex-col justify-center gap-1.5 text-xs text-muted-foreground">
        <div className="grid grid-cols-[auto_1fr] gap-x-2">
          <span />
          <div
            className="grid gap-x-[3px]"
            style={{ gridTemplateColumns: columns }}
          >
            {months.map(({ week, label }) => (
              <span key={week} style={{ gridColumnStart: week + 1 }}>
                {label}
              </span>
            ))}
          </div>
          <div className="grid grid-rows-7 gap-[3px]">
            {WEEKDAYS.map((day, index) => (
              <span key={index} className="leading-none">
                {day}
              </span>
            ))}
          </div>
          <div
            className="grid grid-flow-col grid-rows-7 gap-[3px]"
            style={{ gridTemplateColumns: columns }}
          >
            {cells.map((cell) => (
              <i
                key={cell.date}
                data-day={cell.date}
                title={`${cell.date}: ${groupedNumber(cell.value, unit)}`}
                className="aspect-square rounded-[3px]"
                style={{
                  gridColumnStart: cell.week + 1,
                  gridRowStart: cell.weekday + 1,
                  backgroundColor: LEVEL_FILL[cell.level],
                }}
              />
            ))}
          </div>
        </div>
        <div aria-hidden="true" className="flex items-center justify-end gap-1">
          <span>Less</span>
          {([0, 1, 2, 3, 4] as const).map((level) => (
            <i
              key={level}
              className="size-2.5 rounded-[2px]"
              style={{ backgroundColor: LEVEL_FILL[level] }}
            />
          ))}
          <span>More</span>
        </div>
      </div>
    </KindFigure>
  );
}

function monthStarts(cells: CalendarCell[]) {
  const starts: { week: number; label: string }[] = [];
  for (const cell of cells) {
    if (!cell.date.endsWith("-01") && cell !== cells[0]) continue;
    if (starts.at(-1)?.week === cell.week) continue;

    starts.push({
      week: cell.week,
      label: MONTH.format(new Date(`${cell.date}T00:00:00Z`)),
    });
  }

  return starts;
}
