import { Bar, BarChart } from "recharts";

import type { MetricResult } from "@/api/custom-client";
import { points, seriesConfig } from "@/components/custom/chart-data";
import { unitFor } from "@/components/custom/chart-format";
import { ChartFrame, SeriesAxes } from "@/components/custom/chart-frame";

export interface CustomBarChartProps {
  result: MetricResult;
  x: string;
  y: string;
}

/** A count per category — the shape most questions about "per" answer with. */
export function CustomBarChart({ result, x, y }: CustomBarChartProps) {
  const data = points(result, [x, y]);

  return (
    <ChartFrame config={seriesConfig(y)} testId="custom-bar-chart">
      <BarChart data={data} margin={{ top: 8, right: 12, left: 0, bottom: 0 }}>
        <SeriesAxes
          x={x}
          categories={data.map((point) => point[x])}
          unit={unitFor(result.percents, y)}
        />
        {/* Without a cap, one category stretches into a slab the width of the
            card. */}
        <Bar
          dataKey={y}
          fill={`var(--color-${y})`}
          radius={[4, 4, 0, 0]}
          maxBarSize={48}
        />
      </BarChart>
    </ChartFrame>
  );
}
