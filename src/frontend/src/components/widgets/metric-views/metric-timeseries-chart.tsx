import { format } from "date-fns";
import type { DotItemDotProps } from "recharts";

import {
  BarChart,
  CartesianGrid,
  ChartBar,
  ChartContainer,
  ChartLine,
  ChartTooltip,
  ChartTooltipContent,
  LineChart,
  ReferenceArea,
  XAxis,
  YAxis,
  type ChartConfig,
} from "@/components/ui/chart";
import {
  buildMetricTimeseriesChartModel,
  commonNullRuns,
  type MetricTimeseriesChartModel,
} from "@/components/widgets/metric-views/metric-timeseries-chart-model";
import type { MetricTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries-model";
import { formatMetricNumber } from "@/lib/format";
import { percentShareLabels } from "@/lib/metrics/shares";
import type { MetricTimeseriesChartConfig } from "@/lib/metrics/timeseries-chart";
import { seriesColors } from "@/lib/series-colors";

export interface MetricTimeseriesChartProps {
  model: MetricTimeseriesModel;
  selectedMetricKey: string;
  multiMetric?: MetricTimeseriesChartConfig["multiMetric"];
  onEvidence?: (
    metricKey: string,
    columnKey: string,
    bucketStart: string | null
  ) => void;
}

function dateLabel(value: string, pattern: string): string {
  const [year, month, day] = value.split("-").map(Number);
  if (!year || !month || !day) return value;
  return format(new Date(year, month - 1, day), pattern);
}

function IsolatedPoint({
  cx,
  cy,
  index,
  points,
  stroke,
  value,
}: DotItemDotProps) {
  if (value == null || cx == null || cy == null) return null;
  if (points[index - 1]?.value != null || points[index + 1]?.value != null) {
    return null;
  }
  return <circle cx={cx} cy={cy} r={3} fill={stroke} />;
}

function TimeseriesXAxis({
  data,
  numeric = false,
}: {
  data: Array<{ bucketIndex: number; label: string }>;
  numeric?: boolean;
}) {
  return (
    <XAxis
      dataKey={numeric ? "bucketIndex" : "label"}
      type={numeric ? "number" : "category"}
      domain={numeric ? [-0.5, data.length - 0.5] : undefined}
      ticks={numeric ? data.map((item) => item.bucketIndex) : undefined}
      tickFormatter={
        numeric
          ? (value) => data[Number(value)]?.label ?? ""
          : (value) => String(value)
      }
      tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
      tickLine={false}
      axisLine={false}
      height={24}
      interval="preserveStartEnd"
    />
  );
}

export function MetricTimeseriesChart({
  model,
  selectedMetricKey,
  multiMetric = "selectable",
  onEvidence,
}: MetricTimeseriesChartProps) {
  const chartModel = buildMetricTimeseriesChartModel(
    model,
    selectedMetricKey,
    multiMetric
  );
  if (!chartModel) return null;

  const colors = seriesColors(
    chartModel.series.map((series) => series.colorSeed)
  );
  const config: ChartConfig = Object.fromEntries(
    chartModel.series.map((series) => [
      series.key,
      { label: series.label, color: colors[series.colorSeed] },
    ])
  );
  const data = model.buckets.map((bucketStart, bucketIndex) => ({
    bucketStart,
    bucketIndex,
    label: dateLabel(
      bucketStart,
      model.bucket === "month" ? "MMM yyyy" : "MMM d"
    ),
    tooltipLabel: dateLabel(bucketStart, "MMMM d, yyyy"),
    ...Object.fromEntries(
      chartModel.series.map((series) => [
        series.key,
        series.points.get(bucketStart) ?? null,
      ])
    ),
  }));
  const totals = chartModel.series.map((series) => series.total);
  const shares = percentShareLabels(totals.map((value) => value ?? 0));
  const nullRuns = chartModel.grouped
    ? []
    : commonNullRuns(
        model.buckets,
        chartModel.series.map((series) => series.points)
      );
  const chartContent = (
    <>
      <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
      <YAxis
        tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
        tickFormatter={(value) =>
          formatMetricNumber(Number(value), chartModel.valueMetric.format)
        }
        tickLine={false}
        axisLine={false}
        width={48}
      />
      <ChartTooltip
        content={
          <ChartTooltipContent
            className="min-w-48"
            labelFormatter={(_, payload) =>
              String(payload?.[0]?.payload?.tooltipLabel ?? "")
            }
          />
        }
      />
    </>
  );
  const openPoint = (
    series: MetricTimeseriesChartModel["series"][number],
    state: unknown
  ) => {
    const point = state as { payload?: { bucketStart?: string } };
    const bucketStart = point.payload?.bucketStart;
    const column = model.columns.find(
      (candidate) => candidate.key === series.columnKey
    );
    if (bucketStart && column && !column.remainder) {
      onEvidence?.(series.metricKey, series.columnKey, bucketStart);
    }
  };

  return (
    <div className="flex h-full flex-col px-4 pb-3 sm:px-6">
      <ChartContainer
        config={config}
        className="aspect-auto min-h-0 w-full flex-1"
      >
        {chartModel.grouped ? (
          <BarChart
            data={data}
            margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
          >
            {chartContent}
            <TimeseriesXAxis data={data} />
            {chartModel.series.map((series) => (
              <ChartBar
                key={series.key}
                dataKey={series.key}
                stackId={chartModel.valueMetric.metric_key}
                fill={`var(--color-${series.key})`}
                name={series.label}
                radius={[2, 2, 0, 0]}
                onClick={(point) => openPoint(series, point)}
              />
            ))}
          </BarChart>
        ) : (
          <LineChart
            data={data}
            margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
          >
            {chartContent}
            <TimeseriesXAxis data={data} numeric />
            {nullRuns.map((run) => (
              <ReferenceArea
                key={`${run.startIndex}-${run.endIndex}`}
                x1={run.startIndex - 0.5}
                x2={run.endIndex + 0.5}
                ifOverflow="hidden"
                fill="var(--muted)"
                fillOpacity={0.65}
                stroke="var(--border)"
                strokeOpacity={0.5}
                label={
                  run.endIndex > run.startIndex
                    ? {
                        value: "No data",
                        position: "insideTop",
                        fill: "var(--muted-foreground)",
                        fontSize: 10,
                      }
                    : undefined
                }
              />
            ))}
            {chartModel.series.map((series) => (
              <ChartLine
                key={series.key}
                type="monotone"
                dataKey={series.key}
                stroke={`var(--color-${series.key})`}
                strokeWidth={2}
                dot={IsolatedPoint}
                connectNulls={false}
                name={series.label}
                onClick={(point) => openPoint(series, point)}
              />
            ))}
          </LineChart>
        )}
      </ChartContainer>
      {chartModel.grouped || chartModel.series.length > 1 ? (
        <ul className="mt-3 flex max-h-16 shrink-0 flex-wrap gap-x-6 gap-y-1.5 overflow-y-auto">
          {chartModel.series.map((series, index) => (
            <li key={series.key} className="flex items-center gap-2 text-xs">
              <span
                aria-hidden
                className="size-2.5 shrink-0 rounded-[3px]"
                style={{ backgroundColor: colors[series.colorSeed] }}
              />
              <span className="font-medium">{series.label}</span>
              {chartModel.grouped ? (
                <span className="text-muted-foreground tabular-nums">
                  {totals[index] == null
                    ? "—"
                    : `${formatMetricNumber(totals[index], chartModel.valueMetric.format)}${chartModel.valueMetric.unit ? ` ${chartModel.valueMetric.unit}` : ""}${shares[index] ? ` · ${shares[index]}%` : ""}`}
                </span>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
