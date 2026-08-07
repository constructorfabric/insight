import { useMemo, useState } from "react";
import { MetricName } from "@/components/widgets/metric-help-tooltip";
import { ArrowDownRight, ArrowUpRight } from "lucide-react";
import {
  PolarAngleAxis,
  PolarGrid,
  PolarRadiusAxis,
  Radar,
  RadarChart,
  ResponsiveContainer,
} from "recharts";

import { AttentionList } from "@/components/portal/attention-list";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { orgScopeGate } from "@/components/portal/org-scope-gate";
import { SectionTrend } from "@/components/portal/section-trend";
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
import { usePortalPeriod } from "@/hooks/use-portal-period";
import {
  attentionSummary,
  computeAttentionFlags,
} from "@/lib/insight/attention-flags";
import { GROUPS } from "@/lib/insight/groups";
import {
  availableSlices,
  cohortKey,
  collectRosterAttrs,
  PLANNED_SLICES,
} from "@/lib/insight/slices";
import { MIN_COHORT } from "@/lib/insight/within-team-peer";
import {
  forEntity,
  type MetricCollectionConfig,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import type {
  MetricBucket,
  MetricDirection,
} from "@/api/metric-results-client";
import { normalizePersonId } from "@/lib/metrics/entity";
import { formatMetricValue } from "@/lib/format";
import { mergeEventHistogram } from "@/lib/portal/event-histogram";
import {
  distribution,
  familyObserved,
  fmtCompact,
  groupCoverage,
  medianAcross,
  perCapita,
  representative,
  topDecileShare,
} from "@/lib/portal/metric-stats";
import {
  lensEntry,
  sectionMetricKeys,
  type ConcentrationFraming,
  type LensConfig,
  type SectionSpec,
} from "@/lib/portal/lens-configs";
import { DIRECTIONS } from "@/lib/portal/nav-model";
import { buildTrendData, pickTrendBucket } from "@/lib/portal/trend-data";
import { usePortalNavActions, usePortalSlice } from "@/lib/portal/portal-nav";
import type { TeamMember } from "@/types/insight";
import { useOrgScope } from "@/lib/portal/use-org-scope";
import { useMetricCollection } from "@/queries/metric-results";
import { useMemberGridData } from "@/queries/member-grid";

const EMPTY_COLLECTION: MetricCollectionConfig = { metrics: [] };

/**
 * One renderer for every Directions lens (design §4): sections come from the
 * lens config, values follow the metric grammar (§3), each section
 * self-suppresses on degenerate data (rule 11), the whole tab collapses to an
 * honest "not ingested" state when no metric of the family is observed
 * (rule 6), and no individual is ever named (rule 10).
 */
export function DomainLensView({
  config,
  gridKeys: gridKeysProp,
}: {
  config: LensConfig;
  /**
   * Direction-wide metric key union (see `directionMetricKeys`). When
   * provided, the fetched member grid requests this stable set instead of
   * the lens's own keys, so switching lenses within a direction reuses the
   * same query key instead of minting a new one on every switch (which would
   * otherwise re-trip the full loading gate). The rule-6 observed-gate and
   * every section below still read only the lens's OWN keys
   * (`sectionMetricKeys(config)`) — widening is fetch-only.
   */
  gridKeys?: readonly string[];
}) {
  const { period, dateRange } = usePortalPeriod();

  const orgScope = useOrgScope();
  const { pivot, roster } = orgScope;
  // The roster IS the member list: identity owns who is on the team and
  // every metric for them comes from `/v1/metric-results`. There is no second
  // source to reconcile — the legacy per-member batch this used to call was
  // removed upstream with the rest of the old metric UI.
  const members = useMemo<TeamMember[]>(
    () =>
      (roster ?? []).map((entry) => ({
        person_id: entry.person_id,
        name: entry.display_name,
      })),
    [roster]
  );
  const memberIds = useMemo(
    () => members.map((m) => normalizePersonId(m.person_id)),
    [members]
  );

  // Lens's own keys drive the rule-6 observed-gate and every section below.
  const lensKeys = useMemo(() => sectionMetricKeys(config), [config]);
  // The FETCH collection may widen to the whole direction's key union (see
  // `gridKeys` prop) so switching lenses within a direction never mints a new
  // grid query key — only the request widens, nothing downstream does.
  const fetchKeys = useMemo(
    () => (gridKeysProp ? [...gridKeysProp] : lensKeys),
    [gridKeysProp, lensKeys]
  );
  const gridCollection = useMemo<MetricCollectionConfig>(
    () => ({
      metrics: fetchKeys.map((key) => ({
        key,
        views: [{ view: "period" as const }, { view: "peer" as const }],
      })),
    }),
    [fetchKeys]
  );
  const grid = useMemberGridData(
    gridCollection.metrics.length ? gridCollection : EMPTY_COLLECTION,
    { type: "person", ids: memberIds },
    dateRange,
    period
  );

  // Trend: bucket coarsened to the roster so org scope never trips the row limit.
  const trendKeys = useMemo(
    () =>
      config.sections
        .filter(
          (s): s is Extract<SectionSpec, { kind: "trend" }> =>
            s.kind === "trend"
        )
        .flatMap((s) => s.metrics),
    [config]
  );
  const trendBucket = useMemo(
    () => pickTrendBucket(memberIds.length, trendKeys.length, dateRange),
    [memberIds.length, trendKeys.length, dateRange]
  );
  const trendCollection = useMemo<MetricCollectionConfig>(
    () => ({
      // No bucket fits → no request. Sending one anyway earns a 400 the reader
      // then has to interpret as "the trend is broken" rather than "this window
      // is too wide for this many people".
      metrics: trendBucket
        ? trendKeys.map((key) => ({
            key,
            views: [{ view: "timeseries" as const, bucket: trendBucket }],
          }))
        : [],
    }),
    [trendKeys, trendBucket]
  );
  const trend = useMetricCollection(
    trendCollection.metrics.length && memberIds.length
      ? trendCollection
      : EMPTY_COLLECTION,
    trendCollection.metrics.length && memberIds.length
      ? { type: "person", ids: memberIds }
      : { type: "person", ids: [] },
    dateRange
  );

  // Composition: one breakdown request covering every composition section.
  const compSections = useMemo(
    () =>
      config.sections.filter(
        (s): s is Extract<SectionSpec, { kind: "composition" }> =>
          s.kind === "composition"
      ),
    [config]
  );
  const compCollection = useMemo<MetricCollectionConfig>(
    () => ({
      metrics: compSections.map((s) => ({
        key: s.metric,
        views: [{ view: "breakdown" as const, dimensions: [s.dimension] }],
      })),
    }),
    [compSections]
  );
  const compData = useMetricCollection(
    compSections.length && memberIds.length ? compCollection : EMPTY_COLLECTION,
    compSections.length && memberIds.length
      ? { type: "person", ids: memberIds }
      : { type: "person", ids: [] },
    dateRange
  );

  // Event histograms: per-entity server bins, merged org-side only when edges
  // align across every member (design §7 open question — probed, not assumed).
  const eventSections = useMemo(
    () =>
      config.sections.filter(
        (s): s is Extract<SectionSpec, { kind: "event-histogram" }> =>
          s.kind === "event-histogram"
      ),
    [config]
  );
  const eventCollection = useMemo<MetricCollectionConfig>(
    () => ({
      metrics: eventSections.map((s) => ({
        key: s.metric,
        views: [{ view: "histogram" as const }],
      })),
    }),
    [eventSections]
  );
  const eventData = useMetricCollection(
    eventSections.length && memberIds.length
      ? eventCollection
      : EMPTY_COLLECTION,
    eventSections.length && memberIds.length
      ? { type: "person", ids: memberIds }
      : { type: "person", ids: [] },
    dateRange
  );

  // Slice → by-unit auto-section (rule 7).
  const slice = usePortalSlice();
  const attrByEntity = useMemo(
    () => collectRosterAttrs(pivot, normalizePersonId),
    [pivot]
  );
  const sliceDims = useMemo(
    () => availableSlices(attrByEntity.values()),
    [attrByEntity]
  );
  const sliceLabel = slice
    ? (sliceDims.find((d) => d.key === slice)?.label ?? slice)
    : null;

  const nameByEntity = useMemo(
    () => new Map(members.map((m) => [normalizePersonId(m.person_id), m.name])),
    [members]
  );
  const personIdByEntity = useMemo(
    () =>
      new Map(
        members.map((m) => [normalizePersonId(m.person_id), m.person_id])
      ),
    [members]
  );
  const cohortOf = useMemo(
    () => (id: string) => cohortKey(attrByEntity.get(id), slice),
    [attrByEntity, slice]
  );
  const cohortLabel = slice ? (sliceLabel ?? "cohort").toLowerCase() : "team";

  const gate = orgScopeGate({
    viewerLoading: orgScope.isLoading,
    viewerError: orgScope.isError,
    membersLoading: false,
    membersError: false,
    memberCount: members.length,
    gridPending: grid.isPending,
    gridError: grid.isError,
    emptyLabel:
      "No team in the current scope — a Direction shows a domain across a team; pick a different scope in the topbar.",
    onRetry: () => {
      orgScope.refetch();
      grid.refetch();
    },
  });
  if (gate) return gate;

  // Rule 6: nothing in this family was ever observed → the source isn't wired.
  if (!familyObserved(grid.byKey, lensKeys, memberIds)) {
    return (
      <Pending
        label={
          config.notIngested ??
          `${config.title} — source isn't ingested for this org yet.`
        }
      />
    );
  }

  return (
    <div className="flex flex-col gap-6 p-4 md:p-6">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">{config.title}</h1>
        <p className="text-sm text-muted-foreground">
          {orgScope.count} members · {config.tagline ?? "trend & balance"}
        </p>
      </div>

      {config.sections.map((s, i) => (
        <Section
          key={`${s.kind}-${i}`}
          spec={s}
          grid={grid}
          trend={trend}
          trendBucket={trendBucket}
          compData={compData.byKey}
          compIsError={compData.isError}
          compRefetch={compData.refetch}
          eventByKey={eventData.byKey}
          eventIsError={eventData.isError}
          memberIds={memberIds}
          cohortOf={cohortOf}
          cohortLabel={cohortLabel}
          nameByEntity={nameByEntity}
          personIdByEntity={personIdByEntity}
        />
      ))}

      {slice ? (
        <ByUnitSection
          config={config}
          grid={grid.byKey}
          memberIds={memberIds}
          keyOf={(id) => attrByEntity.get(id)?.[slice]?.value ?? null}
          sliceKey={slice}
          sliceLabel={sliceLabel ?? slice}
        />
      ) : null}
    </div>
  );
}

/* ── Section dispatch ────────────────────────────────────────────────── */

interface GridData {
  byKey: Map<string, NormalizedMetricResult>;
  previousByKey: Map<string, NormalizedMetricResult>;
}
interface TrendData {
  byKey: Map<string, NormalizedMetricResult>;
  isPending: boolean;
  isError: boolean;
  refetch: () => void;
}

function Section({
  spec,
  grid,
  trend,
  trendBucket,
  compData,
  compIsError,
  compRefetch,
  eventByKey,
  eventIsError,
  memberIds,
  cohortOf,
  cohortLabel,
  nameByEntity,
  personIdByEntity,
}: {
  spec: SectionSpec;
  grid: GridData;
  trend: TrendData;
  trendBucket: MetricBucket | null;
  compData: Map<string, NormalizedMetricResult>;
  compIsError: boolean;
  compRefetch: () => void;
  eventByKey: Map<string, NormalizedMetricResult>;
  eventIsError: boolean;
  memberIds: readonly string[];
  nameByEntity: Map<string, string>;
  personIdByEntity: Map<string, string>;
  cohortOf: (id: string) => string | null;
  cohortLabel: string;
}) {
  switch (spec.kind) {
    case "headline":
      return (
        <HeadlineSection
          metrics={spec.metrics}
          grid={grid}
          memberIds={memberIds}
        />
      );
    case "stat-tiles":
      return (
        <StatTilesSection
          title={spec.title}
          metrics={spec.metrics}
          grid={grid}
          memberIds={memberIds}
        />
      );
    case "trend":
      return trendBucket ? (
        <TrendSection
          metrics={spec.metrics}
          grid={grid}
          trend={trend}
          bucket={trendBucket}
          memberIds={memberIds}
        />
      ) : (
        // Say which of the two dials to turn — a bare "no data" would read as
        // an ingestion gap rather than a request nobody can answer.
        <Pending label="Trend needs a narrower period or a smaller scope — this many people over this window exceeds one request." />
      );
    case "distribution":
      return (
        <DistributionSection spec={spec} grid={grid} memberIds={memberIds} />
      );
    case "concentration":
      return (
        <ConcentrationSection spec={spec} grid={grid} memberIds={memberIds} />
      );
    case "composition":
      return (
        <CompositionSection
          spec={spec}
          compData={compData}
          compIsError={compIsError}
          compRefetch={compRefetch}
          grid={grid}
          memberIds={memberIds}
        />
      );
    case "participation":
      return (
        <ParticipationSection
          spec={spec}
          grid={grid}
          trend={trend}
          memberIds={memberIds}
        />
      );
    case "event-histogram":
      return (
        <EventHistogramSection
          spec={spec}
          grid={grid}
          eventByKey={eventByKey}
          eventIsError={eventIsError}
          memberIds={memberIds}
        />
      );
    case "attention":
      return (
        <AttentionSection
          spec={spec}
          grid={grid}
          memberIds={memberIds}
          cohortOf={cohortOf}
          cohortLabel={cohortLabel}
          nameByEntity={nameByEntity}
          personIdByEntity={personIdByEntity}
        />
      );
    case "direction-cards":
      return (
        <DirectionCardsSection
          variant={spec.variant}
          grid={grid}
          memberIds={memberIds}
        />
      );
    case "coverage-radar":
      return <CoverageRadarSection grid={grid} memberIds={memberIds} />;
  }
}

/* ── participation (rule 8 variant — "N of M are active") ────────────── */

function ParticipationSection({
  spec,
  grid,
  trend,
  memberIds,
}: {
  spec: Extract<SectionSpec, { kind: "participation" }>;
  grid: GridData;
  trend: TrendData;
  memberIds: readonly string[];
}) {
  const isActive = (byKey: Map<string, NormalizedMetricResult>, id: string) =>
    spec.metrics.some((key) => {
      const r = byKey.get(key);
      return r != null && (forEntity(r, id).value ?? 0) > 0;
    });

  // Active people per trend bucket (client count over the fetched timeseries).
  // Memoised: the scan is metrics × roster × buckets, and at org scope every
  // parent re-render — a slice or period change — repeated all of it.
  const data = useMemo(() => {
    const byDate = new Map<string, Set<string>>();
    for (const key of spec.metrics) {
      const r = trend.byKey.get(key);
      if (!r) continue;
      for (const id of memberIds) {
        for (const s of forEntity(r, id).series) {
          for (const p of s.points) {
            if ((p.value ?? 0) > 0) {
              let ids = byDate.get(p.bucket_start);
              if (!ids) byDate.set(p.bucket_start, (ids = new Set()));
              ids.add(id);
            }
          }
        }
      }
    }
    return [...byDate.entries()]
      .map(([date, ids]) => ({ date, active: ids.size }))
      .sort((a, b) => a.date.localeCompare(b.date));
  }, [spec.metrics, trend.byKey, memberIds]);

  const active = memberIds.filter((id) => isActive(grid.byKey, id)).length;
  const prevActive = memberIds.filter((id) =>
    isActive(grid.previousByKey, id)
  ).length;
  // After the hook: an early return above it would make the hook conditional.
  if (memberIds.length === 0) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {spec.title}
      </p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(14rem,1fr))] gap-3">
        <Card>
          <CardContent className="p-4">
            <div className="flex items-center justify-between gap-2">
              <div className="text-xs font-medium text-muted-foreground">
                {spec.noun}
              </div>
              <Delta
                now={active}
                prev={prevActive || null}
                direction="higher_is_better"
              />
            </div>
            <div className="mt-1 text-2xl font-semibold tabular-nums">
              {active} of {memberIds.length}
            </div>
            <div className="text-xs text-muted-foreground">
              {Math.round((active / memberIds.length) * 100)}% of the team this
              period
            </div>
          </CardContent>
        </Card>
      </div>
      {data.length > 1 ? (
        <SectionTrend
          title="Active people over time"
          series={[{ key: "active", label: spec.noun, type: "line" }]}
          data={data}
          isPending={trend.isPending}
        />
      ) : null}
    </section>
  );
}

/* ── headline (rules 1-2) ────────────────────────────────────────────── */

function HeadlineSection({
  metrics,
  grid,
  memberIds,
}: {
  metrics: readonly string[];
  grid: GridData;
  memberIds: readonly string[];
}) {
  const cards = metrics
    .map((key) => {
      const r = grid.byKey.get(key);
      if (!r) return null;
      const now = representative(r, memberIds);
      if (now == null) return null;
      const prev = representative(grid.previousByKey.get(key), memberIds);
      const isSum = r.computation === "sum";
      return { key, r, now, prev, isSum };
    })
    .filter((x): x is NonNullable<typeof x> => x != null);
  if (!cards.length) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        Per person · vs previous period
      </p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(12rem,1fr))] gap-3">
        {cards.map((c) => (
          <Card key={c.key}>
            <CardContent className="p-4">
              <div className="flex items-center justify-between gap-2">
                <MetricName
                  metric={c.r}
                  text={c.r.short_label ?? c.r.label}
                  className="text-xs font-medium text-muted-foreground"
                />
                <Delta now={c.now} prev={c.prev} direction={c.r.direction} />
              </div>
              <div className="mt-1 text-2xl font-semibold tabular-nums">
                {formatMetricValue(
                  c.isSum ? perCapita(c.r, memberIds) : c.now,
                  c.r.format,
                  c.r.unit
                )}
              </div>
              <div className="text-xs text-muted-foreground">
                {c.isSum
                  ? `per active person · ${formatMetricValue(c.now, c.r.format, c.r.unit)} team total`
                  : "median / person"}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>
    </section>
  );
}

/* ── stat-tiles (rule 2, with deltas) ────────────────────────────────── */

function StatTilesSection({
  title,
  metrics,
  grid,
  memberIds,
}: {
  title: string;
  metrics: readonly string[];
  grid: GridData;
  memberIds: readonly string[];
}) {
  const tiles = metrics
    .map((key) => {
      const r = grid.byKey.get(key);
      if (!r) return null;
      const median = medianAcross(r, memberIds);
      if (median == null) return null;
      const prev = medianAcross(grid.previousByKey.get(key), memberIds);
      return { key, r, median, prev };
    })
    .filter((x): x is NonNullable<typeof x> => x != null);
  if (!tiles.length) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {title}
      </p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(11rem,1fr))] gap-3">
        {tiles.map((t) => (
          <Card key={t.key}>
            <CardContent className="p-4">
              <div className="flex items-center justify-between gap-2">
                <MetricName
                  metric={t.r}
                  text={t.r.short_label ?? t.r.label}
                  className="text-xs font-medium text-muted-foreground"
                />
                <Delta now={t.median} prev={t.prev} direction={t.r.direction} />
              </div>
              <div className="mt-1 text-2xl font-semibold tabular-nums">
                {formatMetricValue(t.median, t.r.format, t.r.unit)}
              </div>
              <div className="text-xs text-muted-foreground">
                median / person
              </div>
            </CardContent>
          </Card>
        ))}
      </div>
    </section>
  );
}

/* ── trend (rule 8) ──────────────────────────────────────────────────── */

function TrendSection({
  metrics,
  grid,
  trend,
  bucket,
  memberIds,
}: {
  metrics: readonly string[];
  grid: GridData;
  trend: TrendData;
  bucket: MetricBucket;
  memberIds: readonly string[];
}) {
  const series = metrics
    .map((key) => {
      const r = grid.byKey.get(key);
      if (!r || r.computation !== "sum") return null;
      return { key, label: r.short_label ?? r.label };
    })
    .filter((x): x is NonNullable<typeof x> => x != null)
    .map((s, i) => ({
      key: s.key,
      label: s.label,
      type: "line" as const,
      yAxisId: (i === 0 ? "left" : "right") as "left" | "right",
    }));
  const data = buildTrendData(
    series.map((s) => s.key),
    trend.byKey,
    memberIds
  );

  if (series.length === 0) return null;
  if (trend.isError)
    return (
      <SectionTrend
        title="Activity over time"
        series={series}
        data={[]}
        isError
        onRetry={trend.refetch}
      />
    );
  if (data.length < 2) return null;
  return (
    <SectionTrend
      title="Activity over time"
      description={`Team totals · per ${bucket}`}
      series={series}
      data={data}
      rightAxis={series.some((s) => s.yAxisId === "right")}
      isPending={trend.isPending}
    />
  );
}

/* ── distribution (rules 3, 11) ──────────────────────────────────────── */

const DIST_CONFIG: ChartConfig = { count: { label: "People" } };

function DistributionSection({
  spec,
  grid,
  memberIds,
}: {
  spec: Extract<SectionSpec, { kind: "distribution" }>;
  grid: GridData;
  memberIds: readonly string[];
}) {
  const r = grid.byKey.get(spec.metric);
  const values = r
    ? memberIds
        .map((id) => forEntity(r, id).value)
        .filter((v): v is number => v != null && Number.isFinite(v) && v >= 0)
    : [];
  const fmt =
    r?.format === "percent"
      ? (n: number) => formatMetricValue(n, "percent", null)
      : fmtCompact;
  const rows = distribution(values, fmt);
  if (!rows.length) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {spec.title} · {values.length} people
      </p>
      <Card>
        <CardContent className="p-4">
          <p className="mb-3 text-xs text-muted-foreground">{spec.caption}</p>
          <ChartContainer config={DIST_CONFIG} className="h-56 w-full">
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
                    labelFormatter={(_, p) =>
                      (p?.[0]?.payload as { range?: string } | undefined)
                        ?.range ?? ""
                    }
                  />
                }
              />
              <ChartBar
                dataKey="count"
                name="People"
                radius={[2, 2, 0, 0]}
                fill="var(--chart-1)"
              />
            </BarChart>
          </ChartContainer>
          <p className="mt-1 text-center text-[10px] text-muted-foreground">
            {spec.unitLabel}
          </p>
        </CardContent>
      </Card>
    </section>
  );
}

/* ── event-histogram (design §7 — bin-aligned merge, honest fallback) ── */

/**
 * Org-wide event histogram assembled from per-entity server bins. This is an
 * enhancement probe, not a guaranteed capability: org-wide bin alignment
 * across entities is unknown until observed (design §7 open question). When
 * `mergeEventHistogram` can't align edges (or errors on the underlying
 * fetch), this section renders nothing — the Flow stat-tiles above still
 * carry the tab, and a missing chart here reads as "not available", never as
 * a fabricated or wrong distribution.
 */
function EventHistogramSection({
  spec,
  grid,
  eventByKey,
  eventIsError,
  memberIds,
}: {
  spec: Extract<SectionSpec, { kind: "event-histogram" }>;
  grid: GridData;
  eventByKey: Map<string, NormalizedMetricResult>;
  eventIsError: boolean;
  memberIds: readonly string[];
}) {
  if (eventIsError) return null;
  const r = grid.byKey.get(spec.metric);
  const bins = mergeEventHistogram(eventByKey.get(spec.metric), memberIds);
  if (!bins) return null;

  const fmt =
    r?.format === "percent"
      ? (n: number) => formatMetricValue(n, "percent", null)
      : fmtCompact;
  const rows = bins.map((bin) => ({
    label: fmt(bin.lo),
    range: `${fmt(bin.lo)}–${fmt(bin.hi)}`,
    count: bin.count,
  }));
  const total = bins.reduce((sum, bin) => sum + bin.count, 0);

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {spec.title} · {total} events
      </p>
      <Card>
        <CardContent className="p-4">
          <p className="mb-3 text-xs text-muted-foreground">
            Distribution of events (every PR), not people.
          </p>
          <ChartContainer config={DIST_CONFIG} className="h-56 w-full">
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
                    labelFormatter={(_, p) =>
                      (p?.[0]?.payload as { range?: string } | undefined)
                        ?.range ?? ""
                    }
                  />
                }
              />
              <ChartBar
                dataKey="count"
                name="Events"
                radius={[2, 2, 0, 0]}
                fill="var(--chart-1)"
              />
            </BarChart>
          </ChartContainer>
          <p className="mt-1 text-center text-[10px] text-muted-foreground">
            {r?.short_label ?? r?.label ?? spec.metric}
            {r?.unit ? ` (${r.unit})` : ""}
          </p>
        </CardContent>
      </Card>
    </section>
  );
}

/* ── concentration (rules 4, 10 — aggregate only, framed per domain) ─── */

const FRAMING_COPY: Record<
  ConcentrationFraming,
  { heading: string; note: string }
> = {
  "bus-factor": {
    heading: "Bus factor · top 10% of contributors",
    note: "high concentration = continuity risk",
  },
  "load-balance": {
    heading: "Load concentration · top 10% of contributors",
    note: "even share ≈ 10%",
  },
};

function ConcentrationSection({
  spec,
  grid,
  memberIds,
}: {
  spec: Extract<SectionSpec, { kind: "concentration" }>;
  grid: GridData;
  memberIds: readonly string[];
}) {
  const cards = spec.metrics
    .map((key) => {
      const r = grid.byKey.get(key);
      if (!r) return null;
      const vals = memberIds
        .map((id) => forEntity(r, id).value)
        .filter((v): v is number => v != null && Number.isFinite(v) && v > 0);
      const share = topDecileShare(vals);
      if (share == null) return null;
      return {
        key,
        label: r.short_label ?? r.label,
        share,
        contributors: vals.length,
      };
    })
    .filter((x): x is NonNullable<typeof x> => x != null);
  if (!cards.length) return null;
  const copy = FRAMING_COPY[spec.framing];

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {copy.heading}
      </p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(14rem,1fr))] gap-3">
        {cards.map((c) => (
          <Card key={c.key}>
            <CardContent className="p-4">
              <div className="text-xs font-medium text-muted-foreground">
                {c.label}
              </div>
              <div className="mt-1 text-2xl font-semibold tabular-nums">
                {Math.round(c.share * 100)}%
              </div>
              <div className="text-xs text-muted-foreground">
                carried by the busiest{" "}
                {Math.max(1, Math.ceil(c.contributors * 0.1))} of{" "}
                {c.contributors} · {copy.note}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>
    </section>
  );
}

/* ── composition (rule 5) ────────────────────────────────────────────── */

function CompositionSection({
  spec,
  compData,
  compIsError,
  compRefetch,
  grid,
  memberIds,
}: {
  spec: Extract<SectionSpec, { kind: "composition" }>;
  compData: Map<string, NormalizedMetricResult>;
  compIsError: boolean;
  compRefetch: () => void;
  grid: GridData;
  memberIds: readonly string[];
}) {
  if (compIsError) {
    return (
      <section className="flex flex-col gap-3">
        <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
          {spec.title}
        </p>
        <ComingSoon
          variant="card"
          state="error"
          label={`${spec.title} — unable to load`}
          onRetry={compRefetch}
        />
      </section>
    );
  }

  const r = grid.byKey.get(spec.metric);
  const bd = compData.get(spec.metric);
  const bucket = new Map<string, number>();
  if (bd) {
    for (const id of memberIds) {
      for (const row of forEntity(bd, id).breakdown) {
        const val = row.dimensions.find((d) => d.key === spec.dimension)?.value;
        if (!val || row.value == null || row.value <= 0) continue;
        bucket.set(val, (bucket.get(val) ?? 0) + row.value);
      }
    }
  }
  const rows = toBarRows(bucket);
  // A single 100%-share bar is an empty shell (rule 11), same as ByUnitSection.
  if (rows.length < 2) return null;

  return (
    <BarList
      title={spec.title}
      rows={rows}
      format={r?.format ?? "integer"}
      unit={r?.unit ?? null}
    />
  );
}

/* ── attention (Overview design O3: the ONLY section that names people —
      actionable pointers into Person, not a leaderboard) ───────────────── */

function AttentionSection({
  spec,
  grid,
  memberIds,
  cohortOf,
  cohortLabel,
  nameByEntity,
  personIdByEntity,
}: {
  spec: Extract<SectionSpec, { kind: "attention" }>;
  grid: GridData;
  memberIds: readonly string[];
  cohortOf: (id: string) => string | null;
  cohortLabel: string;
  nameByEntity: Map<string, string>;
  personIdByEntity: Map<string, string>;
}) {
  const flags = useMemo(
    () =>
      computeAttentionFlags({
        headlineKeys: spec.metrics,
        byKey: grid.byKey,
        previousByKey: grid.previousByKey,
        memberIds,
        cohortOf,
        nameOf: (id) => nameByEntity.get(id) ?? id,
        personIdOf: (id) => personIdByEntity.get(id) ?? id,
        cohortLabel,
      }),
    [
      spec.metrics,
      grid.byKey,
      grid.previousByKey,
      memberIds,
      cohortOf,
      cohortLabel,
      nameByEntity,
      personIdByEntity,
    ]
  );
  if (!memberIds.length) return null;
  const flaggedPeople = new Set(flags.map((f) => f.personId)).size;
  return (
    <AttentionList
      flags={flags}
      summary={attentionSummary(flags, flaggedPeople, memberIds.length)}
      peopleLabel={
        flags.length
          ? `${flaggedPeople} of ${memberIds.length} people`
          : undefined
      }
      max={spec.max}
    />
  );
}

/* ── direction-cards (Overview design O4: cards derive from DIRECTION_LENSES,
      click routes into the Directions zone) ─────────────────────────────── */

function DirectionCardsSection({
  variant,
  grid,
  memberIds,
}: {
  variant: "compact" | "full";
  grid: GridData;
  memberIds: readonly string[];
}) {
  const { setDir, setLens, setZone } = usePortalNavActions();
  const cards = DIRECTIONS.map((d) => {
    const entry = lensEntry(d.id, "Overview");
    if (!entry || "comingSoon" in entry) return null;
    const headline = entry.sections.find(
      (s): s is Extract<SectionSpec, { kind: "headline" }> =>
        s.kind === "headline"
    );
    if (!headline) return null;
    const keys =
      variant === "compact" ? headline.metrics.slice(0, 2) : headline.metrics;
    const observed = familyObserved(
      grid.byKey,
      sectionMetricKeys(entry),
      memberIds
    );
    return { id: d.id, name: d.name, keys, observed };
  }).filter((c): c is NonNullable<typeof c> => c != null);
  if (!cards.length) return null;

  const go = (dir: string) => {
    setDir(dir);
    setLens("Overview");
    setZone("directions");
  };

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        By direction
      </p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(16rem,1fr))] gap-3">
        {cards.map((c) => (
          <button
            key={c.id}
            type="button"
            onClick={() => go(c.id)}
            className="rounded-xl border bg-card text-left transition-colors hover:bg-accent"
          >
            <div className="flex flex-col gap-2 p-4">
              <div className="text-sm font-semibold">{c.name}</div>
              {c.observed ? (
                c.keys.map((key) => {
                  const r = grid.byKey.get(key);
                  if (!r) return null;
                  const now = representative(r, memberIds);
                  if (now == null) return null;
                  const prev = representative(
                    grid.previousByKey.get(key),
                    memberIds
                  );
                  const isSum = r.computation === "sum";
                  return (
                    <div
                      key={key}
                      className="flex items-center justify-between gap-2 text-sm"
                    >
                      <MetricName
                        metric={r}
                        text={r.short_label ?? r.label}
                        className="truncate text-muted-foreground"
                      />
                      <span className="flex items-center gap-2">
                        <span className="font-medium tabular-nums">
                          {formatMetricValue(
                            isSum ? perCapita(r, memberIds) : now,
                            r.format,
                            r.unit
                          )}
                        </span>
                        <Delta now={now} prev={prev} direction={r.direction} />
                      </span>
                    </div>
                  );
                })
              ) : (
                <span className="text-xs text-muted-foreground">
                  source isn&apos;t ingested for this org yet
                </span>
              )}
            </div>
          </button>
        ))}
      </div>
    </section>
  );
}

/* ── coverage-radar (Overview design O5: coverage = share of members with ≥1
      OBSERVED metric per group — entityObserved, never zero-filled sums) ── */

function CoverageRadarSection({
  grid,
  memberIds,
}: {
  grid: GridData;
  memberIds: readonly string[];
}) {
  if (memberIds.length < MIN_COHORT) return null;
  const data = GROUPS.map((g) => ({
    domain: g.title,
    coverage: Math.round(
      (groupCoverage(grid.byKey, g.card.preview, memberIds) ?? 0) * 100
    ),
  }));
  if (data.every((d) => d.coverage === 0)) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        Health radar
      </p>
      <Card>
        <CardContent className="p-4">
          <div className="h-72 w-full">
            <ResponsiveContainer width="100%" height="100%">
              <RadarChart data={data} outerRadius="70%">
                <PolarGrid />
                <PolarAngleAxis dataKey="domain" tick={{ fontSize: 12 }} />
                {/* Percent scale is absolute: full edge = 100% coverage, not
                    the period's max — and the explicit axis gives recharts a
                    radius scale (without one the polygon collapses to center). */}
                <PolarRadiusAxis
                  domain={[0, 100]}
                  tick={false}
                  axisLine={false}
                />
                <Radar
                  dataKey="coverage"
                  stroke="var(--primary)"
                  fill="var(--primary)"
                  fillOpacity={0.25}
                  isAnimationActive={false}
                />
              </RadarChart>
            </ResponsiveContainer>
          </div>
          <p className="text-xs text-muted-foreground">
            Coverage — % of people in scope with any observed activity in each
            domain this period.
          </p>
        </CardContent>
      </Card>
    </section>
  );
}

/* ── by-unit auto-section (rule 7) ───────────────────────────────────── */

const NO_COMPARABLE_UNITS_NOTE =
  "No comparable units for this lens at this slice (needs a summable headline metric and ≥2 units of ≥4 people).";

function ByUnitSection({
  config,
  grid,
  memberIds,
  keyOf,
  sliceKey,
  sliceLabel,
}: {
  config: LensConfig;
  grid: Map<string, NormalizedMetricResult>;
  memberIds: readonly string[];
  keyOf: (id: string) => string | null;
  sliceKey: string;
  sliceLabel: string;
}) {
  // A declared-but-unfed dimension (e.g. functional team) can never produce
  // by-unit data — say so plainly rather than fall through to the generic
  // "no comparable units" note, which would wrongly suggest a data quirk.
  const planned = PLANNED_SLICES.find((d) => d.key === sliceKey);
  if (planned) {
    return (
      <SliceNote text={`The ${planned.label} dimension isn't ingested yet.`} />
    );
  }

  // Compare units on the lens's first headline counter, per active person.
  const headline = config.sections.find(
    (s): s is Extract<SectionSpec, { kind: "headline" }> =>
      s.kind === "headline"
  );
  // No headline → by-unit was never promised for this lens; stay silent.
  if (!headline) return null;
  const key = headline.metrics.find((k) => grid.get(k)?.computation === "sum");
  const r = key ? grid.get(key) : undefined;
  if (!r) return <SliceNote text={NO_COMPARABLE_UNITS_NOTE} />;

  const byUnit = new Map<string, string[]>();
  for (const id of memberIds) {
    const unit = keyOf(id);
    if (!unit) continue;
    (byUnit.get(unit) ?? byUnit.set(unit, []).get(unit)!).push(id);
  }
  const bucket = new Map<string, number>();
  for (const [unit, ids] of byUnit) {
    if (ids.length < MIN_COHORT) continue; // small-cohort suppression
    const v = perCapita(r, ids);
    if (v > 0) bucket.set(`${unit} · ${ids.length}`, v);
  }
  const rows = toBarRows(bucket);
  if (rows.length < 2) return <SliceNote text={NO_COMPARABLE_UNITS_NOTE} />;

  return (
    <BarList
      title={`${r.short_label ?? r.label} per active person · by ${sliceLabel}`}
      rows={rows}
      format={r.format}
      unit={r.unit}
      showShare={false}
    />
  );
}

/* ── shared bits ─────────────────────────────────────────────────────── */

interface BarRow {
  label: string;
  value: number;
  pct: number;
}

function toBarRows(bucket: Map<string, number>): BarRow[] {
  const total = [...bucket.values()].reduce((a, b) => a + b, 0) || 1;
  return [...bucket.entries()]
    .map(([label, value]) => ({
      label,
      value,
      pct: Math.round((value / total) * 100),
    }))
    .sort((a, b) => b.value - a.value);
}

/** Rows shown before the reader opts into the full list. */
const BAR_LIST_COLLAPSED = 12;

function BarList({
  title,
  rows,
  format,
  unit,
  showShare = true,
}: {
  title: string;
  rows: BarRow[];
  format: NormalizedMetricResult["format"];
  unit: string | null;
  /** False for per-capita values, where a share-of-total percent would mislead. */
  showShare?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const visible = expanded ? rows : rows.slice(0, BAR_LIST_COLLAPSED);
  const max = rows[0]?.value || 1;
  // Collapsed view is a sample, not the full picture — the title says so,
  // and the "+N more" button below hands over the rest on demand.
  const displayTitle =
    !expanded && rows.length > BAR_LIST_COLLAPSED
      ? `${title} · top ${BAR_LIST_COLLAPSED}`
      : title;
  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {displayTitle}
      </p>
      <Card>
        <CardContent className="flex flex-col gap-2 p-4">
          {visible.map((row) => (
            <div key={row.label} className="flex items-center gap-3">
              <div className="w-44 shrink-0 truncate text-sm">{row.label}</div>
              <div className="relative h-6 flex-1 overflow-hidden rounded bg-muted">
                <div
                  className="h-full rounded bg-primary/25"
                  style={{ width: `${Math.round((row.value / max) * 100)}%` }}
                />
                <span className="absolute inset-y-0 left-2 flex items-center text-xs font-medium tabular-nums">
                  {formatMetricValue(row.value, format, unit)}
                  {showShare ? ` · ${row.pct}%` : ""}
                </span>
              </div>
            </div>
          ))}
          {rows.length > BAR_LIST_COLLAPSED ? (
            <button
              type="button"
              onClick={() => setExpanded((v) => !v)}
              className="self-start pt-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            >
              {expanded
                ? "Show less"
                : `+${rows.length - BAR_LIST_COLLAPSED} more`}
            </button>
          ) : null}
        </CardContent>
      </Card>
    </section>
  );
}

function SliceNote({ text }: { text: string }) {
  return (
    <p className="rounded-md border border-dashed bg-muted/30 p-3 text-xs text-muted-foreground">
      {text}
    </p>
  );
}

function Delta({
  now,
  prev,
  direction,
}: {
  now: number;
  prev: number | null;
  direction: MetricDirection;
}) {
  if (prev == null || prev === 0) return null;
  const diff = now - prev;
  if (Math.abs(diff) / Math.abs(prev) < 0.01) {
    return (
      <span className="text-xs text-muted-foreground tabular-nums">±0%</span>
    );
  }
  const up = diff > 0;
  const good =
    direction === "neutral"
      ? null
      : direction === "higher_is_better"
        ? up
        : !up;
  const color =
    good == null
      ? "text-muted-foreground"
      : good
        ? "text-success"
        : "text-destructive";
  const Icon = up ? ArrowUpRight : ArrowDownRight;
  const pct = Math.round((diff / Math.abs(prev)) * 100);
  return (
    <span
      className={`flex items-center gap-0.5 text-xs font-medium tabular-nums ${color}`}
    >
      <Icon className="h-3.5 w-3.5" />
      {up ? "+" : ""}
      {pct}%
    </span>
  );
}

export function Pending({ label }: { label: string }) {
  return (
    <div className="mx-auto w-full max-w-md p-8">
      <ComingSoon variant="card" state="empty" label={label} />
    </div>
  );
}
