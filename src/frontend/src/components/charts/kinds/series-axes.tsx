import { CartesianGrid, Tooltip, XAxis, YAxis } from "recharts";

import {
  categoryTick,
  compactNumber,
  shortDate,
  spansYears,
} from "@/components/custom/chart-format";

import { AXIS_TICK, GRID_STROKE, tooltipContent } from "../chart-style";

export function SeriesAxes({
  categories,
  unit = "",
}: {
  categories: unknown[];
  unit?: string;
}) {
  const withYear = spansYears(categories);

  return (
    <>
      <CartesianGrid stroke={GRID_STROKE} vertical={false} />
      <XAxis
        dataKey="x"
        tick={AXIS_TICK}
        tickLine={false}
        axisLine={false}
        minTickGap={28}
        tickFormatter={(value) => categoryTick(value, withYear)}
      />
      <YAxis
        tick={AXIS_TICK}
        tickLine={false}
        axisLine={false}
        width={46}
        tickFormatter={(value) => compactNumber(value, unit)}
      />
      <Tooltip
        content={tooltipContent({
          unit,
          labelFormatter: (label) => shortDate(label, true),
        })}
      />
    </>
  );
}
