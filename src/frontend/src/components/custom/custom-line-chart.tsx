import { Line, LineChart } from "recharts";

import type { MetricResult } from "@/api/custom-client";
import { points, seriesConfig } from "@/components/custom/chart-data";
import { unitFor } from "@/components/custom/chart-format";
import { ChartFrame, SeriesAxes } from "@/components/custom/chart-frame";

export interface CustomLineChartProps {
  result: MetricResult;
  x: string;
  y: string;
}

export function CustomLineChart({ result, x, y }: CustomLineChartProps) {
  const data = points(result, [x, y]);

  return (
    <ChartFrame config={seriesConfig(y)} testId="custom-line-chart">
      <LineChart data={data} margin={{ top: 8, right: 12, left: 0, bottom: 0 }}>
        <SeriesAxes
          x={x}
          categories={data.map((point) => point[x])}
          unit={unitFor(result.percents, y)}
        />
        <Line
          dataKey={y}
          type="monotone"
          stroke={`var(--color-${y})`}
          strokeWidth={2}
          dot={false}
          activeDot={{ r: 4 }}
        />
      </LineChart>
    </ChartFrame>
  );
}
