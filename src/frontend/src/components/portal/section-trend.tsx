import { ComingSoon } from "@/components/widgets/coming-soon";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  CartesianGrid,
  ChartArea,
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartLine,
  ChartTooltip,
  ChartTooltipContent,
  ComposedChart,
  ReferenceLine,
  type ChartConfig,
  XAxis,
  YAxis,
} from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";

/** One plotted series: its metric key, its label, and how to draw it. */
export type SectionTrendSeries = {
  key: string;
  label: string;
  type?: "line" | "area" | "stacked-area";
  yAxisId?: "left" | "right";
};

/**
 * One bucket. A series key is ABSENT when that series has no reading for the
 * bucket — never zero, so the line shows a gap instead of a measurement.
 */
export type SectionTrendPoint = {
  date: string;
  [seriesKey: string]: number | string;
};

export interface SectionTrendProps {
  title: string;
  description?: string;
  series: SectionTrendSeries[];
  data: SectionTrendPoint[];
  targetLine?: { value: number; label: string };
  /**
   * Force the right axis on. Normally unnecessary — it is rendered whenever a
   * series asks for it — but a caller can pin it so the plot area does not
   * shift when a right-axis series drops out of the data.
   */
  rightAxis?: boolean;
  height?: number;
  isPending?: boolean;
  isError?: boolean;
  onRetry?: () => void;
}

const DEFAULT_CHART_KEYS = ["chart-1", "chart-2", "chart-3", "chart-4", "chart-5"];

/** The shared trend card: one chart, the same states, wherever a trend appears. */
export function SectionTrend({
  title,
  description,
  series,
  data,
  targetLine,
  rightAxis,
  height = 180,
  isPending,
  isError,
  onRetry,
}: SectionTrendProps) {
  if (isPending) {
    return <Skeleton className="h-48 w-full rounded-lg" />;
  }
  if (isError) {
    return (
      <ComingSoon
        state="error"
        label={`${title} — unable to load`}
        onRetry={onRetry}
      />
    );
  }
  if (data.length === 0) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-semibold">{title}</CardTitle>
          <CardDescription>No trend data yet.</CardDescription>
        </CardHeader>
      </Card>
    );
  }

  // Series keys are metric keys like "collab.messages_sent". A dot is illegal
  // in a CSS custom-property name (so `--color-<key>` never resolves → invisible
  // strokes) and makes recharts treat the dataKey as a nested path. Remap every
  // series to a safe, collision-free alias and rewrite the data rows to match.
  const safeKeys = series.map((s, i) => `s${i}_${s.key.replace(/[^a-zA-Z0-9_-]/g, "_")}`);
  const safeSeries = series.map((s, i) => ({ ...s, key: safeKeys[i] }));
  const safeData = data.map((row) => {
    const next: SectionTrendPoint = { date: row.date as string };
    series.forEach((s, i) => {
      const v = row[s.key];
      if (v != null) next[safeKeys[i]] = v;
    });
    return next;
  });

  // Derived, not just taken from the prop: a series with `yAxisId: "right"`
  // and no right YAxis rendered is dropped by recharts without a word.
  const needsRightAxis =
    rightAxis || safeSeries.some((s) => s.yAxisId === "right");

  const config: ChartConfig = Object.fromEntries(
    safeSeries.map((s, i) => [
      s.key,
      { label: s.label, color: `var(--${DEFAULT_CHART_KEYS[i % DEFAULT_CHART_KEYS.length]})` },
    ]),
  );

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-semibold">{title}</CardTitle>
        {description ? (
          <CardDescription className="text-xs">{description}</CardDescription>
        ) : null}
      </CardHeader>
      <CardContent>
        <ChartContainer
          config={config}
          className="w-full"
          style={{ height }}
        >
          <ComposedChart data={safeData} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
            <XAxis
              dataKey="date"
              tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
              tickLine={false}
              axisLine={false}
              interval="preserveStartEnd"
            />
            <YAxis
              yAxisId="left"
              tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
              tickLine={false}
              axisLine={false}
            />
            {needsRightAxis ? (
              <YAxis
                yAxisId="right"
                orientation="right"
                tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
                tickLine={false}
                axisLine={false}
                width={28}
              />
            ) : null}
            <ChartTooltip content={<ChartTooltipContent />} />
            <ChartLegend content={<ChartLegendContent />} />
            {targetLine ? (
              <ReferenceLine
                yAxisId="left"
                y={targetLine.value}
                stroke="var(--success)"
                strokeDasharray="4 4"
                label={{
                  value: targetLine.label,
                  position: "right",
                  fontSize: 10,
                  fill: "var(--success)",
                }}
              />
            ) : null}
            {safeSeries.map((s) => {
              const yAxisId = s.yAxisId ?? "left";
              const color = `var(--color-${s.key})`;
              if (s.type === "area" || s.type === "stacked-area") {
                return (
                  <ChartArea
                    key={s.key}
                    yAxisId={yAxisId}
                    dataKey={s.key}
                    name={s.label}
                    stackId={s.type === "stacked-area" ? "stack" : undefined}
                    stroke={color}
                    fill={color}
                    fillOpacity={0.25}
                    strokeWidth={2}
                  />
                );
              }
              return (
                <ChartLine
                  key={s.key}
                  yAxisId={yAxisId}
                  dataKey={s.key}
                  name={s.label}
                  stroke={color}
                  strokeWidth={2}
                />
              );
            })}
          </ComposedChart>
        </ChartContainer>
      </CardContent>
    </Card>
  );
}
