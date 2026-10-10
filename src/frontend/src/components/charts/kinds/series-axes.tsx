import { CartesianGrid, Tooltip, XAxis, YAxis } from "recharts";

import {
  categoryTick,
  compactNumber,
  shortDate,
  spansYears,
} from "@/components/custom/chart-format";

import {
  AXIS_TICK,
  CHAR_WIDTH,
  EVERY_TICK_LIMIT,
  tooltipContent,
} from "../chart-style";
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
      <CartesianGrid vertical={false} />
      {everyTick && categories.length <= EVERY_TICK_LIMIT ? (
        <XAxis
          dataKey="x"
          interval={0}
          tick={<CategoryTick anchor="end" angle={-90} format={label} />}
          tickLine={false}
          axisLine={false}
          height={Math.ceil((withYear ? 12 : 6) * CHAR_WIDTH) + 12}
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
