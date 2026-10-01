import { CartesianGrid, Tooltip, XAxis, YAxis } from "recharts";

import {
  categoryTick,
  compactNumber,
  shortDate,
  spansYears,
} from "@/components/custom/chart-format";

import { AXIS_TICK, GRID_STROKE, tooltipContent } from "../chart-style";
import { CategoryTick } from "../chrome";

export function SeriesAxes({
  categories,
  unit = "",
  everyTick = false,
}: {
  categories: unknown[];
  unit?: string;
  everyTick?: boolean;
}) {
  const withYear = spansYears(categories);
  const label = (value: unknown) => categoryTick(value, withYear);

  return (
    <>
      <CartesianGrid stroke={GRID_STROKE} vertical={false} />
      {everyTick ? (
        <XAxis
          dataKey="x"
          interval={0}
          tick={<CategoryTick anchor="end" angle={-90} format={label} />}
          tickLine={false}
          axisLine={false}
          height={withYear ? 84 : 52}
        />
      ) : (
        <XAxis
          dataKey="x"
          tick={AXIS_TICK}
          tickLine={false}
          axisLine={false}
          minTickGap={28}
          tickFormatter={label}
        />
      )}
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
