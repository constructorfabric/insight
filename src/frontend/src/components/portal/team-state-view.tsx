import { useMemo } from "react";

import { AttentionList } from "@/components/portal/attention-list";
import { orgScopeGate } from "@/components/portal/org-scope-gate";
import { MembersGrid } from "@/components/widgets/dashboard/members-grid";
import { Card, CardContent } from "@/components/ui/card";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { formatMetricValue } from "@/lib/format";
import {
  attentionSummary,
  computeAttentionFlags,
} from "@/lib/insight/attention-flags";
import { headlineMetricKeys, GROUPS } from "@/lib/insight/groups";
import {
  cohortKey,
  collectRosterAttrs,
} from "@/lib/insight/slices";
import { quantile, withinCohortPeer } from "@/lib/insight/within-team-peer";
import {
  forEntity,
  type MetricCollectionConfig,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import {
  usePortalSlice,
} from "@/lib/portal/portal-nav";
import type { TeamMember } from "@/types/insight";
import { useCohortLabel } from "@/lib/portal/use-cohort-label";
import { useOrgScope } from "@/lib/portal/use-org-scope";
import { useMemberGridData } from "@/queries/member-grid";
import { TEXT_FIGURE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const EMPTY_COLLECTION: MetricCollectionConfig = { metrics: [] };

/**
 * Team-state dashboard — the People roster reframed for a lead: "what's the
 * overall state, and where do I look?". Peer-cohort stats aren't computed yet,
 * so signals are honest and self-contained:
 *   • outlier — a member in the worse quartile *within this team*;
 *   • decline — a member's metric materially worse than last period;
 *   • collapse — zero on an activity the team otherwise shows.
 * The AI summary is rule-based for now (a real insights endpoint would slot in).
 * The roster is the active org scope (topbar) — the view owns no scoping of its
 * own.
 */
export function TeamStateView() {
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
    [roster],
  );
  const memberIds = useMemo(
    () => members.map((m) => normalizePersonId(m.person_id)),
    [members],
  );
  const nameByEntity = useMemo(
    () => new Map(members.map((m) => [normalizePersonId(m.person_id), m.name])),
    [members],
  );
  const personIdByEntity = useMemo(
    () => new Map(members.map((m) => [normalizePersonId(m.person_id), m.person_id])),
    [members],
  );

  // Active slice → each person's peer cohort (whole roster when no slice set).
  const attrByEntity = useMemo(
    () => collectRosterAttrs(pivot, normalizePersonId),
    [pivot],
  );
  const slice = usePortalSlice();
  const cohortOf = useMemo(
    () => (id: string) => cohortKey(attrByEntity.get(id), slice),
    [attrByEntity, slice],
  );
  const cohortLabel = useCohortLabel();

  // Headline metrics only (card.preview): the set a lead scans, and — crucially —
  // small enough to stay under the API's 50-metrics-per-request cap when the full
  // metric catalog across every group would blow past it. `GROUPS` is a module
  // constant, so it is read inside the memo and left out of the dependency
  // list — listing it would be noise, not safety.
  const headlineKeys = useMemo(() => headlineMetricKeys(), []);
  const gridCollection = useMemo<MetricCollectionConfig>(() => {
    const want = new Set(headlineKeys);
    const byKey = new Map<string, MetricCollectionConfig["metrics"][number]>();
    for (const g of GROUPS) {
      for (const m of g.collection.metrics) {
        if (want.has(m.key) && !byKey.has(m.key)) byKey.set(m.key, m);
      }
    }
    return { metrics: [...byKey.values()] };
  }, [headlineKeys]);

  const grid = useMemberGridData(
    gridCollection.metrics.length ? gridCollection : EMPTY_COLLECTION,
    { type: "person", ids: memberIds },
    dateRange,
    period,
  );

  const teamName = orgScope.label;

  // ── Headline: team totals (summable) / medians (everything else) ──
  const summary = useMemo(() => {
    return headlineKeys
      .map((key) => grid.byKey.get(key))
      .filter((r): r is NormalizedMetricResult => Boolean(r))
      .map((r) => {
        const vals = memberIds
          .map((id) => forEntity(r, id).value)
          .filter((v): v is number => v != null && Number.isFinite(v));
        if (vals.length === 0) return null;
        const isSum = r.computation === "sum";
        const value = isSum
          ? vals.reduce((a, b) => a + b, 0)
          : quantile([...vals].sort((a, b) => a - b), 0.5);
        return {
          key: r.metric_key,
          label: r.short_label ?? r.label,
          text: formatMetricValue(value, r.format, r.unit),
          kind: isSum ? "team total" : "team median",
        };
      })
      .filter((x): x is NonNullable<typeof x> => x != null)
      .slice(0, 6);
  }, [headlineKeys, grid.byKey, memberIds]);

  // ── Attention: shared within-cohort outliers + declines + collapses ──
  const flags = useMemo(
    () =>
      computeAttentionFlags({
        headlineKeys,
        byKey: grid.byKey,
        previousByKey: grid.previousByKey,
        memberIds,
        cohortOf,
        nameOf: (id) => nameByEntity.get(id) ?? id,
        personIdOf: (id) => personIdByEntity.get(id) ?? id,
        cohortLabel,
      }),
    [
      headlineKeys,
      grid.byKey,
      grid.previousByKey,
      memberIds,
      nameByEntity,
      personIdByEntity,
      cohortOf,
      cohortLabel,
    ],
  );

  // Columns with data for at least one member (drop all-empty ones, e.g. the
  // un-ingested task metrics), and a within-team heat overlay for the grid.
  const shownKeys = useMemo(
    () =>
      headlineKeys.filter((k) => {
        const r = grid.byKey.get(k);
        return (
          !!r &&
          memberIds.some((id) => {
            const v = forEntity(r, id).value;
            return v != null && Number.isFinite(v);
          })
        );
      }),
    [headlineKeys, grid.byKey, memberIds],
  );
  const heatByKey = useMemo(() => {
    const m = new Map<string, NormalizedMetricResult>();
    for (const k of shownKeys) {
      const r = grid.byKey.get(k);
      if (r) m.set(k, withinCohortPeer(r, memberIds, cohortOf));
    }
    return m;
  }, [shownKeys, grid.byKey, memberIds, cohortOf]);

  const gate = orgScopeGate({
    viewerLoading: orgScope.isLoading,
    viewerError: orgScope.isError,
    membersLoading: false,
    membersError: false,
    memberCount: members.length,
    gridPending: grid.isPending,
    gridError: grid.isError,
    emptyLabel: "No people in the current scope. Pick a different scope at the top of the page.",
    onRetry: () => {
      orgScope.refetch();
      grid.refetch();
    },
  });
  if (gate) return gate;

  const flaggedPeople = new Set(flags.map((f) => f.personId)).size;

  return (
    <div className="flex flex-col gap-6 p-4 md:p-6">
      <div>
        <h1 className="text-lg font-semibold tracking-tight">
          {teamName ? `${teamName}'s team` : "Team"}
        </h1>
        <p className="text-sm text-muted-foreground">
          {orgScope.count} people · state &amp; attention
        </p>
      </div>

      {/* Team state at a glance */}
      {summary.length > 0 ? (
        <div className="grid grid-cols-[repeat(auto-fit,minmax(10rem,1fr))] gap-3">
          {summary.map((s) => (
            <Card key={s.key}>
              <CardContent className="p-4">
                <div className="text-xs font-medium text-muted-foreground">{s.label}</div>
                <div className={cn("mt-1", TEXT_FIGURE)}>{s.text}</div>
                <div className="text-xs text-muted-foreground">{s.kind}</div>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : null}

      {/* What to look at — shared with the org Overview */}
      <AttentionList
        flags={flags}
        summary={attentionSummary(flags, flaggedPeople, members.length)}
      />

      {/* Detailed scan */}
      <section className="flex flex-col gap-3">
        <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
          Members
        </p>
        <Card>
          <CardContent className="p-0">
            <MembersGrid
              members={members.map((m) => ({
                entityId: normalizePersonId(m.person_id),
                displayName: m.name,
                personId: m.person_id,
              }))}
              metricKeys={shownKeys}
              byKey={heatByKey}
              previousByKey={grid.previousByKey}
              caption={`${teamName || "Team"} — people × metrics`}
              cohortLabel={cohortLabel}
            />
          </CardContent>
        </Card>
      </section>
    </div>
  );
}
