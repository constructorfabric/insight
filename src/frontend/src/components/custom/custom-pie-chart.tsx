import { Cell, Pie, PieChart } from "recharts";

import type { MetricResult } from "@/api/custom-client";
import {
  CHART_COLORS,
  type ChartConfig,
  points,
} from "@/components/custom/chart-data";
import {
  categoryTick,
  groupedNumber,
  unitFor,
} from "@/components/custom/chart-format";
import {
  ChartFrame,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/custom/chart-frame";

export interface CustomPieChartProps {
  result: MetricResult;
  label: string;
  value: string;
}

/** A share of a total: one slice per row, painted round the kit's palette. */
export function CustomPieChart({ result, label, value }: CustomPieChartProps) {
  const data = points(result, [label, value]);
  const unit = unitFor(result.percents, value);
  const config: ChartConfig = Object.fromEntries(
    data.map((slice, index) => [
      String(slice[label]),
      {
        label: categoryTick(slice[label]),
        color: `var(--chart-${(index % CHART_COLORS) + 1})`,
      },
    ])
  );

  return (
    <ChartFrame config={config} testId="custom-pie-chart">
      <PieChart>
        <ChartTooltip
          content={
            <ChartTooltipContent
              nameKey={label}
              hideLabel
              formatter={(sliced, name) =>
                `${name}  ${groupedNumber(sliced, unit)}`
              }
            />
          }
        />
        <ChartLegend content={<ChartLegendContent nameKey={label} />} />
        <Pie data={data} dataKey={value} nameKey={label}>
          {data.map((slice, index) => (
            <Cell
              key={String(slice[label])}
              fill={`var(--chart-${(index % CHART_COLORS) + 1})`}
            />
          ))}
        </Pie>
      </PieChart>
    </ChartFrame>
  );
}
