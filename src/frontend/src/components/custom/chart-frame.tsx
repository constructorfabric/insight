import type { ReactElement } from "react";
import { CartesianGrid, XAxis, YAxis } from "recharts";

import {
  ChartContainer,
  type ChartConfig,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
} from "@gears-frontx/ui-kit";

import {
  categoryTick,
  compactNumber,
  groupedNumber,
  shortDate,
  spansYears,
} from "@/components/custom/chart-format";

/**
 * The chrome every chart shares: the kit's container, its palette, its
 * tooltip, and axes that print what they draw rather than the raw column.
 *
 * Each kind contributes only its series.
 */
export function ChartFrame({
  config,
  testId,
  children,
}: {
  config: ChartConfig;
  testId: string;
  children: ReactElement;
}) {
  return (
    <div data-testid={testId} className="h-64 w-full">
      <ChartContainer config={config} className="h-full w-full">
        {children}
      </ChartContainer>
    </div>
  );
}

/**
 * The x and y axes for a series chart, plus the grid and the tooltip.
 *
 * A day column arrives as `2024-11-23 00:00:00.000` and a sha as 40
 * characters; printed raw they crowd the axis until it is unreadable.
 */
export function SeriesAxes({
  x,
  categories,
  unit = "",
}: {
  x: string;
  categories: unknown[];
  /** What the y values are counted in — `%` for a rate. */
  unit?: string;
}) {
  const withYear = spansYears(categories);

  return (
    <>
      <CartesianGrid vertical={false} strokeDasharray="4 4" />
      <XAxis
        dataKey={x}
        tickLine={false}
        axisLine={false}
        tickMargin={8}
        minTickGap={24}
        interval="preserveStartEnd"
        tickFormatter={(value) => categoryTick(value, withYear)}
      />
      <YAxis
        tickLine={false}
        axisLine={false}
        width={44}
        tickCount={5}
        tickFormatter={(value) => compactNumber(value, unit)}
      />
      <ChartTooltip
        cursor={false}
        content={
          <ChartTooltipContent
            indicator="dot"
            labelFormatter={(label) => shortDate(label, true)}
            formatter={(value, name) =>
              `${name}  ${groupedNumber(value, unit)}`
            }
          />
        }
      />
    </>
  );
}

export { ChartLegend, ChartLegendContent, ChartTooltip, ChartTooltipContent };
