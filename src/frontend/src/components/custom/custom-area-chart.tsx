import { Area, AreaChart } from "recharts";

import type { MetricResult } from "@/api/custom-client";
import { points, seriesConfig } from "@/components/custom/chart-data";
import { unitFor } from "@/components/custom/chart-format";
import { ChartFrame, SeriesAxes } from "@/components/custom/chart-frame";

export interface CustomAreaChartProps {
  result: MetricResult;
  x: string;
  y: string;
}

export function CustomAreaChart({ result, x, y }: CustomAreaChartProps) {
  const data = points(result, [x, y]);
  const gradient = `fill-${y}`;

  return (
    <ChartFrame config={seriesConfig(y)} testId="custom-area-chart">
      <AreaChart data={data} margin={{ top: 8, right: 12, left: 0, bottom: 0 }}>
        <defs>
          <linearGradient id={gradient} x1="0" y1="0" x2="0" y2="1">
            <stop
              offset="5%"
              stopColor={`var(--color-${y})`}
              stopOpacity={0.7}
            />
            <stop
              offset="95%"
              stopColor={`var(--color-${y})`}
              stopOpacity={0.05}
            />
          </linearGradient>
        </defs>
        <SeriesAxes
          x={x}
          categories={data.map((point) => point[x])}
          unit={unitFor(result.percents, y)}
        />
        <Area
          dataKey={y}
          type="monotone"
          stroke={`var(--color-${y})`}
          strokeWidth={2}
          fill={`url(#${gradient})`}
        />
      </AreaChart>
    </ChartFrame>
  );
}
