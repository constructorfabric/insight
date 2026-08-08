import { useMemo, useState } from "react";

import { useViewer } from "@/auth";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { GroupDrilldownSheet } from "@/components/widgets/dashboard/group-drilldown-sheet";
import { IcNeedsAttention } from "@/components/widgets/dashboard/ic-needs-attention";
import { PersonCoverage } from "@/components/widgets/dashboard/person-coverage";
import { KpiTile } from "@/components/widgets/dashboard/kpi-tile";
import { previousPeriodRange } from "@/api/period-to-date-range";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import type { PeriodValue } from "@/types/insight";
import { useSettings } from "@/hooks/use-settings";
import {
  metricAttentionItems,
  orderAttentionItems,
} from "@/lib/insight/attention";
import { typicalPeerPool } from "@/lib/insight/peer-pool";
import {
  KPI_ROW,
  KPI_ROW_COLLECTION,
  GROUPS,
  type GroupId,
} from "@/lib/insight/groups";
import { metricKpiTiles } from "@/lib/insight/kpi-row";
import { injectCohortPeer } from "@/lib/insight/within-team-peer";
import {
  projectViews,
  type MetricCollectionConfig,
} from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import { cn } from "@/lib/utils";
import { useCohortLabel } from "@/lib/portal/use-cohort-label";
import { usePersonCohort } from "@/lib/portal/use-person-cohort";
import {
  personTrendPoints,
  runningBucketStart,
  TREND_BUCKETS,
  trendBucket,
  trendRange,
} from "@/lib/portal/person-trend";
import { usePersonSectionStandings } from "@/lib/portal/use-person-sections";
import {
  collectionSetPending,
  useMetricCollection,
  useMetricCollectionSet,
} from "@/queries/metric-results";

const EMPTY_COLLECTION: MetricCollectionConfig = { metrics: [] };

/**
 * Column counts by how many tiles there are. Written out rather than composed,
 * because the class scanner reads source text and cannot see a name built at
 * runtime.
 */
const TILE_GRID: Record<number, string> = {
  0: "grid-cols-1",
  1: "grid-cols-1",
  2: "grid-cols-1 sm:grid-cols-2",
  3: "grid-cols-1 sm:grid-cols-2 lg:grid-cols-3",
  4: "grid-cols-1 sm:grid-cols-2 lg:grid-cols-4",
};

/** What the previous period IS, so the badge can be named in plain words. */
const PERIOD_NOUN: Record<PeriodValue, string> = {
  week: "week",
  month: "month",
  quarter: "quarter",
  year: "year",
};
const CLOSED_ENTITY = { type: "person" as const, ids: [] };
const CLOSED_DRILLDOWN_DATA = {
  byKey: new Map(),
  previousByKey: null,
  isPending: true,
  isFetching: false,
  isError: false,
  refetch: () => {},
} as const;

/** The person and catalogue slice a metric-groups screen renders. */
export interface MetricGroupsViewProps {
  /** Person id the lens is scoped to (org-level rollup is a backend follow-up). */
  personId: string;
  /** Which metric-family groups to render; empty ⇒ nothing available yet. */
  groupIds: readonly GroupId[];
  /** Overview renders the KPI row + needs-attention; direction lenses don't. */
  showKpis?: boolean;
  /**
   * When set, a group card selects the group via this callback (e.g. to expand
   * it inline in a second sidebar level) instead of opening the drilldown
   * modal. Passing it also suppresses the modal sheets entirely.
   */
  onSelectGroup?: (id: GroupId) => void;
}

/**
 * Catalog-driven metric-group screen — the reusable body behind the portal's
 * Overview and Direction lenses. Reuses the v2 dashboard widgets
 * (KpiTile / MetricGroupCard / GroupDrilldownSheet) and the `/metric-results`
 * collection queries, parameterised by the set of groups to show.
 */
export function MetricGroupsView({
  personId,
  groupIds,
  showKpis = false,
  onSelectGroup,
}: MetricGroupsViewProps) {
  const cohortLabel = useCohortLabel();
  const { period, dateRange } = usePortalPeriod();
  const { focusMode } = useSettings();
  const entityId = normalizePersonId(personId);
  // Whose page this is. A person reading their own page and a manager
  // reading it want different things first — see `orderAttentionItems`.
  const { personId: viewerPersonId } = useViewer();
  const isSelf = normalizePersonId(viewerPersonId ?? "") === entityId;
  const entity = { type: "person" as const, ids: [entityId] };

  const defs = GROUPS.filter((d) => groupIds.includes(d.id));

  const kpiData = useMetricCollection(
    showKpis ? KPI_ROW_COLLECTION : EMPTY_COLLECTION,
    showKpis ? entity : CLOSED_ENTITY,
    dateRange,
    { previousPeriod: period }
  );
  const groupData = useMetricCollectionSet(
    defs.map((def) => ({
      key: def.id,
      collection: projectViews(def.collection, ["period", "peer"]),
    })),
    entity,
    dateRange
  );
  // The same collections over the previous period: attention needs to know
  // whether a standing is also a change (see `metricAttentionItems`), and
  // "period" alone is enough for that — no peer stats needed.
  const previousRange = previousPeriodRange(dateRange, period);
  const previousGroupData = useMetricCollectionSet(
    showKpis
      ? defs.map((def) => ({
          key: def.id,
          collection: projectViews(def.collection, ["period"]),
        }))
      : [],
    showKpis ? entity : CLOSED_ENTITY,
    previousRange
  );

  // Slice cohort: the people who share this person's active-slice attribute
  // value. Only fetched when a slice is picked — otherwise the person's own
  // numbers stand alone (no cohort, tiles show "no peer data" as before).
  const cohortIds = usePersonCohort(entityId);
  const cohortEntity = cohortIds.length
    ? { type: "person" as const, ids: cohortIds }
    : CLOSED_ENTITY;
  const cohortKpi = useMetricCollection(
    cohortIds.length && showKpis ? KPI_ROW_COLLECTION : EMPTY_COLLECTION,
    // Entity gated on the SAME condition as the collection — a live entity
    // with an empty collection still issues a useless network request.
    cohortIds.length && showKpis ? cohortEntity : CLOSED_ENTITY,
    dateRange
  );
  const cohortGroup = useMetricCollectionSet(
    cohortIds.length
      ? defs.map((def) => ({
          key: def.id,
          collection: projectViews(def.collection, ["period"]),
        }))
      : [],
    cohortEntity,
    dateRange
  );

  const sectionStandings = usePersonSectionStandings(personId);

  // Deliberately outside every loading and error gate below: the tiles render
  // from the current period, and the line appears when it arrives. A first
  // screen must not wait on decoration.
  const bucket = trendBucket(period);
  const trendCollection = useMemo(
    () => ({
      metrics: KPI_ROW.map((key) => ({
        key,
        views: [{ view: "timeseries" as const, bucket }],
      })),
    }),
    [bucket]
  );
  const trendData = useMetricCollection(
    showKpis ? trendCollection : EMPTY_COLLECTION,
    showKpis ? entity : CLOSED_ENTITY,
    trendRange(dateRange.to, bucket)
  );

  const [openGroup, setOpenGroup] = useState<GroupId | null>(null);
  // With `onSelectGroup` a card/alert navigates to the section inline; without
  // it, it opens the drilldown modal.
  const openOrSelect = onSelectGroup ?? setOpenGroup;
  const openDef =
    openGroup != null ? (defs.find((d) => d.id === openGroup) ?? null) : null;
  const drilldownData = useMetricCollection(
    openDef?.collection ?? EMPTY_COLLECTION,
    openDef ? entity : CLOSED_ENTITY,
    dateRange
  );

  const [prevPersonId, setPrevPersonId] = useState(personId);
  if (personId !== prevPersonId) {
    setPrevPersonId(personId);
    setOpenGroup(null);
  }

  if (defs.length === 0) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="empty"
          label="Not in the semantic layer yet — bullet-only direction"
        />
      </div>
    );
  }

  // The previous period counts as part of the picture: attention needs it to
  // tell a change from a standing, so a pending one is still loading and a
  // failed one is an error. Left out, a failed comparison would render as
  // "nothing needs attention" — the most misleading empty state on the page.
  //
  // The cohort's own results count too. Every comparison on the page is drawn
  // against them — `injectCohortPeer` only copies peer values that have
  // arrived — so rendering before they do shows the person measured against a
  // cohort that is not yet there, under a heading naming how many people are
  // in it.
  const cohortPending =
    cohortIds.length > 0 &&
    ((showKpis && cohortKpi.isPending) || collectionSetPending(cohortGroup));
  const isLoading =
    (showKpis &&
      (kpiData.isPending || collectionSetPending(previousGroupData))) ||
    collectionSetPending(groupData) ||
    cohortPending;
  if (isLoading) return <CenteredSpinner className="min-h-[60vh]" />;

  // Surface a backend failure as a retryable error, not empty section cards.
  const isError =
    (showKpis &&
      (kpiData.isError ||
        [...previousGroupData.values()].some((r) => r.isError))) ||
    [...groupData.values()].some((r) => r.isError) ||
    (cohortIds.length > 0 &&
      ((showKpis && cohortKpi.isError) ||
        [...cohortGroup.values()].some((r) => r.isError)));
  if (isError)
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          onRetry={() => {
            kpiData.refetch();
            groupData.forEach((r) => r.refetch());
            previousGroupData.forEach((r) => r.refetch());
            cohortKpi.refetch();
            cohortGroup.forEach((r) => r.refetch());
          }}
        />
      </div>
    );

  // Cohort-injected views: with a slice active, the person's results carry
  // their cohort's peer stats so tiles/cards/attention read "vs <slice> median".
  const kpiByKey = injectCohortPeer(kpiData.byKey, cohortKpi.byKey, cohortIds);
  const groupResult = (id: GroupId) => {
    const r = groupData.get(id);
    if (!r) return undefined;
    const cr = cohortGroup.get(id);
    if (!cohortIds.length || !cr) return r;
    return { ...r, byKey: injectCohortPeer(r.byKey, cr.byKey, cohortIds) };
  };

  const peerPool = showKpis ? typicalPeerPool(kpiByKey, entityId) : null;

  const tiles = showKpis
    ? metricKpiTiles(kpiByKey, kpiData.previousByKey, entityId, focusMode)
    : [];

  // What the row actually rendered — the block skips exactly those, no more.
  const headlineKeys = new Set(tiles.map((t) => t.key));

  // Sections with no reading at all, split by whose fault the blank is: a
  // pool that reads means the measurement works and this person is absent
  // from it, an empty pool means nobody here is measured. A page that shows
  // only what it can see otherwise reads as a whole picture of a person.
  const blank = showKpis
    ? sectionStandings.filter((st) => !st.isPending && !st.hasData)
    : [];
  const unmeasured = blank.filter((st) => !st.peersHaveData).map((st) => st.title);
  const inactive = blank.filter((st) => st.peersHaveData).map((st) => st.title);

  // Deduped across groups, not within one: a metric and the wider metric that
  // contains it need not live in the same section.
  // Thinned across groups AND against the row above: a metric and the one it
  // restates need not live in the same section, and the row is the more
  // prominent of the two places a fact can appear.
  const attentionItems = showKpis
    ? orderAttentionItems(
        defs.flatMap((def) =>
          metricAttentionItems(
            def,
            groupResult(def.id)?.byKey ?? new Map(),
            previousGroupData.get(def.id)?.byKey ?? null,
            entityId,
            headlineKeys
          )
        ),
        headlineKeys,
        isSelf
      )
    : [];

  return (
    <>
      <main className="flex flex-1 flex-col gap-8 p-4 md:p-6">
        {showKpis ? (
          <>
            {/* Before the numbers, not after them. On a person's own page the
                first question is what this thing knows about them, and a page
                that answers it only at the bottom has already been read as a
                complete picture by then. */}
            <PersonCoverage unmeasured={unmeasured} inactive={inactive} />
            <section className="flex flex-col gap-3">
              <p className="flex flex-wrap items-baseline gap-x-2 text-xs font-medium tracking-wider text-muted-foreground uppercase">
                At a glance
                {/* Whose median. Every comparison on this page says "vs median"
                    and none of them said against whom — which is the one thing
                    a reader needs to judge any of it, and the one that reveals
                    a lead being measured against their own reports. */}
                {peerPool ? (
                  <span className="font-normal tracking-normal normal-case">
                    · compared with {peerPool} people in the same {cohortLabel}
                  </span>
                ) : null}
                {/* The lines carry no axis and no labels — a tile has room for
                    neither — so the one thing they cannot say for themselves
                    is said once, here: what a line spans. */}
                <span className="font-normal tracking-normal normal-case">
                  · lines cover the last {TREND_BUCKETS}{" "}
                  {bucket === "week" ? "weeks" : "months"}
                </span>
              </p>
              {/* A counted column grid, not auto-fit: auto-fit packs in as many
                  tiles as the width allows and leaves whatever is left over on
                  its own. The count follows how many tiles there ARE, so a
                  person with two of them gets two full-width tiles rather than
                  half a row of blanks — which reads as tiles that failed to
                  load, not as a person with two measurements. */}
              <div
                className={cn(
                  "grid gap-3",
                  TILE_GRID[tiles.length] ?? TILE_GRID[4]
                )}
              >
                {/* The tiles ARE the row: `metricKpiTiles` already picked the
                    metrics this person is observed for, in candidate order. A
                    slot per fixed key is what painted "—" over the most
                    valuable space on the page. */}
                {tiles.map((tile) => (
                  <KpiTile
                    key={tile.key}
                    tile={tile}
                    periodNoun={PERIOD_NOUN[period]}
                    trend={personTrendPoints(
                      trendData.byKey.get(tile.key),
                      entityId,
                      runningBucketStart(dateRange.to, bucket)
                    )}
                    onOpenGroup={openOrSelect}
                  />
                ))}
              </div>
            </section>
            <IcNeedsAttention
              items={attentionItems}
              onOpenGroup={openOrSelect}
            />
          </>
        ) : null}
      </main>

      {/* A click on a number is answered here, with the evidence behind it.
          This used to be gated on a `showSections` flag that also hid the
          per-group cards — and those cards moved to the navigation long ago,
          so the flag survived only to make clicks land nowhere. Only a caller
          that routes clicks elsewhere (`onSelectGroup`) has no use for the
          drilldowns. */}
      {onSelectGroup
        ? null
        : defs.map((def) => (
            <GroupDrilldownSheet
              key={def.id}
              open={openGroup === def.id}
              onOpenChange={(o) => setOpenGroup(o ? def.id : null)}
              def={def}
              metricTarget={{
                kind: "person",
                entityId,
                data:
                  def.id === openGroup ? drilldownData : CLOSED_DRILLDOWN_DATA,
              }}
              range={dateRange}
              period={period}
              cohortLabel={cohortLabel}
            />
          ))}
    </>
  );
}
