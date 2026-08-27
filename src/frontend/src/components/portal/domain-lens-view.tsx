import { ExplainWithAi } from "@/components/widgets/dashboard/explain-with-ai";
import { trendSnapshot } from "@/lib/insight/explain-snapshot";
import { Link } from "@tanstack/react-router";
import { Fragment, useMemo, useState, type CSSProperties } from "react";
import { MetricName } from "@/components/widgets/metric-help-tooltip";
import { ArrowDownRight, ArrowUpRight } from "lucide-react";
import { AttentionList } from "@/components/portal/attention-list";
import { personDisplayName } from "@/lib/identities/person-display";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { orgScopeGate } from "@/components/portal/org-scope-gate";
import {
  SectionTrend,
  type SectionTrendPoint,
  type SectionTrendSeries,
} from "@/components/portal/section-trend";
import { Card, CardContent } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
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
import { GROUPS, visibleGroups } from "@/lib/insight/groups";
import type { PersonCoverage } from "@/lib/insight/coverage";
import { useScopeCoverage } from "@/lib/portal/use-scope-coverage";
import { useVisibilityPolicy } from "@/queries/identity-me";
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
import {
  personsEvidenceSelection,
  type MetricEvidenceSelection,
} from "@/api/metric-drilldown-client";
import {
  useMetricEvidenceOptional,
  type EvidencePeopleView,
} from "@/components/metric-evidence-context";
import { peopleEvidenceView } from "@/lib/portal/evidence-people";
import { formatMetricValue } from "@/lib/format";
import { seriesColors } from "@/lib/series-colors";
import {
  labelForDimensionValue,
  toBarRows,
  UNSPLIT_SEGMENT,
  type BarEntry,
  type BarRow,
  type BarSegment,
} from "@/lib/portal/bar-rows";
import { narrowedEvidenceSelection } from "@/lib/metrics/evidence-targets";
import { evidenceMetricFor } from "@/lib/metrics/evidence-via";
import {
  dayHourMatrix,
  HOUR_BLOCKS,
  WEEKDAY_LABELS,
} from "@/lib/portal/day-hour-matrix";
import { mergeEventHistogram } from "@/lib/portal/event-histogram";
import { peerPopulationLabel } from "@/lib/portal/use-cohort-label";
import {
  bandAtClick,
  distribution,
  entityValues,
  familyObserved,
  fmtCompact,
  medianAcross,
  perCapita,
  representative,
  topDecile,
} from "@/lib/portal/metric-stats";
import {
  lensEntry,
  overviewCardDirections,
  sectionMetricKeys,
  visibleSections,
  type ConcentrationFraming,
  type LensConfig,
  type SectionSpec,
} from "@/lib/portal/lens-configs";
import {
  buildActiveContributorData,
  buildMedianTrendData,
  buildTrendData,
  pickTrendBucket,
} from "@/lib/portal/trend-data";
import { bucketBreakdown, bucketRange } from "@/lib/portal/trend-drilldown";
import {
  TrendDrilldownDialog,
  type TrendDrilldownState,
} from "@/components/portal/trend-drilldown-dialog";
import { usePortalNavActions, usePortalSlice } from "@/lib/portal/portal-nav";
import { usePortalSearch } from "@/lib/portal/portal-search";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import type { TeamMember } from "@/types/insight";
import { useOrgScope } from "@/lib/portal/use-org-scope";
import { useMetricCollection } from "@/queries/metric-results";
import { useMemberGridData } from "@/queries/member-grid";
import { TEXT_FIGURE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const EMPTY_COLLECTION: MetricCollectionConfig = { metrics: [] };

/**
 * One renderer for every Directions lens (design §4): sections come from the
 * lens config, values follow the metric grammar (§3), each section
 * self-suppresses on degenerate data (rule 11), the whole tab collapses to an
 * honest "not ingested" state when no metric of the family is observed
 * (rule 6), and no individual is ever named (rule 10).
 */
export function DomainLensView({
  config: declared,
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

  const [drilldown, setDrilldown] = useState<TrendDrilldownState | null>(null);
  // What this install shows, not what the registry declares: a gated metric
  // takes its tile with it, and a section left with none of its own is gone
  // rather than drawn empty (`visibleSections`).
  const showPlanned = usePortalShowPlanned();
  // One repository under inspection turns the lens into that repository's
  // screen: the drilldown's own sections, every request filtered to the value,
  // and a breadcrumb back. A `repo` that outlives its lens is ignored rather
  // than rendering a screen about a value this lens does not group by.
  const { repo } = usePortalSearch();
  const { openRepository } = usePortalNavActions();
  const evidence = useMetricEvidenceOptional();
  const scoped = repo && declared.drilldown ? repo : null;
  const scopeDimension = scoped ? (declared.drilldown?.dimension ?? null) : null;
  const config = useMemo(
    () =>
      visibleSections(
        scoped && declared.drilldown
          ? {
              ...declared,
              tagline: declared.drilldown.tagline ?? declared.tagline,
              sections: declared.drilldown.sections,
            }
          : declared,
        showPlanned
      ),
    [declared, showPlanned, scoped]
  );
  // Every request the screen makes carries this, so no section can forget it
  // and quietly answer about the whole tenant instead.
  const scopeFilters = useMemo(
    () =>
      scopeDimension && scoped
        ? [{ dimension: scopeDimension, values: [scoped] }]
        : undefined,
    [scopeDimension, scoped]
  );


  const orgScope = useOrgScope();
  const { isFlat } = useVisibilityPolicy();
  const { pivot, roster } = orgScope;
  // The roster IS the member list: identity owns who is on the team and
  // every metric for them comes from `/v1/metric-results`. There is no second
  // source to reconcile — the legacy per-member batch this used to call was
  // removed upstream with the rest of the old metric UI.
  const members = useMemo<TeamMember[]>(
    () =>
      (roster ?? []).map((entry) => ({
        person_id: entry.person_id,
        name: personDisplayName(entry),
      })),
    [roster]
  );
  const memberIds = useMemo(
    () => members.map((m) => normalizePersonId(m.person_id)),
    [members]
  );
  // Metric rows are keyed by the normalized id; the display name rides along
  // so a drilldown row can say whose work it was.
  const scopedMembers = useMemo(
    () =>
      members.map((m) => ({
        person_id: normalizePersonId(m.person_id),
        name: m.name,
      })),
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
        ...(scopeFilters ? { filters: scopeFilters } : {}),
        views: [{ view: "period" as const }, { view: "peer" as const }],
      })),
    }),
    [fetchKeys, scopeFilters]
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
    () => pickTrendBucket(memberIds.length, dateRange),
    [memberIds.length, dateRange]
  );
  const trendCollection = useMemo<MetricCollectionConfig>(
    () => ({
      // No bucket fits → no request. Sending one anyway earns a 400 the reader
      // then has to interpret as "the trend is broken" rather than "this window
      // is too wide for this many people".
      metrics: trendBucket
        ? trendKeys.map((key) => ({
            key,
            ...(scopeFilters ? { filters: scopeFilters } : {}),
            views: [{ view: "timeseries" as const, bucket: trendBucket }],
          }))
        : [],
    }),
    [trendKeys, trendBucket, scopeFilters]
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

  // Composition + ownership: one breakdown request covering every section
  // that reads per-person dimension rows. Sections sharing a metric merge
  // their dimension lists into one entry — the request rejects duplicate keys.
  const breakdownSections = useMemo(
    () =>
      config.sections.filter(
        (
          s
        ): s is Extract<SectionSpec, { kind: "composition" | "ownership" }> =>
          s.kind === "composition" || s.kind === "ownership"
      ),
    [config]
  );
  const compCollection = useMemo<MetricCollectionConfig>(() => {
    const dims = new Map<string, Set<string>>();
    for (const s of breakdownSections) {
      const set = dims.get(s.metric) ?? new Set<string>();
      set.add(s.dimension);
      if (s.kind === "composition" && s.splitBy) set.add(s.splitBy);
      dims.set(s.metric, set);
    }
    return {
      metrics: [...dims].map(([key, set]) => ({
        key,
        ...(scopeFilters ? { filters: scopeFilters } : {}),
        views: [{ view: "breakdown" as const, dimensions: [...set] }],
      })),
    };
  }, [breakdownSections, scopeFilters]);
  const compData = useMetricCollection(
    breakdownSections.length && memberIds.length
      ? compCollection
      : EMPTY_COLLECTION,
    breakdownSections.length && memberIds.length
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
        ...(scopeFilters ? { filters: scopeFilters } : {}),
        views: [{ view: "histogram" as const }],
      })),
    }),
    [eventSections, scopeFilters]
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

  // Dimension tables: one rollup request covering every table section. The
  // rollup view collapses the person grain server-side, so a metric appears
  // once with the first table's dimension.
  const tableSections = useMemo(
    () =>
      config.sections.filter(
        (s): s is Extract<SectionSpec, { kind: "dimension-table" }> =>
          s.kind === "dimension-table"
      ),
    [config]
  );
  const tableCollection = useMemo<MetricCollectionConfig>(() => {
    const dimensionByMetric = new Map<string, string>();
    for (const s of tableSections) {
      for (const key of s.metrics) {
        if (!dimensionByMetric.has(key)) dimensionByMetric.set(key, s.dimension);
      }
    }
    return {
      metrics: [...dimensionByMetric].map(([key, dimension]) => ({
        key,
        ...(scopeFilters ? { filters: scopeFilters } : {}),
        views: [{ view: "rollup" as const, dimensions: [dimension] }],
      })),
    };
  }, [tableSections, scopeFilters]);
  const tableData = useMetricCollection(
    tableSections.length && memberIds.length
      ? tableCollection
      : EMPTY_COLLECTION,
    tableSections.length && memberIds.length
      ? { type: "person", ids: memberIds }
      : { type: "person", ids: [] },
    dateRange
  );

  // Heatmap: a timeseries of its own, because the same metric cannot carry two
  // timeseries views in one request — and the trend already claims that view
  // for the metrics it charts. Bucketed by DAY whatever the trend chose: the
  // matrix reads the weekday off each bucket, so a week-sized bucket would
  // collapse five weekdays into one cell.
  const hourSections = useMemo(
    () =>
      config.sections.filter(
        (s): s is Extract<SectionSpec, { kind: "heatmap-hours" }> =>
          s.kind === "heatmap-hours"
      ),
    [config]
  );
  const hourCollection = useMemo<MetricCollectionConfig>(
    () => ({
      metrics: hourSections.map((s) => ({
        key: s.metric,
        ...(scopeFilters ? { filters: scopeFilters } : {}),
        views: [
          {
            view: "timeseries" as const,
            bucket: "day" as const,
            dimensions: [HOUR_BLOCK_DIMENSION],
          },
        ],
      })),
    }),
    [hourSections, scopeFilters]
  );
  const hourData = useMetricCollection(
    hourSections.length && memberIds.length ? hourCollection : EMPTY_COLLECTION,
    hourSections.length && memberIds.length
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
  const cohortLabel = peerPopulationLabel(
    slice ? (sliceLabel ?? "cohort") : null,
    isFlat
  );

  const gate = orgScopeGate({
    viewerLoading: orgScope.isLoading,
    viewerError: orgScope.isError,
    membersLoading: false,
    membersError: false,
    memberCount: members.length,
    gridPending: grid.isPending,
    gridError: grid.isError,
    emptyLabel:
      "No people in the current scope. Pick a different scope at the top of the page.",
    onRetry: () => {
      orgScope.refetch();
      grid.refetch();
    },
  });
  if (gate) return gate;

  // Rule 6: nothing in this family was ever observed → the source isn't wired.
  //
  // Exempt: a lens that reads no metric of its own cannot be judged by whether
  // its metrics were observed, and one whose SUBJECT is coverage must survive
  // exactly the case this gate fires on. Telling a reader "no source is
  // ingested" on the screen built to tell them which sources are not ingested
  // would withhold the answer at the moment it is worth most.
  const readsGrid = config.sections.some((s) => s.kind !== "coverage-levels");
  if (readsGrid && !familyObserved(grid.byKey, lensKeys, memberIds)) {
    return (
      <Pending
        label={
          config.notIngested ??
          `${config.title} — this data source is not connected yet.`
        }
      />
    );
  }

  // The value's own label, found wherever the responses named it. The URL
  // carries the id, which is `<source>:<owner>/<repo>` and not what a reader
  // recognises; the id stands in until a row arrives that knows better.
  const scopedLabel = scoped
    ? (labelForDimensionValue(
        scopeDimension,
        scoped,
        compData.byKey,
        tableData.byKey
      ) ?? null)
    : null;

  // The affordance sits with the page title, not on the chart: it explains the
  // view, and a chart that grows a second card would carry two of them.
  const trendSpec = config.sections.find(
    (section): section is Extract<SectionSpec, { kind: "trend" }> =>
      section.kind === "trend"
  );
  const charts =
    trendSpec && trendBucket ? trendCharts(trendSpec, grid, trend, memberIds) : [];
  const drawable = charts.filter((c) => c.data.length > 1);
  // The roster the chart sums IS the drilldown's scope, so the two can never
  // disagree: under a flat policy that roster is the whole organisation, and
  // under an org chart it is the viewer's own subtree — the permission
  // boundary identity already enforces.
  // One hour block over the whole period — the narrowest slice of a heatmap the
  // evidence request can express. A single cell is that block on ONE weekday,
  // and there is no weekday predicate to ask for.
  const openHourBlock = (block: string) => {
    const heat = config.sections.find(
      (section): section is Extract<SectionSpec, { kind: "heatmap-hours" }> =>
        section.kind === "heatmap-hours"
    );
    const carrier = heat && grid.byKey.get(evidenceMetricFor(heat.metric));
    if (!evidence || !carrier?.drilldown) return;
    const selection = narrowedEvidenceSelection(carrier, memberIds, {
      filters: [
        ...(scopeDimension && scoped
          ? [{ dimension: scopeDimension, value: scoped }]
          : []),
        { dimension: HOUR_BLOCK_DIMENSION, value: block },
      ],
    });
    if (selection) {
      evidence.openEvidenceTargets([{ selection, label: carrier.label }], {
        activeMetricKey: selection.metric_key,
      });
    }
  };

  const openTrendDrilldown = (chart: TrendChart, bucketStart?: string) => {
    // A clicked point asks about its own bucket; the card asks about the
    // period. The label says which, so the dialog never looks like the other.
    const range = bucketStart
      ? bucketRange(bucketStart, trendBucket ?? "day")
      : { from: dateRange.from, to: dateRange.to };
    setDrilldown({
      metricKey: chart.derived ? null : chart.drilldownKey,
      label: bucketStart ? `${chart.title} · ${bucketStart}` : chart.title,
      bucketLabel: trendBucket ?? "period",
      period: range,
      members: scopedMembers,
      breakdown: bucketBreakdown(chart.drilldownKey, trend.byKey, scopedMembers),
    });
  };

  return (
    <div className="flex flex-col gap-6 p-4 md:p-6">
      {scoped ? (
        <nav aria-label="Breadcrumb" className="flex items-center gap-2 text-sm">
          <button
            type="button"
            onClick={() => openRepository("")}
            className="cursor-pointer text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
          >
            {declared.title}
          </button>
          <span aria-hidden className="text-muted-foreground">
            ›
          </span>
          <span className="font-medium">{scopedLabel ?? scoped}</span>
        </nav>
      ) : null}
      <div className="relative flex items-start justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold tracking-tight">
            {scoped ? (scopedLabel ?? scoped) : config.title}
          </h1>
          <p className="text-sm text-muted-foreground">
            {orgScope.count} {orgScope.count === 1 ? "person" : "people"} ·{" "}
            {config.tagline ?? "trend & balance"}
          </p>
        </div>
        {drawable.length > 0 && trendBucket ? (
          <ExplainWithAi
            className="static"
            snapshot={trendSnapshot(drawable, {
              title: config.title,
              bucket: trendBucket,
              since: dateRange.from,
              until: dateRange.to,
              people: memberIds.length,
            })}
          />
        ) : null}
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
          hourByKey={hourData.byKey}
          hourIsError={hourData.isError}
          tableByKey={tableData.byKey}
          tableIsError={tableData.isError}
          tableRefetch={tableData.refetch}
          memberIds={memberIds}
          cohortOf={cohortOf}
          cohortLabel={cohortLabel}
          nameByEntity={nameByEntity}
          personIdByEntity={personIdByEntity}
          onOpenChart={openTrendDrilldown}
          onOpenValue={
            // Only from the listing screen: a table INSIDE the drilldown is
            // already about one value, so descending again means nothing.
            !scoped && declared.drilldown
              ? (value) => openRepository(value)
              : undefined
          }
          onOpenBlock={openHourBlock}
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

      <TrendDrilldownDialog
        state={drilldown}
        onClose={() => setDrilldown(null)}
      />
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
  hourByKey,
  hourIsError,
  tableByKey,
  tableIsError,
  tableRefetch,
  memberIds,
  cohortOf,
  cohortLabel,
  nameByEntity,
  personIdByEntity,
  onOpenChart,
  onOpenValue,
  onOpenBlock,
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
  hourByKey: Map<string, NormalizedMetricResult>;
  hourIsError: boolean;
  tableByKey: Map<string, NormalizedMetricResult>;
  tableIsError: boolean;
  tableRefetch: () => void;
  memberIds: readonly string[];
  nameByEntity: Map<string, string>;
  personIdByEntity: Map<string, string>;
  cohortOf: (id: string) => string | null;
  cohortLabel: string;
  onOpenChart: (chart: TrendChart, bucketStart?: string) => void;
  /** Descend into one dimension value; absent when the lens has no screen for one. */
  onOpenValue?: (value: string) => void;
  /** The records of one hour block of a heatmap. */
  onOpenBlock?: (block: string) => void;
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
          spec={spec}
          grid={grid}
          trend={trend}
          bucket={trendBucket}
          memberIds={memberIds}
          onOpenChart={onOpenChart}
        />
      ) : (
        // Say which of the two dials to turn — a bare "no data" would read as
        // an ingestion gap rather than a request nobody can answer.
        <Pending label="Too many people over too long a period to chart at once. Pick a shorter period or a smaller scope." />
      );
    case "distribution":
      return (
        <DistributionSection
          spec={spec}
          grid={grid}
          memberIds={memberIds}
          nameByEntity={nameByEntity}
          personIdByEntity={personIdByEntity}
        />
      );
    case "concentration":
      return (
        <ConcentrationSection
          spec={spec}
          grid={grid}
          memberIds={memberIds}
          nameByEntity={nameByEntity}
          personIdByEntity={personIdByEntity}
        />
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
    case "contributors":
      return (
        <ContributorsSection
          spec={spec}
          grid={grid}
          memberIds={memberIds}
          nameByEntity={nameByEntity}
        />
      );
    case "heatmap-hours":
      return (
        <HeatmapHoursSection
          spec={spec}
          hourByKey={hourByKey}
          hourIsError={hourIsError}
          memberIds={memberIds}
          onOpenBlock={onOpenBlock}
        />
      );
    case "dimension-table":
      return (
        <DimensionTableSection
          spec={spec}
          tableByKey={tableByKey}
          tableIsError={tableIsError}
          tableRefetch={tableRefetch}
          onOpenValue={onOpenValue}
        />
      );
    case "ownership":
      return (
        <OwnershipSection
          spec={spec}
          compData={compData}
          compIsError={compIsError}
          compRefetch={compRefetch}
          memberIds={memberIds}
          nameByEntity={nameByEntity}
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
    case "coverage-levels":
      return (
        <CoverageLevelsSection
          memberIds={memberIds}
          nameByEntity={nameByEntity}
          personIdByEntity={personIdByEntity}
        />
      );
  }
}

/* ── coverage (#2408): three cuts of one model, read top to bottom — the
      verdict, then which parts are missing, then who is thinly seen. Little
      prose on purpose: a screen that needs a paragraph to explain itself has
      already failed the reader who only glanced at it. ──────────────────── */

function CoverageBar({
  filled,
  total,
  warn,
}: {
  filled: number;
  total: number;
  warn?: boolean;
}) {
  return (
    <div className="h-2.5 min-w-px flex-1 overflow-hidden rounded-full bg-muted">
      <div
        className={`h-full rounded-full ${warn ? "bg-warning/80" : "bg-primary/60"}`}
        style={{ width: `${total > 0 ? (filled / total) * 100 : 0}%` }}
      />
    </div>
  );
}

function CoverageLevelsSection({
  memberIds,
  nameByEntity,
  personIdByEntity,
}: {
  memberIds: readonly string[];
  nameByEntity: Map<string, string>;
  personIdByEntity: Map<string, string>;
}) {
  const [openLevel, setOpenLevel] = useState<number | null>(null);
  const showPlanned = usePortalShowPlanned();
  const { distribution, parts, people, thin, isPending, isError } =
    useScopeCoverage(memberIds);
  if (isPending) return <Pending label="Reading coverage…" />;
  // Before anything else. With a request failed nothing is known to reach the
  // tenant, so every part would read "no data" and every person
  // would sit at zero — a fault in our infrastructure printed as a verdict
  // about named people. Saying we could not check is the only honest output.
  if (isError) {
    return (
      <Pending label="Could not read coverage. The check did not finish, so nothing here is claimed about anyone." />
    );
  }
  const counted = distribution.counted;
  if (counted === 0) return null;

  // The same sections the coverage hook counted, or the denominator would name
  // sections this install does not show.
  const partCount = visibleGroups(showPlanned).length;
  const levels = [...distribution.byLevel.entries()].sort(
    (a, b) => b[0] - a[0]
  );
  const missing = parts.filter((p) => p.unreachable);

  return (
    <section className="flex flex-col gap-6">
      {/* 1 — the verdict, as a number rather than a sentence: it is meant to
          be seen, not parsed. */}
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <span className={TEXT_FIGURE}>
          {/* Same amber as the rows it is the sum of. The link between the
              number and the block of bars is the one thing a reader has to
              make unaided, and colour makes it without a caption. */}
          <span className="text-warning">{thin}</span>
          <span className="text-muted-foreground">/{counted}</span>
        </span>
        <p className="max-w-md text-sm text-muted-foreground">
          people have data for fewer than half the sections. Everything shown
          about them comes from those sections only.
        </p>
      </div>

      {/* 2 — where it is missing. A part nothing reaches is NOT drawn as a bar
          at zero, because that reads as people who did nothing, which is the
          one thing it does not mean. */}
      <div className="flex flex-col gap-2">
        <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
          By section
        </p>
        {parts.map((part) => (
          <div key={part.id} className="flex items-center gap-3 text-sm">
            <span className="w-36 shrink-0 truncate">{part.title}</span>
            {part.unreachable ? (
              <span className="flex-1 text-xs text-warning">
                {/* No cause named. Absent observations, a disabled metric and
                    a broken schema all land here, and only the first is a
                    missing connector — sending someone to plumb a live one is
                    the wrong direction to be wrong in. */}
                no data
              </span>
            ) : (
              <CoverageBar filled={part.seen} total={counted} />
            )}
            <span className="w-14 shrink-0 text-right text-muted-foreground tabular-nums">
              {part.unreachable ? "—" : part.seen}
            </span>
          </div>
        ))}
      </div>

      {/* 3 — who. Colour carries the finding, so the shape reads without the
          labels: the amber block IS the number at the top. */}
      <div className="flex flex-col gap-2">
        <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
          By person · sections with data
        </p>
        {levels.map(([level, n], i) => {
          const thinHere = level < partCount / 2;
          // The rule sits where it applies. Without it the reader has to work
          // out which rows the headline counted, and having to work it out is
          // the same as not knowing it.
          const boundary =
            thinHere && !(levels[i - 1] && levels[i - 1]![0] < partCount / 2);
          return (
            <div key={level} className="flex flex-col gap-2">
              {boundary && (
                <div className="mt-1 flex items-center gap-3">
                  <span className="w-36 shrink-0 text-xs text-warning">
                    fewer than half
                  </span>
                  <span className="h-px flex-1 bg-warning/40" />
                  <span className="w-14 shrink-0 text-right text-xs font-medium text-warning tabular-nums dark:text-warning">
                    {thin}
                  </span>
                </div>
              )}
              <button
                type="button"
                disabled={n === 0}
                onClick={() => setOpenLevel(openLevel === level ? null : level)}
                aria-expanded={openLevel === level}
                aria-controls={`coverage-level-${level}`}
                className="-mx-2 flex items-center gap-3 rounded-sm px-2 py-0.5 text-left text-sm enabled:hover:bg-muted/60 disabled:cursor-default"
              >
                <span className="w-36 shrink-0 text-muted-foreground tabular-nums">
                  {level} of {partCount}
                </span>
                <CoverageBar filled={n} total={counted} warn={thinHere} />
                <span className="w-14 shrink-0 text-right tabular-nums">
                  {n}
                </span>
              </button>
              {openLevel === level && (
                <CoverageLevelPeople
                  id={`coverage-level-${level}`}
                  people={people.filter((p) => p.level === level)}
                  nameByEntity={nameByEntity}
                  personIdByEntity={personIdByEntity}
                />
              )}
            </div>
          );
        })}
      </div>

      <p className="text-xs text-muted-foreground">
        A section counts when at least one of its metrics has a value for that
        person in this period. This shows where data exists, not how well anyone
        worked.
        {missing.length > 0 && (
          <>
            {" "}
            No one can reach {partCount} of {partCount} here:{" "}
            {missing.map((m) => m.title).join(", ")}{" "}
            {missing.length === 1 ? "has" : "have"} no data for anyone.
          </>
        )}{" "}
        Counted over the {counted} {counted === 1 ? "person" : "people"} in this
        scope.
      </p>
    </section>
  );
}

/**
 * The people at one coverage level, and what is missing for each.
 *
 * The missing parts are the point, not the names. A level says how much we
 * cannot see; this says which systems to go and look at — and separates the
 * two kinds of absence, because they lead different places. "No connector"
 * is somebody's job to fix. "Nothing recorded" is a person who does that work
 * elsewhere, or does not do it, and no amount of plumbing changes it.
 */
function CoverageLevelPeople({
  id,
  people,
  nameByEntity,
  personIdByEntity,
}: {
  id: string;
  people: readonly PersonCoverage[];
  nameByEntity: Map<string, string>;
  personIdByEntity: Map<string, string>;
}) {
  // Every title, not just the visible ones: this only resolves ids that are
  // already in a person's coverage states, which the hook gated on its way in.
  const titleById = new Map(GROUPS.map((g) => [g.id, g.title]));
  const rows = [...people].sort((a, b) =>
    (nameByEntity.get(a.entityId) ?? a.entityId).localeCompare(
      nameByEntity.get(b.entityId) ?? b.entityId
    )
  );

  return (
    <ul id={id} className="mb-2 ml-36 flex flex-col gap-1 border-s ps-3">
      {rows.map((p) => {
        const unconnected: string[] = [];
        const idle: string[] = [];
        for (const [id, state] of p.states) {
          const title = titleById.get(id) ?? id;
          if (state === "no_data_reaches_us") unconnected.push(title);
          else if (state === "nothing_recorded") idle.push(title);
        }
        const personId = personIdByEntity.get(p.entityId);
        const name = nameByEntity.get(p.entityId) ?? p.entityId;
        return (
          <li
            key={p.entityId}
            className="flex flex-wrap items-baseline gap-x-2 text-xs"
          >
            {personId ? (
              <Link
                to="/ic/$person/personal"
                params={{ person: personId }}
                className="font-medium hover:underline"
              >
                {name}
              </Link>
            ) : (
              <span className="font-medium">{name}</span>
            )}
            {unconnected.length > 0 && (
              <span className="text-warning">
                not measured for anyone: {unconnected.join(", ")}
              </span>
            )}
            {idle.length > 0 && (
              <span className="text-muted-foreground">
                nothing recorded for this person: {idle.join(", ")}
              </span>
            )}
          </li>
        );
      })}
    </ul>
  );
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
            <div className={cn("mt-1", TEXT_FIGURE)}>
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
      // The records that carry the figure: its own, unless the family says
      // another metric holds them (a line count is carried by commits).
      const carrier = grid.byKey.get(evidenceMetricFor(key));
      return { key, r, now, prev, isSum, carrier };
    })
    .filter((x): x is NonNullable<typeof x> => x != null);
  if (!cards.length) return null;

  // Every drillable card of the row, so the dialog a reader opens on one of
  // them lists its neighbours — the same set the members grid offers from a
  // row of cells. Deduplicated by metric: two tiles carried by one metric
  // would otherwise name the same key twice, which the request refuses.
  const seen = new Set<string>();
  const targets = cards.flatMap((c) => {
    if (!c.carrier?.drilldown || seen.has(c.carrier.metric_key)) return [];
    const selection = personsEvidenceSelection(c.carrier.selection, memberIds);
    if (!selection) return [];
    seen.add(c.carrier.metric_key);
    return [{ selection, label: c.carrier.label }];
  });

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        Per person · vs previous period
      </p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(12rem,1fr))] gap-3">
        {cards.map((c) => (
          <HeadlineCard
            key={c.key}
            card={c}
            targets={targets}
            memberIds={memberIds}
          />
        ))}
      </div>
    </section>
  );
}

/**
 * One headline figure, and the records it was taken over.
 *
 * The card opens the evidence dialog for the WHOLE roster rather than for a
 * person: the number on it is the roster's, and a reader asking what it is
 * made of is asking about all of them. Nothing here names an individual —
 * the dialog lists records, and who did what stays inside it.
 *
 * A metric whose evidence cannot be read renders as a plain card: an
 * affordance that opens an empty dialog is worse than none.
 */
function HeadlineCard({
  card,
  targets,
  memberIds,
}: {
  card: {
    r: NormalizedMetricResult;
    now: number;
    prev: number | null;
    isSum: boolean;
    carrier: NormalizedMetricResult | undefined;
  };
  targets: readonly { selection: MetricEvidenceSelection; label: string }[];
  memberIds: readonly string[];
}) {
  const evidence = useMetricEvidenceOptional();
  const c = card;
  const selection = c.carrier?.drilldown
    ? personsEvidenceSelection(c.carrier.selection, memberIds)
    : null;
  const body = (
    <CardContent className="p-4">
      <div className="flex items-center justify-between gap-2">
        {/* The full label, not the short one: `short_label` exists for a grid
            column head or a heatmap axis, and it drops the very word that says
            what was counted — "Issues" for issues CLOSED. A card twelve rem
            wide has room to say it. */}
        <MetricName
          metric={c.r}
          className="text-xs font-medium text-muted-foreground"
        />
        <Delta now={c.now} prev={c.prev} direction={c.r.direction} />
      </div>
      <div className={cn("mt-1", TEXT_FIGURE)}>
        {formatMetricValue(c.now, c.r.format, c.r.unit)}
      </div>
      {/* The team total leads, because the dialog this card opens lists the
        roster's records — a per-person figure on the face and a roster-sized
        table behind it read as a contradiction. The per-person figure keeps
        its place underneath, where the comparison it serves still works. */}
      <div className="text-xs text-muted-foreground">
        {c.isSum
          ? `team total · ${formatMetricValue(perCapita(c.r, memberIds), c.r.format, c.r.unit)} per active person`
          : "median / person"}
      </div>
    </CardContent>
  );

  if (!selection || !evidence) return <Card>{body}</Card>;

  return (
    <Card className="transition-colors hover:bg-muted/40 focus-within:ring-2 focus-within:ring-ring">
      <button
        type="button"
        aria-haspopup="dialog"
        className="w-full cursor-pointer text-left focus-visible:outline-none"
        onClick={() =>
          evidence.openEvidenceTargets(targets, {
            activeMetricKey: selection.metric_key,
          })
        }
      >
        {body}
      </button>
    </Card>
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
              <div className={cn("mt-1", TEXT_FIGURE)}>
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

/**
 * The lines the trend chart draws and their points. Shared with the zone
 * header, whose explain affordance describes the very same chart.
 */
export interface TrendChart {
  id: string;
  title: string;
  description: string;
  /** The catalog metric this chart drills into. */
  drilldownKey: string;
  /** Counted from another metric's rows, so it has no records of its own. */
  derived?: boolean;
  series: SectionTrendSeries[];
  data: SectionTrendPoint[];
}

/**
 * One chart per measure, plus the derived contributor count when the spec
 * asks for it. Shared with the zone header, whose explain affordance describes
 * every chart on the page.
 */
function trendCharts(
  spec: Extract<SectionSpec, { kind: "trend" }>,
  grid: GridData,
  trend: TrendData,
  memberIds: readonly string[]
): TrendChart[] {
  const charts = spec.metrics
    .map((key): TrendChart | null => {
      const r = grid.byKey.get(key);
      if (!r) return null;
      const label = r.short_label ?? r.label;
      if (r.computation !== "sum") {
        // Non-additive metrics chart the roster median per bucket — summing
        // per-person ratios or medians would fabricate a number. `derived`
        // because the plotted value is computed here, not a server figure a
        // record list can back.
        return {
          id: key,
          title: label,
          description: "Median per person",
          drilldownKey: key,
          derived: true,
          series: [{ key, label, type: "line" as const }],
          data: buildMedianTrendData(key, trend.byKey, memberIds),
        };
      }
      return {
        id: key,
        title: label,
        description: "Team total",
        drilldownKey: key,
        series: [{ key, label, type: "line" as const }],
        data: buildTrendData([key], trend.byKey, memberIds),
      };
    })
    .filter((c): c is TrendChart => c != null);

  const activeKey = spec.activeContributorsFor;
  if (activeKey && grid.byKey.has(activeKey)) {
    const of = grid.byKey.get(activeKey);
    charts.push({
      id: `${activeKey}:active`,
      title: `Active contributors · ${of?.short_label ?? of?.label ?? activeKey}`,
      description: "People with at least one",
      drilldownKey: activeKey,
      derived: true,
      series: [{ key: "active", label: "People", type: "line" as const }],
      data: buildActiveContributorData(activeKey, trend.byKey, memberIds),
    });
  }

  return charts;
}

function TrendSection({
  spec,
  grid,
  trend,
  bucket,
  memberIds,
  onOpenChart,
}: {
  spec: Extract<SectionSpec, { kind: "trend" }>;
  grid: GridData;
  trend: TrendData;
  bucket: MetricBucket;
  memberIds: readonly string[];
  onOpenChart: (chart: TrendChart, bucketStart?: string) => void;
}) {
  const charts = trendCharts(spec, grid, trend, memberIds);

  if (charts.length === 0) return null;
  if (trend.isError)
    return (
      <SectionTrend
        title="Activity over time"
        series={charts[0]?.series ?? []}
        data={[]}
        isError
        onRetry={trend.refetch}
      />
    );

  const drawable = charts.filter((c) => c.data.length > 1);
  if (drawable.length === 0) return null;

  // One chart per measure rather than three lines over two axes: a shared
  // axis makes a count of people and a count of lines look comparable when
  // they are not, and there is nowhere to click from a legend.
  return (
    <div className="grid grid-cols-[repeat(auto-fit,minmax(min(34rem,100%),1fr))] gap-4">
      {drawable.map((chart) => (
        <button
          key={chart.id}
          type="button"
          className="rounded-xl text-left transition-opacity hover:opacity-90 focus-visible:ring-ring focus-visible:ring-2 focus-visible:outline-none"
          onClick={() => onOpenChart(chart)}
          aria-label={`Open ${chart.title} details`}
        >
          <SectionTrend
            title={chart.title}
            description={`${chart.description} · per ${bucket}`}
            series={chart.series}
            data={chart.data}
            isPending={trend.isPending}
            onBucketClick={(bucketStart) => onOpenChart(chart, bucketStart)}
          />
        </button>
      ))}
    </div>
  );
}

/* ── distribution (rules 3, 11) ──────────────────────────────────────── */

const DIST_CONFIG: ChartConfig = { count: { label: "People" } };

function DistributionSection({
  spec,
  grid,
  memberIds,
  nameByEntity,
  personIdByEntity,
}: {
  spec: Extract<SectionSpec, { kind: "distribution" }>;
  grid: GridData;
  memberIds: readonly string[];
  nameByEntity: Map<string, string>;
  personIdByEntity: Map<string, string>;
}) {
  const evidence = useMetricEvidenceOptional();
  const r = grid.byKey.get(spec.metric);
  const entries = entityValues(r, memberIds).filter((e) => e.value >= 0);
  const fmt =
    r?.format === "percent"
      ? (n: number) => formatMetricValue(n, "percent", null)
      : fmtCompact;
  const rows = distribution(entries, fmt);
  if (!rows.length || !r) return null;

  // Per band, because a band IS a set of people: the reader pointing at a bar
  // is asking who those people are, and the values are already here — the
  // records behind any one of them are a step further in, inside the dialog.
  const bandsByRange = new Map(
    evidence
      ? rows.flatMap((row) =>
          row.count
            ? ([
                [
                  row.range,
                  peopleEvidenceView(
                    r,
                    row.ids,
                    `${r.label} · ${row.range} ${spec.unitLabel}`,
                    { nameByEntity, personIdByEntity }
                  ),
                ],
              ] as const)
            : []
        )
      : []
  );
  const openBand = (range: string) => {
    const view = bandsByRange.get(range);
    if (view) evidence?.openEvidencePeople(view);
  };

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {spec.title} · {entries.length} people
      </p>
      <Card>
        <CardContent className="p-4">
          <p className="mb-3 text-xs text-muted-foreground">{spec.caption}</p>
          <ChartContainer
            config={DIST_CONFIG}
            className={cn("h-56 w-full", bandsByRange.size && "cursor-pointer")}
          >
            <BarChart
              data={rows}
              margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
              // The whole column, not the drawn bar: a band holding two people
              // is a rectangle a pixel tall, and those are the bands a reader
              // most wants to open.
              onClick={(next) => {
                const row = bandAtClick(rows, next);
                if (row) openBand(row.range);
              }}
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
          <p className="mt-1 text-center text-xs text-muted-foreground">
            {spec.unitLabel}
          </p>
          {/* The bands again, as buttons. Not decoration: the tail bars of a
              skewed distribution are a pixel tall, and those are the ones a
              reader wants to open — and a bar is not reachable by keyboard. */}
          {bandsByRange.size > 0 && (
            <div className="mt-3 flex flex-col gap-1.5">
              <p className="text-xs text-muted-foreground">
                See who is in a band
              </p>
              <div className="flex flex-wrap gap-1.5">
                {rows
                  .filter((row) => bandsByRange.has(row.range))
                  .map((row) => (
                    <button
                      key={row.range}
                      type="button"
                      aria-haspopup="dialog"
                      aria-label={`${row.range} ${spec.unitLabel} · ${row.count} ${row.count === 1 ? "person" : "people"}`}
                      onClick={() => openBand(row.range)}
                      className="cursor-pointer rounded-sm border px-2 py-0.5 text-xs hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    >
                      <span className="tabular-nums">{row.range}</span>
                      <span className="ml-1.5 text-muted-foreground tabular-nums">
                        {row.count}
                      </span>
                    </button>
                  ))}
              </div>
            </div>
          )}
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
          <p className="mt-1 text-center text-xs text-muted-foreground">
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
    heading: "How much of the work sits with the busiest tenth",
    note: "high concentration = continuity risk",
  },
  "load-balance": {
    heading: "How much of the load sits with the busiest tenth",
    note: "even share ≈ 10%",
  },
};

function ConcentrationSection({
  spec,
  grid,
  memberIds,
  nameByEntity,
  personIdByEntity,
}: {
  spec: Extract<SectionSpec, { kind: "concentration" }>;
  grid: GridData;
  memberIds: readonly string[];
  nameByEntity: Map<string, string>;
  personIdByEntity: Map<string, string>;
}) {
  const cards = spec.metrics
    .map((key) => {
      const r = grid.byKey.get(key);
      if (!r) return null;
      const contributors = entityValues(r, memberIds).filter(
        (e) => e.value > 0
      );
      const top = topDecile(contributors);
      if (!top) return null;
      const subset = `busiest ${top.ids.length} of ${contributors.length}`;
      return {
        key,
        label: r.short_label ?? r.label,
        share: top.share,
        subset,
        // The busiest tenth, not the roster: this card's figure is a statement
        // about those people only.
        people: peopleEvidenceView(r, top.ids, `${r.label} · ${subset}`, {
          nameByEntity,
          personIdByEntity,
        }),
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
          <ConcentrationCard key={c.key} card={c} note={copy.note} />
        ))}
      </div>
    </section>
  );
}

/**
 * One concentration figure, and who the busiest tenth actually is.
 *
 * The card opens that subset rather than the roster: a list of everyone would
 * answer a different question from the one the card raises. Names stay inside
 * the dialog — the card itself still says nothing about any individual.
 */
function ConcentrationCard({
  card,
  note,
}: {
  card: {
    label: string;
    share: number;
    subset: string;
    people: EvidencePeopleView;
  };
  note: string;
}) {
  const evidence = useMetricEvidenceOptional();
  const c = card;
  const body = (
    <CardContent className="p-4">
      <div className="text-xs font-medium text-muted-foreground">{c.label}</div>
      <div className={cn("mt-1", TEXT_FIGURE)}>
        {Math.round(c.share * 100)}%
      </div>
      <div className="text-xs text-muted-foreground">
        carried by the {c.subset} · {note}
      </div>
    </CardContent>
  );

  if (!evidence || !c.people.rows.length) return <Card>{body}</Card>;

  return (
    <Card className="transition-colors hover:bg-muted/40 focus-within:ring-2 focus-within:ring-ring">
      <button
        type="button"
        aria-haspopup="dialog"
        className="w-full cursor-pointer text-left focus-visible:outline-none"
        onClick={() => evidence.openEvidencePeople(c.people)}
      >
        {body}
      </button>
    </Card>
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
  const evidence = useMetricEvidenceOptional();
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
  // Summed per dimension VALUE and shown under its LABEL: the value is the id
  // the response groups by (a source id joined to `owner/repo`), which is
  // neither what a reader recognises nor distinguishable once a list of them is
  // truncated. Two things that happen to share a label stay two bars, because
  // the sum is keyed by the id.
  const bucket = new Map<string, BarEntry>();
  if (bd) {
    for (const id of memberIds) {
      for (const row of forEntity(bd, id).breakdown) {
        const dim = row.dimensions.find((d) => d.key === spec.dimension);
        if (!dim?.value || row.value == null || row.value <= 0) continue;
        const running = bucket.get(dim.value);
        const split = running?.split ?? (spec.splitBy ? new Map() : undefined);
        if (split && spec.splitBy) {
          const by = row.dimensions.find((d) => d.key === spec.splitBy);
          // A row the response did not split still belongs to the total, so it
          // lands in a segment of its own rather than being dropped.
          const seed = by?.value || UNSPLIT_SEGMENT;
          const seen = split.get(seed);
          split.set(seed, {
            seed,
            label: by?.label?.trim() || seen?.label || seed,
            value: (seen?.value ?? 0) + row.value,
          });
        }
        const label = dim.label?.trim() || running?.label || dim.value;
        const href = running?.href ?? dim.href;
        bucket.set(dim.value, {
          label,
          value: (running?.value ?? 0) + row.value,
          href,
          split,
        });
      }
    }
  }
  const rows = toBarRows(bucket, spec.splitBy);
  // A single 100%-share bar is an empty shell (rule 11), same as ByUnitSection.
  if (rows.length < 2) return null;

  // The records behind a bar are the ones that carry the figure, which for a
  // line count is its commits rather than its own per-day summary.
  const carrier = grid.byKey.get(evidenceMetricFor(spec.metric));
  const openBar =
    evidence && carrier?.drilldown
      ? (row: BarRow, segment?: BarSegment) => {
          const selection = narrowedEvidenceSelection(carrier, memberIds, {
            filters: [
              { dimension: spec.dimension, value: row.key },
              ...(segment && spec.splitBy
                ? [{ dimension: spec.splitBy, value: segment.seed }]
                : []),
            ],
          });
          if (selection) {
            evidence.openEvidenceTargets(
              [{ selection, label: carrier.label }],
              { activeMetricKey: selection.metric_key }
            );
          }
        }
      : undefined;

  return (
    <BarList
      title={spec.title}
      rows={rows}
      format={r?.format ?? "integer"}
      unit={r?.unit ?? null}
      notes={spec.notes}
      onOpen={openBar}
      canOpen={(_row, segment) => segment?.seed !== UNSPLIT_SEGMENT}
    />
  );
}

/* ── contributors + heatmap-hours (inside a drilldown) ───────────────── */

const CONTRIBUTOR_ROWS = 8;

/**
 * The people who carried this one value, ranked.
 *
 * Named, unlike every roster-wide section: the subject is already one
 * repository, and "who works on this" is the question a reader descends to ask.
 * The roster-wide screens stay anonymous — see `AttentionSection` for the only
 * other place a name appears.
 */
function ContributorsSection({
  spec,
  grid,
  memberIds,
  nameByEntity,
}: {
  spec: Extract<SectionSpec, { kind: "contributors" }>;
  grid: GridData;
  memberIds: readonly string[];
  nameByEntity: Map<string, string>;
}) {
  const r = grid.byKey.get(spec.metric);
  if (!r) return null;

  const ranked = entityValues(r, memberIds)
    .filter((e) => e.value > 0)
    .sort((a, b) => b.value - a.value);
  if (ranked.length < 2) return null;

  const limit = spec.limit ?? CONTRIBUTOR_ROWS;
  const bucket = new Map<string, BarEntry>();
  for (const entry of ranked.slice(0, limit)) {
    bucket.set(entry.id, {
      label: nameByEntity.get(entry.id) ?? entry.id,
      value: entry.value,
    });
  }
  const rest = ranked.slice(limit);
  if (rest.length) {
    bucket.set("__rest__", {
      label: `${rest.length} more ${rest.length === 1 ? "person" : "people"}`,
      value: rest.reduce((sum, e) => sum + e.value, 0),
    });
  }

  return (
    <BarList
      title={spec.title}
      rows={toBarRows(bucket)}
      format={r.format}
      unit={r.unit}
    />
  );
}

/**
 * Weekday × two-hour block: when the work lands.
 *
 * The weekday comes from each bucket's own date and the block from the
 * metric's `hour_block` dimension, which is why the request behind this is
 * bucketed by DAY — a week-sized bucket has no weekday to read.
 */
function HeatmapHoursSection({
  spec,
  hourByKey,
  hourIsError,
  memberIds,
  onOpenBlock,
}: {
  spec: Extract<SectionSpec, { kind: "heatmap-hours" }>;
  hourByKey: Map<string, NormalizedMetricResult>;
  hourIsError: boolean;
  memberIds: readonly string[];
  /**
   * The records of one hour block, across the whole period. A CELL cannot be
   * opened: it is one weekday of one block, and the evidence request has no
   * weekday predicate to express that — a dialog for the block alone would
   * answer a wider question than the cell asks.
   */
  onOpenBlock?: (block: string) => void;
}) {
  if (hourIsError) return null;
  const r = hourByKey.get(spec.metric);
  if (!r) return null;

  const { cells, max, total } = dayHourMatrix(
    memberIds.flatMap((id) => forEntity(r, id).series)
  );
  if (total <= 0 || max <= 0) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {spec.title} · {formatMetricValue(total, r.format, r.unit)}
      </p>
      <Card>
        <CardContent className="flex flex-col gap-3 p-4">
          <div
            className="grid gap-px text-xs"
            style={{
              gridTemplateColumns: `3rem repeat(${HOUR_BLOCKS.length}, 1fr)`,
            }}
          >
            <div />
            {HOUR_BLOCKS.map((block, index) => (
              <div key={block} className="text-center text-muted-foreground">
                {onOpenBlock ? (
                  // Every block is openable, including the ones whose label is
                  // suppressed for density: the button fills its column, so an
                  // unlabelled one is still a target for a mouse and a
                  // screen reader both.
                  <button
                    type="button"
                    aria-haspopup="dialog"
                    aria-label={`Open the records from the ${block}:00 block`}
                    onClick={() => onOpenBlock(block)}
                    className="w-full cursor-pointer underline-offset-2 hover:text-foreground hover:underline"
                  >
                    <span aria-hidden={index % 2 !== 0}>
                      {index % 2 === 0 ? block : "·"}
                    </span>
                  </button>
                ) : (
                  <span>{index % 2 === 0 ? block : ""}</span>
                )}
              </div>
            ))}
            {WEEKDAY_LABELS.map((day, dayIndex) => (
              <Fragment key={day}>
                <div className="self-center text-muted-foreground">{day}</div>
                {HOUR_BLOCKS.map((block, blockIndex) => {
                  const value = cells[dayIndex]?.[blockIndex] ?? 0;
                  return (
                    <div
                      key={block}
                      title={`${day} ${block}:00 · ${formatMetricValue(value, r.format, r.unit)}`}
                      className="aspect-square rounded-[3px]"
                      style={{
                        backgroundColor: value
                          ? `color-mix(in oklab, var(--primary) ${Math.round((value / max) * 100)}%, var(--muted))`
                          : "var(--muted)",
                      }}
                    />
                  );
                })}
              </Fragment>
            ))}
          </div>
          <p className="text-xs text-muted-foreground">{spec.caption}</p>
        </CardContent>
      </Card>
    </section>
  );
}

/* ── dimension-table (rollup: one row per dimension value) ───────────── */

const DIMENSION_TABLE_ROWS = 12;

/** The dimension a `heatmap-hours` section reads. */
const HOUR_BLOCK_DIMENSION = "hour_block";

function tableCellText(
  value: number | null | undefined,
  result: NormalizedMetricResult
): string {
  return value == null ? "—" : formatMetricValue(value, result.format, result.unit);
}

function DimensionTableSection({
  spec,
  tableByKey,
  tableIsError,
  tableRefetch,
  onOpenValue,
}: {
  spec: Extract<SectionSpec, { kind: "dimension-table" }>;
  tableByKey: Map<string, NormalizedMetricResult>;
  tableIsError: boolean;
  tableRefetch: () => void;
  /** Descend into one row; absent when the lens has no screen for a value. */
  onOpenValue?: (value: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  if (tableIsError) {
    return (
      <section className="flex flex-col gap-3">
        <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
          {spec.title}
        </p>
        <ComingSoon
          variant="card"
          state="error"
          label={`${spec.title} — unable to load`}
          onRetry={tableRefetch}
        />
      </section>
    );
  }

  const results = spec.metrics
    .map((key) => tableByKey.get(key))
    .filter((r): r is NormalizedMetricResult => r?.rollup != null);
  const primary = results[0];
  if (!primary?.rollup) return null;

  // Rows keyed by the dimension VALUE (the id the response groups by), shown
  // under its label — same rule as composition bars.
  type Row = {
    // The id the response groups by. Two values that happen to share a label
    // are two rows, so the id — not the label — is what identifies one.
    value: string;
    label: string;
    persons: number | null;
    cells: Map<string, number | null>;
  };
  const rows = new Map<string, Row>();
  for (const r of results) {
    for (const v of r.rollup?.values ?? []) {
      const dim = v.dimensions.find((d) => d.key === spec.dimension);
      if (!dim?.value) continue;
      const row: Row = rows.get(dim.value) ?? {
        value: dim.value,
        label: dim.value,
        persons: null,
        cells: new Map(),
      };
      if (dim.label?.trim()) row.label = dim.label.trim();
      row.cells.set(r.metric_key, v.value);
      // The person count is read off the ranking metric only: rollup counts
      // distinct contributors per metric, and mixing counts across metrics
      // would let a row's "people" disagree with the number it ranks by.
      if (r.metric_key === primary.metric_key) {
        row.persons = v.contributing_entity_count;
      }
      rows.set(dim.value, row);
    }
  }
  const ranked = [...rows.values()].sort(
    (a, b) =>
      (b.cells.get(primary.metric_key) ?? 0) -
      (a.cells.get(primary.metric_key) ?? 0)
  );
  // A single row is an empty shell, same rule as composition.
  if (ranked.length < 2) return null;

  const limit = spec.limit ?? DIMENSION_TABLE_ROWS;
  const visible = expanded ? ranked : ranked.slice(0, limit);
  const rest = expanded ? [] : ranked.slice(limit);
  // A remainder only for what still adds up: sums add; a median or a ratio
  // over "everything else" is not derivable from the shown rows.
  const remainder = rest.length
    ? {
        label: `Other (${rest.length})`,
        cells: new Map<string, number | null>(
          results.map((r) => [
            r.metric_key,
            r.computation === "sum"
              ? rest.reduce(
                  (sum, row) => sum + (row.cells.get(r.metric_key) ?? 0),
                  0
                )
              : null,
          ])
        ),
      }
    : null;

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-baseline justify-between gap-3">
        <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
          {spec.title} · {ranked.length} {spec.noun}
        </p>
        {expanded && ranked.length > limit ? (
          <button
            type="button"
            onClick={() => setExpanded(false)}
            className="cursor-pointer text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
          >
            Show top {limit}
          </button>
        ) : null}
      </div>
      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead />
                {results.map((r) => (
                  <TableHead key={r.metric_key} className="text-right">
                    {r.short_label ?? r.label}
                  </TableHead>
                ))}
                <TableHead className="text-right">People</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visible.map((row) => (
                <TableRow key={row.value}>
                  <TableCell
                    className="max-w-64 truncate text-sm font-medium"
                    title={row.label}
                  >
                    {onOpenValue ? (
                      <button
                        type="button"
                        onClick={() => onOpenValue(row.value)}
                        className="max-w-full cursor-pointer truncate underline-offset-2 hover:underline"
                      >
                        {row.label}
                      </button>
                    ) : (
                      row.label
                    )}
                  </TableCell>
                  {results.map((r) => (
                    <TableCell
                      key={r.metric_key}
                      className="text-right tabular-nums"
                    >
                      {tableCellText(row.cells.get(r.metric_key), r)}
                    </TableCell>
                  ))}
                  <TableCell className="text-right tabular-nums">
                    {row.persons ?? "—"}
                  </TableCell>
                </TableRow>
              ))}
              {remainder ? (
                <TableRow className="text-muted-foreground">
                  <TableCell className="text-sm">
                    {/* The remainder is the way into the rows it hides, not a
                      dead summary line: a reader who wants the long tail has
                      nowhere else to ask for it. */}
                    <button
                      type="button"
                      onClick={() => setExpanded(true)}
                      className="cursor-pointer underline-offset-2 hover:text-foreground hover:underline"
                    >
                      {remainder.label}
                    </button>
                  </TableCell>
                  {results.map((r) => (
                    <TableCell
                      key={r.metric_key}
                      className="text-right tabular-nums"
                    >
                      {tableCellText(remainder.cells.get(r.metric_key), r)}
                    </TableCell>
                  ))}
                  <TableCell className="text-right">—</TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </section>
  );
}

/* ── ownership (concentration risk per dimension value; nobody named) ── */

// Above this share in one person's hands a row is flagged — the same
// "one person carries most of it" reading as the bus-factor section.
const OWNERSHIP_RISK_SHARE = 0.6;
const OWNERSHIP_ROWS = 12;

function ownershipPct(share: number): string {
  return `${Math.round(share * 100)}%`;
}

function OwnershipSection({
  spec,
  compData,
  compIsError,
  compRefetch,
  memberIds,
  nameByEntity,
}: {
  spec: Extract<SectionSpec, { kind: "ownership" }>;
  compData: Map<string, NormalizedMetricResult>;
  compIsError: boolean;
  compRefetch: () => void;
  memberIds: readonly string[];
  nameByEntity: Map<string, string>;
}) {
  const [expanded, setExpanded] = useState(false);
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

  const bd = compData.get(spec.metric);
  const byValue = new Map<
    string,
    { label: string; total: number; byEntity: Map<string, number> }
  >();
  if (bd) {
    for (const id of memberIds) {
      for (const row of forEntity(bd, id).breakdown) {
        const dim = row.dimensions.find((d) => d.key === spec.dimension);
        if (!dim?.value || row.value == null || row.value <= 0) continue;
        const bucket = byValue.get(dim.value) ?? {
          label: dim.value,
          total: 0,
          byEntity: new Map<string, number>(),
        };
        if (dim.label?.trim()) bucket.label = dim.label.trim();
        bucket.total += row.value;
        bucket.byEntity.set(id, (bucket.byEntity.get(id) ?? 0) + row.value);
        byValue.set(dim.value, bucket);
      }
    }
  }
  const rows = [...byValue.entries()]
    .map(([value, bucket]) => {
      const ranked = [...bucket.byEntity.entries()].sort((a, b) => b[1] - a[1]);
      const shares = ranked.map(([, share]) => share);
      const top1 = (shares[0] ?? 0) / bucket.total;
      const top3 =
        shares.slice(0, 3).reduce((sum, v) => sum + v, 0) / bucket.total;
      const nameOf = ([id]: [string, number]) => nameByEntity.get(id) ?? id;
      return {
        value,
        label: bucket.label,
        total: bucket.total,
        top1,
        top3,
        // The bar stays anonymous; hovering a segment is the reader asking.
        topName: ranked[0] ? nameOf(ranked[0]) : null,
        nextNames: ranked.slice(1, 3).map(nameOf),
        othersCount: Math.max(0, ranked.length - 3),
      };
    })
    .sort((a, b) => b.total - a.total);
  if (rows.length < 2) return null;
  const visible = expanded ? rows : rows.slice(0, OWNERSHIP_ROWS);

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {spec.title}
      </p>
      <Card>
        <CardContent className="flex flex-col gap-2 p-4">
         <TooltipProvider delay={HOVER_DELAY_MS}>
          <ul
            className="flex flex-wrap items-center gap-x-4 gap-y-1 pb-1"
            aria-label="What each colour in a bar means"
          >
            <li className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <span aria-hidden className="size-2.5 shrink-0 rounded-[2px] bg-primary" />
              Top person
            </li>
            <li className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <span aria-hidden className="size-2.5 shrink-0 rounded-[2px] bg-primary/50" />
              Next 2
            </li>
            <li className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <span aria-hidden className="size-2.5 shrink-0 rounded-[2px] border border-border bg-muted" />
              Everyone else
            </li>
          </ul>
          {visible.map((row) => (
            <div key={row.value} className="flex items-center gap-3">
              <div
                className="w-44 shrink-0 truncate text-sm md:w-64 lg:w-80"
                title={row.label}
              >
                {row.label}
              </div>
              <div
                className="flex h-3.5 flex-1 gap-0.5"
                role="img"
                aria-label={`${row.label}: top person ${ownershipPct(row.top1)}, top three ${ownershipPct(row.top3)}`}
              >
                <OwnershipSegment
                  className="rounded-[3px] bg-primary"
                  width={row.top1 * 100}
                  who={row.topName ? [row.topName] : []}
                  share={ownershipPct(row.top1)}
                  role="Top person"
                />
                {row.top3 > row.top1 ? (
                  <OwnershipSegment
                    className="rounded-[3px] bg-primary/50"
                    width={(row.top3 - row.top1) * 100}
                    who={row.nextNames}
                    share={ownershipPct(row.top3 - row.top1)}
                    role="Next 2"
                  />
                ) : null}
                {row.top3 < 1 ? (
                  <OwnershipSegment
                    className="rounded-[3px] border border-border bg-muted"
                    width={(1 - row.top3) * 100}
                    who={[]}
                    share={ownershipPct(1 - row.top3)}
                    role={
                      row.othersCount
                        ? `Everyone else · ${row.othersCount} ${row.othersCount === 1 ? "person" : "people"}`
                        : "Everyone else"
                    }
                  />
                ) : null}
              </div>
              <div className="w-40 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
                <span
                  className={
                    row.top1 >= OWNERSHIP_RISK_SHARE
                      ? "font-medium text-warning"
                      : "font-medium text-foreground"
                  }
                >
                  top-1 {ownershipPct(row.top1)}
                </span>{" "}
                · top-3 {ownershipPct(row.top3)}
              </div>
            </div>
          ))}
          {rows.length > visible.length || expanded ? (
            <button
              type="button"
              onClick={() => setExpanded((v) => !v)}
              className="cursor-pointer self-start pt-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            >
              {expanded
                ? `Show top ${OWNERSHIP_ROWS}`
                : `+${rows.length - visible.length} more`}
            </button>
          ) : null}
         </TooltipProvider>
        </CardContent>
      </Card>
    </section>
  );
}

/**
 * One share of an ownership bar, named only when hovered.
 *
 * The bar's job is the shape of the concentration, which is why the row itself
 * says no names. A reader who hovers is asking who, and withholding it there
 * makes the section unusable for the decision it exists to inform.
 */
function OwnershipSegment({
  className,
  width,
  who,
  share,
  role,
}: {
  className: string;
  width: number;
  who: readonly string[];
  share: string;
  role: string;
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={<span className={className} style={{ width: `${width}%` }} />}
      />
      <TooltipContent side="top">
        {role} · {share}
        {who.length ? ` · ${who.join(", ")}` : ""}
      </TooltipContent>
    </Tooltip>
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
  const { openDirection } = usePortalNavActions();
  const showPlanned = usePortalShowPlanned();
  const cards = overviewCardDirections(showPlanned)
    .map((d) => {
      const entry = lensEntry(d.id, "Overview");
      // Overview lenses are person-grain by construction; a tenant entry here
      // would have no roster to preview, so it contributes no card.
      if (!entry || "comingSoon" in entry || "entity" in entry) return null;
      const gated = visibleSections(entry, showPlanned);
      const headline = gated.sections.find(
        (s): s is Extract<SectionSpec, { kind: "headline" }> =>
          s.kind === "headline"
      );
      if (!headline) return null;
      const keys =
        variant === "compact" ? headline.metrics.slice(0, 2) : headline.metrics;
      const observed = familyObserved(
        grid.byKey,
        sectionMetricKeys(gated),
        memberIds
      );
      return { id: d.id, name: d.name, keys, observed };
    })
    .filter((c): c is NonNullable<typeof c> => c != null);
  if (!cards.length) return null;

  const go = (dir: string) => openDirection(dir, "Overview");

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
                  this data source is not connected yet
                </span>
              )}
            </div>
          </button>
        ))}
      </div>
    </section>
  );
}

/* ── by-unit auto-section (rule 7) ───────────────────────────────────── */

const NO_COMPARABLE_UNITS_NOTE =
  "Nothing to compare at this grouping: it needs at least two groups of four or more people, and a metric that can be added up.";

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
      <SliceNote text={`Grouping by ${planned.label} is not available yet.`} />
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
  const bucket = new Map<string, BarEntry>();
  for (const [unit, ids] of byUnit) {
    if (ids.length < MIN_COHORT) continue; // small-cohort suppression
    const v = perCapita(r, ids);
    if (v > 0) bucket.set(unit, { label: `${unit} · ${ids.length}`, value: v });
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

/** One slice of a bar: a value of the split dimension, and its share of the row. */


/**
 * A share, as a reader should see it. "0%" beside a non-zero count reads as
 * "none of it" — which is not what a small share means.
 */
function shareLabel(pct: number): string {
  if (pct > 0 && pct < 1) return "<1%";
  return `${Math.round(pct)}%`;
}

/** Same dwell as every other hover explanation in the product. */
const HOVER_DELAY_MS = 400;

/** Rows shown before the reader opts into the full list. */
const BAR_LIST_COLLAPSED = 12;

/**
 * One fill inside a bar — a button when its records can be opened, a plain
 * span when they cannot. Same geometry either way, so a list where only some
 * rows are drillable still draws one row of bars.
 */
function BarPiece({
  className,
  style,
  onOpen,
  label,
}: {
  className: string;
  style?: CSSProperties;
  onOpen?: () => void;
  label: string;
}) {
  if (!onOpen) return <span className={className} style={style} />;
  return (
    <button
      type="button"
      aria-haspopup="dialog"
      aria-label={`Open the records behind ${label}`}
      onClick={onOpen}
      className={cn(
        className,
        "cursor-pointer transition-opacity hover:opacity-80 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring"
      )}
      style={style}
    />
  );
}

export function BarList({
  title,
  rows,
  format,
  unit,
  showShare = true,
  notes,
  onOpen,
  canOpen,
}: {
  title: string;
  rows: BarRow[];
  format: NormalizedMetricResult["format"];
  unit: string | null;
  /** False for per-capita values, where a share-of-total percent would mislead. */
  showShare?: boolean;
  /** Below the bars, not above: it explains what was read, not what to read. */
  notes?: readonly string[];
  /**
   * The records behind one bar, or behind one of its segments. Omitted where
   * the caller cannot narrow the figure — the bars then stay inert rather
   * than offering a dialog that would answer a different question.
   */
  onOpen?: (row: BarRow, segment?: BarSegment) => void;
  /**
   * Whether ONE piece can be narrowed, when the list as a whole can. A
   * segment the response did not name has no value to filter on, and an
   * affordance that opens an empty dialog is worse than none.
   */
  canOpen?: (row: BarRow, segment?: BarSegment) => boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const visible = expanded ? rows : rows.slice(0, BAR_LIST_COLLAPSED);
  const max = rows[0]?.value || 1;
  // Seeded from EVERY row, not the visible page: a colour must not change
  // meaning when the reader expands the list. Ordered by total so the legend
  // reads biggest-first, like the bars.
  const totals = new Map<string, { label: string; value: number }>();
  for (const row of rows) {
    for (const segment of row.segments ?? []) {
      const seen = totals.get(segment.seed);
      totals.set(segment.seed, {
        label: seen?.label ?? segment.label,
        value: (seen?.value ?? 0) + segment.value,
      });
    }
  }
  const colors = seriesColors([...totals.keys()]);
  const legend = [...totals.entries()]
    .sort((a, b) => b[1].value - a[1].value)
    .map(([seed, { label }]) => ({ seed, label, color: colors[seed] }));

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {title}
      </p>
      <Card>
        <CardContent className="flex flex-col gap-2 p-4">
          <TooltipProvider delay={HOVER_DELAY_MS}>
            {legend.length ? (
              <ul
                className="flex flex-wrap items-center gap-x-4 gap-y-1 pb-1"
                aria-label="What each colour in a bar means"
              >
                {legend.map((entry) => (
                  <li
                    key={entry.seed}
                    className="flex items-center gap-1.5 text-xs text-muted-foreground"
                  >
                    <span
                      aria-hidden
                      className="size-2.5 shrink-0 rounded-[2px]"
                      style={{ backgroundColor: entry.color }}
                    />
                    {entry.label}
                  </li>
                ))}
              </ul>
            ) : null}
            {visible.map((row) => (
              <div key={row.key} className="flex items-center gap-3">
                {/* A dimension value is often a path (`owner/repo`), and the part
                  that tells two rows apart is at the END — so the column grows
                  with the viewport and the full value stays on hover. */}
                <div
                  className="w-44 shrink-0 truncate text-sm md:w-64 lg:w-80"
                  title={row.label}
                >
                  {row.href ? (
                    <a
                      href={row.href}
                      target="_blank"
                      rel="noreferrer"
                      className="hover:text-foreground hover:underline"
                    >
                      {row.label}
                    </a>
                  ) : (
                    row.label
                  )}
                </div>
                <div className="h-6 flex-1 overflow-hidden rounded bg-muted">
                  {/* One bar, however it is cut: the width is the row's share of
                    the largest row, and the segments divide THAT width, so a
                    split bar stays comparable with an unsplit one. */}
                  <div
                    className="flex h-full overflow-hidden rounded"
                    style={{ width: `${Math.round((row.value / max) * 100)}%` }}
                  >
                    {row.segments?.length ? (
                      row.segments.map((segment) => (
                        <Tooltip key={segment.seed}>
                          <TooltipTrigger
                            render={
                              <BarPiece
                                className="h-full first:rounded-s last:rounded-e"
                                style={{
                                  width: `${(segment.value / row.value) * 100}%`,
                                  backgroundColor: colors[segment.seed],
                                }}
                                onOpen={
                                  onOpen &&
                                  (canOpen?.(row, segment) ?? true)
                                    ? () => onOpen(row, segment)
                                    : undefined
                                }
                                label={`${row.label} · ${segment.label}`}
                              />
                            }
                          />
                          <TooltipContent side="top">
                            {segment.label} ·{" "}
                            {formatMetricValue(segment.value, format, unit)} ·{" "}
                            {shareLabel((segment.value / row.value) * 100)}
                            {onOpen ? " · click to open the records" : ""}
                          </TooltipContent>
                        </Tooltip>
                      ))
                    ) : (
                      <BarPiece
                        className="h-full w-full rounded bg-primary/25"
                        onOpen={
                          onOpen && (canOpen?.(row) ?? true)
                            ? () => onOpen(row)
                            : undefined
                        }
                        label={row.label}
                      />
                    )}
                  </div>
                </div>
                {/* Beside the bar, not on it: a segment fill is a full-strength
                  colour, and no single text colour stays legible across every
                  hue a palette can hand out. */}
                {/* Fixed width, right-aligned: sized by its content the column
                  would differ on every row, and every bar would end somewhere
                  else. */}
                <div className="w-28 shrink-0 text-right text-xs font-medium text-muted-foreground tabular-nums md:w-36">
                  {formatMetricValue(row.value, format, unit)}
                  {showShare ? ` · ${shareLabel(row.pct)}` : ""}
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
          </TooltipProvider>
          {notes?.length ? (
            <div className="mt-4 flex flex-col gap-1 border-t pt-3">
              {notes.map((note) => (
                <p key={note} className="text-xs text-muted-foreground">
                  {note}
                </p>
              ))}
            </div>
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

export function Delta({
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
