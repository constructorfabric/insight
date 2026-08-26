import { useMemo } from "react";

import { Pending } from "@/components/portal/domain-lens-view";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { Skeleton } from "@/components/ui/skeleton";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import type { MetricCollectionConfig } from "@/lib/metrics/collection";
import { pickTrendBucket } from "@/lib/portal/trend-data";
import type {
  TenantLensConfig,
  TenantSectionSpec,
} from "@/lib/portal/lens-configs";
import {
  collectionSetPending,
  useMetricCollection,
  useMetricCollectionSet,
} from "@/queries/metric-results";

import { CalloutPairSection } from "./callout-pair";
import { CompositionSection } from "./composition";
import { CumulativeSection } from "./cumulative";
import { DecompositionSection } from "./decomposition";
import { DumbbellSection } from "./dumbbell";
import { HeatmapHoursSection } from "./heatmap-hours";
import { HistogramSection } from "./histogram";
import { HourColumnsSection } from "./hour-columns";
import { MarginalImpactSection } from "./marginal-impact";
import { ScatterSection } from "./scatter-plot";
import { SlopeSection, MomentumSection } from "./slope-momentum";
import { SmallMultiplesSection } from "./small-multiples";
import { StackedTrendSection } from "./stacked-trend";
import { TileRow } from "./tiles";
import { TrendSection } from "./trend";
import { VerdictTableSection } from "./verdict-table";
import { splitDateRange } from "./derived";
import {
  planTenantRequests,
  sectionNeeds,
  type ResolveView,
} from "./plan";
import { tenantData } from "./data";

/**
 * One renderer for every tenant-grain lens (issue #2803): the entity is the
 * ORGANIZATION, so there is no roster, no peer view and no per-person split —
 * each value is one number. Sections come from the lens config; each
 * self-suppresses on degenerate data, and the whole tab collapses to an honest
 * "not ingested" state when no metric of the family is observed (rule 6).
 *
 * The backend takes one view of each kind per metric per request, so the
 * section needs are packed into several concurrent requests by `plan.ts`:
 * collection 0 (with the previous-period twin the tiles need), the overflow
 * collections, and a first-/second-half pair for the slope/momentum sections.
 */
export function TenantLensView({ config }: { config: TenantLensConfig }) {
  const { period, dateRange } = usePortalPeriod();
  // One entity → the projected-row budget is all headroom; the picker still
  // guards a pathological custom range, and month always fits.
  const bucket = pickTrendBucket(1, dateRange) ?? "month";
  const plan = useMemo(
    () => planTenantRequests(config, bucket),
    [config, bucket]
  );
  const halfRanges = useMemo(() => splitDateRange(dateRange), [dateRange]);

  const main = useMetricCollection(
    plan.collections[0],
    { type: "tenant" },
    dateRange,
    { previousPeriod: period }
  );
  const extraEntries = useMemo(
    () =>
      plan.collections
        .slice(1)
        .map((collection, index) => ({ key: extraKey(index + 1), collection })),
    [plan]
  );
  const extras = useMetricCollectionSet(
    extraEntries,
    { type: "tenant" },
    dateRange
  );
  // Halves stay disabled (empty collection) when the window is too short to
  // split — the slope/momentum sections then suppress themselves.
  const halvesCollection = halfRanges ? plan.halves : EMPTY_COLLECTION;
  const firstHalf = useMetricCollection(
    halvesCollection,
    { type: "tenant" },
    halfRanges?.first ?? dateRange
  );
  const secondHalf = useMetricCollection(
    halvesCollection,
    { type: "tenant" },
    halfRanges?.second ?? dateRange
  );

  const resolve: ResolveView = (need) => {
    const location = plan.locate(need);
    if (!location) return undefined;
    if (location.at === "first-half") return firstHalf.byKey.get(need.metric);
    if (location.at === "second-half") return secondHalf.byKey.get(need.metric);
    const byKey =
      location.index === 0
        ? main.byKey
        : extras.get(extraKey(location.index))?.byKey;
    return byKey?.get(need.metric);
  };

  if (
    main.isPending ||
    collectionSetPending(extras) ||
    firstHalf.isPending ||
    secondHalf.isPending
  ) {
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
  const failed = [
    main,
    ...extras.values(),
    firstHalf,
    secondHalf,
  ].filter((result) => result.isError);
  if (failed.length > 0) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          label={`${config.title} — unable to load`}
          onRetry={() => failed.forEach((result) => result.refetch())}
        />
      </div>
    );
  }
  // Observed = any lens view returned an actual reading; "no data yet" and
  // "not collected" must not both render as zeros — or, worse, as a bare
  // title over a page of self-suppressed sections.
  //
  // A view's PRESENCE proves nothing: the period view zero-fills the
  // requested entity with a null value, and a non-dimensioned timeseries is
  // seeded with one all-null series per entity, so a tenant with no rows at
  // all still gets both back (`metric_results/builder.rs`). Only a non-null
  // value inside them is evidence.
  const observed = config.sections
    .flatMap((section) => sectionNeeds(section, bucket))
    .some((need) => {
      const r = resolve(need);
      if (!r) return false;
      const e = tenantData(r);
      return (
        e.value != null ||
        e.series.some((s) => s.points.some((p) => p.value != null)) ||
        e.breakdown.some((v) => v.value != null) ||
        e.histogram.some((h) => h.bins.length > 0)
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
          bucket={bucket}
          resolve={resolve}
          main={main}
        />
      ))}
    </div>
  );
}

const EMPTY_COLLECTION: MetricCollectionConfig = { metrics: [] };

function extraKey(index: number): string {
  return `extra-${index}`;
}

function TenantSection({
  section,
  bucket,
  resolve,
  main,
}: {
  section: TenantSectionSpec;
  bucket: NonNullable<ReturnType<typeof pickTrendBucket>>;
  resolve: ResolveView;
  main: ReturnType<typeof useMetricCollection>;
}) {
  switch (section.kind) {
    case "headline":
      // Period views live in collection 0 by construction (`plan.ts`), the
      // only request with the previous-period twin the deltas need.
      return (
        <TileRow
          metrics={section.metrics}
          data={main.byKey}
          previous={main.previousByKey}
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
            data={main.byKey}
            previous={main.previousByKey}
            minWidth="11rem"
          />
        </section>
      );
    case "trend":
      return <TrendSection section={section} resolve={resolve} bucket={bucket} />;
    case "composition":
      return (
        <CompositionSection section={section} resolve={resolve} bucket={bucket} />
      );
    case "histogram":
      return <HistogramSection section={section} resolve={resolve} />;
    case "stacked-trend":
      return (
        <StackedTrendSection section={section} resolve={resolve} bucket={bucket} />
      );
    case "small-multiples":
      return (
        <SmallMultiplesSection
          section={section}
          resolve={resolve}
          bucket={bucket}
        />
      );
    case "scatter":
      return <ScatterSection section={section} resolve={resolve} bucket={bucket} />;
    case "heatmap-hours":
      return <HeatmapHoursSection section={section} resolve={resolve} />;
    case "hour-columns":
      return <HourColumnsSection section={section} resolve={resolve} />;
    case "slope":
      return <SlopeSection section={section} resolve={resolve} />;
    case "momentum":
      return <MomentumSection section={section} resolve={resolve} />;
    case "marginal-impact":
      return <MarginalImpactSection section={section} resolve={resolve} />;
    case "callout-pair":
      return <CalloutPairSection section={section} resolve={resolve} />;
    case "dumbbell":
      return <DumbbellSection section={section} resolve={resolve} />;
    case "cumulative":
      return (
        <CumulativeSection section={section} resolve={resolve} bucket={bucket} />
      );
    case "decomposition":
      return (
        <DecompositionSection section={section} resolve={resolve} bucket={bucket} />
      );
    case "verdict-table":
      return <VerdictTableSection section={section} resolve={resolve} />;
    default: {
      const _exhaustive: never = section;
      throw new Error(`Unhandled section: ${JSON.stringify(_exhaustive)}`);
    }
  }
}
