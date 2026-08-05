import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { ChartEmpty } from "@/components/widgets/metric-views/chart-empty";
import { MetricCardActions } from "@/components/widgets/metric-views/metric-card-actions";
import {
  dimensionColorSeed,
  dimensionLabel,
  dimensionSeriesKey,
} from "@/components/widgets/metric-views/dimension-series";
import { formatMetricValue } from "@/lib/format";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { percentShareLabels } from "@/lib/metrics/shares";
import { seriesColors } from "@/lib/series-colors";
import { evidenceSelection } from "@/api/metric-drilldown-client";

export interface MetricBreakdownProps {
  metric: NormalizedMetricResult;
  entityId: string;
}

function num(value: number | null | undefined): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

/** Proportional composition strip over the breakdown view's dimension groups. */
export function MetricBreakdown({ metric, entityId }: MetricBreakdownProps) {
  const evidence = metric.drilldown
    ? evidenceSelection(
        metric.selection,
        entityId,
        undefined,
        undefined,
        metric.breakdown?.dimensions
      )
    : null;
  const rows = forEntity(metric, entityId)
    .breakdown.filter((row) => num(row.value) > 0)
    .map((row) => ({
      key: dimensionSeriesKey(row.dimensions),
      colorSeed: dimensionColorSeed(row.dimensions),
      label: dimensionLabel(row.dimensions),
      value: num(row.value),
    }))
    .sort((a, b) => b.value - a.value);

  if (rows.length === 0) {
    return (
      <Card className="relative shrink-0">
        <MetricCardActions evidence={evidence} label={metric.label} />
        <CardHeader className="pr-12 pb-2">
          <CardTitle className="text-sm font-semibold">
            {metric.label}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <ChartEmpty message="No composition data yet" className="min-h-32" />
        </CardContent>
      </Card>
    );
  }

  const dimensions = metric.breakdown?.dimensions ?? [];
  const colorsBySeed = seriesColors(rows.map((row) => row.colorSeed));
  const total = rows.reduce((sum, row) => sum + row.value, 0);
  // Ribbon widths use the exact fraction; displayed percents use share
  // labels that sum to exactly 100 (no 99%/101% in the legend, and a
  // nonzero part never reads 0%).
  const shares = percentShareLabels(rows.map((row) => row.value));
  const items = rows.map((row, index) => ({
    ...row,
    color: colorsBySeed[row.colorSeed],
    pct: total > 0 ? (row.value / total) * 100 : 0,
    share: shares[index] ?? "0",
    formatted: formatMetricValue(row.value, metric.format, metric.unit),
  }));

  return (
    <Card className="relative shrink-0">
      <MetricCardActions evidence={evidence} label={metric.label} />
      <CardHeader className="pr-12 pb-2">
        <CardTitle className="text-sm font-semibold">{metric.label}</CardTitle>
        <CardDescription className="text-xs">
          {dimensions.length > 0
            ? `Period total by ${dimensions.join(" / ")}`
            : "Period total"}
        </CardDescription>
      </CardHeader>
      <CardContent>
        {/* Proportional ribbon carries color only; every label/value/percent
            lives in the legend below, so a narrow slice never clips its text. */}
        <div className="flex h-4 w-full overflow-hidden rounded-full bg-muted">
          {items.map((item) => (
            <div
              key={item.key}
              className="h-full min-w-[2px] border-r border-background last:border-r-0"
              style={{ width: `${item.pct}%`, backgroundColor: item.color }}
              title={`${item.label}: ${item.formatted} (${item.share}%)`}
            />
          ))}
        </div>
        <ul className="mt-3 grid grid-cols-1 gap-x-6 gap-y-1.5 sm:grid-cols-2">
          {items.map((item) => (
            <li key={item.key} className="flex items-center gap-2 text-xs">
              <span
                aria-hidden
                className="size-2.5 shrink-0 rounded-[3px]"
                style={{ backgroundColor: item.color }}
              />
              <span className="min-w-0 flex-1 truncate font-medium">
                {item.label}
              </span>
              <span className="shrink-0 text-muted-foreground tabular-nums">
                {item.formatted}
              </span>
              <span className="w-8 shrink-0 text-right text-muted-foreground tabular-nums">
                {item.share}%
              </span>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}
