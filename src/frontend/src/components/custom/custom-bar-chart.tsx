import { BarChart, LabelList } from "recharts";

import type { MetricResult } from "@/api/custom-client";
import {
  type ChartConfig,
  pointClick,
  points,
  seriesConfig,
  stackColor,
  stacks,
} from "@/components/custom/chart-data";
import { compactNumber, unitFor } from "@/components/custom/chart-format";
import {
  ChartFrame,
  ChartLegend,
  ChartLegendContent,
  SeriesAxes,
} from "@/components/custom/chart-frame";
import { ChartBar } from "@/components/ui/chart";

export interface CustomBarChartProps {
  result: MetricResult;
  x: string;
  y: string;
  series?: string;
  labels?: boolean;
  fillHeight?: boolean;
  onPointClick?: () => void;
}

/** A count per category — the shape most questions about "per" answer with. */
export function CustomBarChart({
  result,
  x,
  y,
  series,
  labels = false,
  fillHeight,
  onPointClick,
}: CustomBarChartProps) {
  const unit = unitFor(result.percents, y);
  const stacked = series ? stacks(result, x, y, series) : null;
  const data = stacked?.rows ?? points(result, [x, y]);
  const segments = stacked?.series ?? [{ key: y, label: y }];
  const config: ChartConfig = stacked
    ? Object.fromEntries(
        segments.map(({ key, label }, index) => [
          key,
          { label, color: stackColor(index) },
        ])
      )
    : seriesConfig(y);

  return (
    <ChartFrame
      config={config}
      testId="custom-bar-chart"
      fillHeight={fillHeight}
    >
      <BarChart
        data={data}
        margin={{ top: labels ? 20 : 8, right: 12, left: 0, bottom: 0 }}
        onClick={pointClick(onPointClick)}
        className={onPointClick ? "cursor-pointer" : undefined}
      >
        <SeriesAxes
          x={x}
          categories={data.map((point) => point[x])}
          unit={unit}
        />
        {stacked && (
          <ChartLegend
            content={<ChartLegendContent className="flex-wrap gap-y-1" />}
            itemSorter={null}
          />
        )}
        {segments.map(({ key, label }) => (
          <ChartBar
            key={key}
            dataKey={key}
            name={label}
            fill={`var(--color-${key})`}
            stackId={stacked ? "stack" : undefined}
            radius={stacked ? 0 : undefined}
          >
            {labels && (
              <LabelList
                {...(stacked
                  ? {
                      valueAccessor: (entry: {
                        payload: Record<string, unknown>;
                      }) =>
                        entry.payload[stacked.top] === key
                          ? (entry.payload[stacked.total] as number)
                          : undefined,
                    }
                  : { dataKey: y })}
                position="top"
                className="fill-foreground"
                fontSize={11}
                formatter={(value) => compactNumber(value, unit)}
              />
            )}
          </ChartBar>
        ))}
      </BarChart>
    </ChartFrame>
  );
}
