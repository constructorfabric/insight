import { useMemo } from "react";

import { Pending } from "@/components/portal/domain-lens-view";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { Skeleton } from "@/components/ui/skeleton";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import type {
  MetricCollectionConfig,
  NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { pickTrendBucket } from "@/lib/portal/trend-data";
import {
  tenantSectionMetricKeys,
  type TenantLensConfig,
  type TenantSectionSpec,
} from "@/lib/portal/lens-configs";
import { useMetricCollection } from "@/queries/metric-results";

import { CompositionSection } from "./composition";
import { HistogramSection } from "./histogram";
import { TileRow } from "./tiles";
import { TrendSection } from "./trend";
import { tenantData } from "./data";

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
  const collection = useMemo<MetricCollectionConfig>(
    () => buildCollection(config, bucket),
    [config, bucket]
  );

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

/** The views each metric needs, unioned across the lens's sections. */
function buildCollection(
  config: TenantLensConfig,
  bucket: NonNullable<ReturnType<typeof pickTrendBucket>>
): MetricCollectionConfig {
  const views = new Map<
    string,
    {
      period: boolean;
      timeseries: boolean;
      breakdown: string[];
      histogram: boolean;
    }
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
        ...(v.timeseries ? [{ view: "timeseries" as const, bucket }] : []),
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
