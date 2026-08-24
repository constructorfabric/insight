import { useMemo } from "react";

import {
  BarList,
  Delta,
  Pending,
  toBarRows,
  UNSPLIT_SEGMENT,
  type BarEntry,
} from "@/components/portal/domain-lens-view";
import {
  SectionTrend,
  type SectionTrendPoint,
  type SectionTrendSeries,
} from "@/components/portal/section-trend";
import { MetricName } from "@/components/widgets/metric-help-tooltip";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { Card, CardContent } from "@/components/ui/card";
import {
  BarChart,
  CartesianGrid,
  ChartBar,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  XAxis,
  YAxis,
  type ChartConfig,
} from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { formatMetricValue } from "@/lib/format";
import {
  forEntity,
  type MetricCollectionConfig,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { fmtCompact } from "@/lib/portal/metric-stats";
import { pickTrendBucket } from "@/lib/portal/trend-data";
import {
  tenantSectionMetricKeys,
  type TenantLensConfig,
  type TenantSectionSpec,
} from "@/lib/portal/lens-configs";
import { useMetricCollection } from "@/queries/metric-results";
import { TEXT_FIGURE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/**
 * One renderer for every tenant-grain lens (issue #2803): the entity is the
 * ORGANIZATION, so there is no roster, no peer view and no per-person split —
 * each value is one number. Sections come from the lens config; each
 * self-suppresses on degenerate data, and the whole tab collapses to an honest
 * "not ingested" state when no metric of the family is observed (rule 6).
 */
export function TenantLensView({ config }: { config: TenantLensConfig }) {
  const { period, dateRange } = usePortalPeriod();
  // One entity → the projected-row budget is all headroom; the picker still
  // guards a pathological custom range, and month always fits.
  const bucket = pickTrendBucket(1, dateRange) ?? "month";
  const collection = useMemo<MetricCollectionConfig>(() => {
    const views = new Map<
      string,
      { period: boolean; timeseries: boolean; breakdown: string[]; histogram: boolean }
    >();
    const need = (key: string) => {
      const got = views.get(key) ?? {
        period: false,
        timeseries: false,
        breakdown: [],
        histogram: false,
      };
      views.set(key, got);
      return got;
    };
    for (const s of config.sections) {
      switch (s.kind) {
        case "headline":
        case "stat-tiles":
          for (const k of s.metrics) need(k).period = true;
          break;
        case "trend":
          for (const k of s.metrics) need(k).timeseries = true;
          break;
        case "composition": {
          const dims = [s.dimension, ...(s.splitBy ? [s.splitBy] : [])];
          need(s.metric).breakdown.push(...dims);
          break;
        }
        case "histogram":
          need(s.metric).histogram = true;
          break;
        default: {
          const _exhaustive: never = s;
          throw new Error(`Unhandled section: ${JSON.stringify(_exhaustive)}`);
        }
      }
    }
    return {
      metrics: [...views.entries()].map(([key, v]) => ({
        key,
        views: [
          ...(v.period ? [{ view: "period" as const }] : []),
          ...(v.timeseries
            ? [{ view: "timeseries" as const, bucket }]
            : []),
          ...(v.breakdown.length
            ? [
                {
                  view: "breakdown" as const,
                  dimensions: [...new Set(v.breakdown)],
                },
              ]
            : []),
          ...(v.histogram ? [{ view: "histogram" as const }] : []),
        ],
      })),
    };
  }, [config, bucket]);

  const data = useMetricCollection(
    collection,
    { type: "tenant" },
    dateRange,
    { previousPeriod: period }
  );

  if (data.isPending) {
    return (
      <div className="flex flex-col gap-6 p-6">
        <Skeleton className="h-8 w-64" />
        <div className="grid grid-cols-[repeat(auto-fit,minmax(14rem,1fr))] gap-3">
          <Skeleton className="h-28" />
          <Skeleton className="h-28" />
          <Skeleton className="h-28" />
        </div>
        <Skeleton className="h-64" />
      </div>
    );
  }
  if (data.isError) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          label={`${config.title} — unable to load`}
          onRetry={data.refetch}
        />
      </div>
    );
  }
  // Observed = any lens metric returned a period value or any rows at all;
  // "no data yet" and "not collected" must not both render as zeros.
  const observed = tenantSectionMetricKeys(config).some((key) => {
    const r = data.byKey.get(key);
    if (!r) return false;
    const e = tenantData(r);
    return (
      e.value != null ||
      e.series.length > 0 ||
      e.breakdown.length > 0 ||
      e.histogram.length > 0
    );
  });
  if (!observed) return <Pending label={config.notIngested} />;

  return (
    <div className="flex flex-col gap-6 p-6">
      <header>
        <h2 className="text-lg font-semibold">{config.title}</h2>
        {config.tagline ? (
          <p className="text-sm text-muted-foreground">{config.tagline}</p>
        ) : null}
      </header>
      {config.sections.map((section, index) => (
        <TenantSection
          key={`${section.kind}-${index}`}
          section={section}
          data={data.byKey}
          previous={data.previousByKey}
        />
      ))}
    </div>
  );
}

/** The single tenant row of each view — entity_id is opaque here (the backend
 *  stamps the organization id), so "the only entity" IS the selection. */
function tenantData(r: NormalizedMetricResult) {
  const id = firstEntityId(r);
  return forEntity(r, id ?? "");
}

function firstEntityId(r: NormalizedMetricResult): string | null {
  return (
    r.period?.values[0]?.entity_id ??
    r.timeseries?.series[0]?.entity_id ??
    r.breakdown?.values[0]?.entity_id ??
    r.histogram?.values[0]?.entity_id ??
    null
  );
}

function TenantSection({
  section,
  data,
  previous,
}: {
  section: TenantSectionSpec;
  data: Map<string, NormalizedMetricResult>;
  previous: Map<string, NormalizedMetricResult> | null;
}) {
  switch (section.kind) {
    case "headline":
      return (
        <TileRow
          metrics={section.metrics}
          data={data}
          previous={previous}
          minWidth="14rem"
        />
      );
    case "stat-tiles":
      return (
        <section className="flex flex-col gap-3">
          <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
            {section.title}
          </p>
          <TileRow
            metrics={section.metrics}
            data={data}
            previous={previous}
            minWidth="11rem"
          />
        </section>
      );
    case "trend":
      return <TrendSection section={section} data={data} />;
    case "composition":
      return <CompositionSection section={section} data={data} />;
    case "histogram":
      return <HistogramSection section={section} data={data} />;
    default: {
      const _exhaustive: never = section;
      throw new Error(`Unhandled section: ${JSON.stringify(_exhaustive)}`);
    }
  }
}

/* ── headline & stat tiles: one org-wide value with its delta ─────────── */

function TileRow({
  metrics,
  data,
  previous,
  minWidth,
}: {
  metrics: readonly string[];
  data: Map<string, NormalizedMetricResult>;
  previous: Map<string, NormalizedMetricResult> | null;
  minWidth: string;
}) {
  const tiles = metrics
    .map((key) => {
      const r = data.get(key);
      if (!r) return null;
      const now = tenantData(r).value;
      if (now == null) return null;
      const prevResult = previous?.get(key);
      const prev = prevResult ? tenantData(prevResult).value : null;
      return { key, r, now, prev };
    })
    .filter((tile): tile is NonNullable<typeof tile> => tile != null);
  if (!tiles.length) return null;

  return (
    <div
      className="grid gap-3"
      style={{
        gridTemplateColumns: `repeat(auto-fit, minmax(${minWidth}, 1fr))`,
      }}
    >
      {tiles.map((tile) => (
        <Card key={tile.key}>
          <CardContent className="p-4">
            <div className="flex items-center justify-between gap-2">
              <MetricName
                metric={tile.r}
                className="text-xs font-medium text-muted-foreground"
              />
              <Delta
                now={tile.now}
                prev={tile.prev}
                direction={tile.r.direction}
              />
            </div>
            <div className={cn("mt-1", TEXT_FIGURE)}>
              {formatMetricValue(tile.now, tile.r.format, tile.r.unit)}
            </div>
            <div className="text-xs text-muted-foreground">org-wide</div>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

/* ── trend: buckets of the single org series ─────────────────────────── */

function TrendSection({
  section,
  data,
}: {
  section: Extract<TenantSectionSpec, { kind: "trend" }>;
  data: Map<string, NormalizedMetricResult>;
}) {
  const series: SectionTrendSeries[] = [];
  const byDate = new Map<string, SectionTrendPoint>();
  for (const key of section.metrics) {
    const r = data.get(key);
    if (!r) continue;
    series.push({
      key,
      label: r.short_label ?? r.label,
      type: section.plot ?? "line",
    });
    for (const point of tenantData(r).series[0]?.points ?? []) {
      if (point.value == null) continue;
      const row = byDate.get(point.bucket_start) ?? {
        date: point.bucket_start,
      };
      row[key] = point.value;
      byDate.set(point.bucket_start, row);
    }
  }
  const points = [...byDate.values()].sort((a, b) =>
    a.date < b.date ? -1 : 1
  );
  // One bucket draws as a dot pretending to be a trend — say nothing instead.
  if (!series.length || points.length < 2) return null;

  return (
    <SectionTrend
      title={section.title}
      description={section.description}
      series={series}
      data={points}
    />
  );
}

/* ── composition: the org total cut by one dimension ─────────────────── */

function CompositionSection({
  section,
  data,
}: {
  section: Extract<TenantSectionSpec, { kind: "composition" }>;
  data: Map<string, NormalizedMetricResult>;
}) {
  const r = data.get(section.metric);
  if (!r) return null;
  const bucket = new Map<string, BarEntry>();
  for (const row of tenantData(r).breakdown) {
    const dim = row.dimensions.find((d) => d.key === section.dimension);
    if (!dim?.value || row.value == null || row.value <= 0) continue;
    const running = bucket.get(dim.value);
    const split = running?.split ?? (section.splitBy ? new Map() : undefined);
    if (split && section.splitBy) {
      const by = row.dimensions.find((d) => d.key === section.splitBy);
      const seed = by?.value || UNSPLIT_SEGMENT;
      const seen = split.get(seed);
      split.set(seed, {
        seed,
        label:
          section.segmentLabels?.[seed] ??
          (by?.label?.trim() || seen?.label || seed),
        value: (seen?.value ?? 0) + row.value,
      });
    }
    bucket.set(dim.value, {
      label: dim.label?.trim() || running?.label || dim.value,
      value: (running?.value ?? 0) + row.value,
      split,
    });
  }
  const rows = toBarRows(bucket);
  // A single 100%-share bar is an empty shell (rule 11).
  if (rows.length < 2) return null;

  return (
    <BarList
      title={section.title}
      rows={rows}
      format={r.format}
      unit={r.unit}
    />
  );
}

/* ── histogram: per-run distribution served by the backend ───────────── */

const HISTOGRAM_CONFIG: ChartConfig = { count: { label: "Runs" } };

function HistogramSection({
  section,
  data,
}: {
  section: Extract<TenantSectionSpec, { kind: "histogram" }>;
  data: Map<string, NormalizedMetricResult>;
}) {
  const r = data.get(section.metric);
  const bins = r ? (tenantData(r).histogram[0]?.bins ?? []) : [];
  if (!r || bins.length === 0) return null;
  const rows = bins.map((bin) => ({
    label: fmtCompact(bin.lo),
    range: `${fmtCompact(bin.lo)}–${fmtCompact(bin.hi)}`,
    count: bin.count,
  }));
  const total = bins.reduce((sum, bin) => sum + bin.count, 0);

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {section.title} · {total} runs
      </p>
      <Card>
        <CardContent className="p-4">
          <p className="mb-3 text-xs text-muted-foreground">{section.caption}</p>
          <ChartContainer config={HISTOGRAM_CONFIG} className="h-56 w-full">
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
              <ChartTooltip
                content={
                  <ChartTooltipContent
                    className="min-w-40"
                    labelFormatter={(_, payload) =>
                      (payload?.[0]?.payload as { range?: string } | undefined)
                        ?.range ?? ""
                    }
                  />
                }
              />
              <ChartBar
                dataKey="count"
                name="Runs"
                radius={[2, 2, 0, 0]}
                fill="var(--chart-1)"
              />
            </BarChart>
          </ChartContainer>
          <p className="mt-1 text-center text-xs text-muted-foreground">
            {r.short_label ?? r.label}
            {r.unit ? ` (${r.unit})` : ""}
          </p>
        </CardContent>
      </Card>
    </section>
  );
}
