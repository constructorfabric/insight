import {
  BarChart,
  CartesianGrid,
  Cell,
  ChartBar,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  ReferenceLine,
  XAxis,
  YAxis,
  type ChartConfig,
} from "@/components/ui/chart";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { ChartEmpty } from "@/components/widgets/metric-views/chart-empty";
import { formatMetricNumber } from "@/lib/format";
import { forEntity, type NormalizedMetricResult } from "@/lib/metrics/collection";
import { STATUS_COLOR_VAR, statusVsMedian, type Status } from "@/lib/status";

export interface MetricHistogramProps {
  metric: NormalizedMetricResult;
  entityId: string;
}

const COUNT_KEY = "count";
const CONFIG: ChartConfig = { [COUNT_KEY]: { label: "Count" } };

/** Bars nearest the pivot are faintest; the tails saturate to this ceiling. */
const MIN_FILL_OPACITY = 0.35;
const MAX_FILL_OPACITY = 0.95;

type BinRow = {
  label: string;
  count: number;
  status: Status;
  opacity: number;
  straddlesPivot: boolean;
};

function directionText(metric: NormalizedMetricResult): string | null {
  if (metric.direction === "higher_is_better") return "Higher is better";
  if (metric.direction === "lower_is_better") return "Lower is better";
  return null;
}

/**
 * Classify each bin against the peer (cohort) median and shade it: bins on the
 * better side of the pivot are green, the worse side red (per the metric's
 * direction), and intensity ramps with distance from the pivot so the tails
 * read strongest. The bin the pivot falls inside is neutral. With no peer
 * median (thin cohort, suppressed) there's no honest comparison, so every bin
 * stays neutral.
 */
function buildRows(
  bins: Array<{ lo: number; hi: number; count: number }>,
  pivot: number | null,
  metric: NormalizedMetricResult,
): { rows: BinRow[]; pivotLabel: string | null } {
  const diverging = pivot != null && metric.direction !== "neutral";
  const higherIsBetter = metric.direction === "higher_is_better";
  const mids = bins.map((bin) => (bin.lo + bin.hi) / 2);
  const maxDistance = diverging
    ? Math.max(...mids.map((mid) => Math.abs(mid - pivot)), Number.EPSILON)
    : 1;

  let pivotLabel: string | null = null;
  const rows = bins.map((bin, index) => {
    const label = `${formatMetricNumber(bin.lo, metric.format)}–${formatMetricNumber(bin.hi, metric.format)}`;
    // The last bin's upper edge is inclusive (the max value lives there), so a
    // pivot exactly on it must straddle that bin rather than fall through.
    const isLastBin = index === bins.length - 1;
    const straddlesPivot =
      diverging &&
      bin.lo <= pivot &&
      (pivot < bin.hi || (isLastBin && pivot <= bin.hi));
    if (straddlesPivot) pivotLabel = label;
    const status: Status =
      !diverging || straddlesPivot
        ? "neutral"
        : statusVsMedian(mids[index]!, pivot, higherIsBetter);
    const intensity = diverging
      ? Math.abs(mids[index]! - pivot) / maxDistance
      : 0.6;
    return {
      label,
      count: bin.count,
      status,
      opacity: MIN_FILL_OPACITY + intensity * (MAX_FILL_OPACITY - MIN_FILL_OPACITY),
      straddlesPivot,
    };
  });
  return { rows, pivotLabel };
}

/**
 * One entity's own value distribution (server-owned bins), shaded by how each
 * range compares to the peer median. Headline is the person's own median; the
 * peer median is drawn as the pivot line the colors key off.
 */
export function MetricHistogram({ metric, entityId }: MetricHistogramProps) {
  const data = forEntity(metric, entityId);
  const bins = data.histogram[0]?.bins ?? [];
  const unit = metric.unit ? ` ${metric.unit}` : "";
  const ownMedian = data.value;
  const peerMedian = data.peer?.median ?? null;
  const direction = directionText(metric);

  // Shared header so an empty tile reads as the same chart, laid out to match
  // its populated neighbours in the grid rather than collapsing to a corner.
  const header = (
    <CardHeader className="pb-2">
      <div className="flex items-start justify-between gap-2">
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-sm font-semibold">{metric.label}</span>
          <span className="text-xs text-muted-foreground">
            {[
              direction,
              peerMedian != null
                ? `vs peer median ${formatMetricNumber(peerMedian, metric.format)}${unit}`
                : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
        </div>
        {ownMedian != null ? (
          <span className="shrink-0 text-xs text-muted-foreground">
            Median{" "}
            <span className="font-semibold text-foreground tabular-nums">
              {formatMetricNumber(ownMedian, metric.format)}
              {unit}
            </span>
          </span>
        ) : null}
      </div>
    </CardHeader>
  );

  if (bins.length === 0) {
    return (
      <Card className="shrink-0">
        {header}
        <CardContent>
          <ChartEmpty message="No values in this period" />
        </CardContent>
      </Card>
    );
  }

  const { rows, pivotLabel } = buildRows(bins, peerMedian, metric);

  return (
    <Card className="shrink-0">
      {header}
      <CardContent>
        <ChartContainer config={CONFIG} className="h-48 min-h-48 w-full">
          <BarChart
            data={rows}
            margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
          >
            <CartesianGrid
              vertical={false}
              strokeDasharray="3 3"
              stroke="var(--border)"
            />
            <XAxis
              dataKey="label"
              tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
              tickLine={false}
              axisLine={false}
              interval="preserveStartEnd"
            />
            <YAxis
              allowDecimals={false}
              width={28}
              tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
              tickLine={false}
              axisLine={false}
            />
            <ChartTooltip content={<ChartTooltipContent className="min-w-40" />} />
            <ChartBar dataKey={COUNT_KEY} name="Count" radius={[2, 2, 0, 0]}>
              {rows.map((row) => (
                <Cell
                  key={row.label}
                  fill={STATUS_COLOR_VAR[row.status]}
                  fillOpacity={row.opacity}
                />
              ))}
            </ChartBar>
            {pivotLabel ? (
              // The peer median: named in the subtitle ("vs peer median …")
              // and the only dashed line on the chart, so it needs no
              // on-chart label (which clipped at the top edge).
              <ReferenceLine
                x={pivotLabel}
                stroke="var(--foreground)"
                strokeDasharray="4 3"
              />
            ) : null}
          </BarChart>
        </ChartContainer>
      </CardContent>
    </Card>
  );
}
