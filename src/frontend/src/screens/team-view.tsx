import { useMemo, useState } from "react";

import { personDisplayName, personName } from "@/lib/identities/person-display";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { DashboardEmptyState } from "@/components/widgets/dashboard/dashboard-empty-state";
import { DashboardHeader } from "@/components/widgets/dashboard/dashboard-header";
import { GroupDrilldownSheet } from "@/components/widgets/dashboard/group-drilldown-sheet";
import { MembersOverview } from "@/components/widgets/dashboard/members-overview";
import { TeamMembersAttention } from "@/components/widgets/dashboard/team-members-attention";
import { TeamMetricGroupCard } from "@/components/widgets/metric-views/team-metric-group-card";
import type { TeamMemberRef } from "@/components/widgets/metric-views/team-collection-drilldown";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { Switch } from "@/components/ui/switch";
import { usePeriod } from "@/hooks/use-period";
import {
  flattenSubordinates,
  hasIndirectReports,
  rosterTree,
  scopeRosterToDirectReports,
} from "@/lib/insight/identity-tree";
import { GROUPS, HEATMAP_COLLECTION, type GroupId } from "@/lib/insight/groups";
import {
  memberMetricEntries,
  metricBelowCounts,
} from "@/lib/insight/team-metrics";
import { projectViews } from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import {
  collectionSetPending,
  useMetricCollectionSet,
} from "@/queries/metric-results";
import { useMemberGridData } from "@/queries/member-grid";
import { useVisibleRoster } from "@/queries/visible-roster";
import type { TeamMember } from "@/types/insight";

// Team surfaces request period + peer only: a per-member timeseries over a
// large roster would exceed the backend's all-or-nothing row limit and fail
// the whole request.
const TEAM_METRIC_COLLECTIONS = GROUPS.map((def) => ({
  key: def.id,
  collection: projectViews(def.collection, ["period", "peer"]),
}));

export interface TeamViewScreenProps {
  /** Pivot person id whose subtree the table shows. */
  teamId: string;
}

export function TeamViewScreen({ teamId }: TeamViewScreenProps) {
  const { period, dateRange, setPeriod } = usePeriod();
  const [openGroup, setOpenGroup] = useState<GroupId | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const openDetails = (group: GroupId) => {
    setOpenGroup(group);
    setDetailsOpen(true);
  };
  const [directReportsOnly, setDirectReportsOnly] = useState(true);

  // Close any open drilldown when the viewed team changes. Render-phase
  // reset against the previous id rather than an effect (no cascading commit).
  const [prevTeamId, setPrevTeamId] = useState(teamId);
  if (teamId !== prevTeamId) {
    setPrevTeamId(teamId);
    setOpenGroup(null);
  }

  const visibleRoster = useVisibleRoster(true);
  const pivot = useMemo(
    () => rosterTree(visibleRoster.roster, teamId),
    [teamId, visibleRoster.roster]
  );

  const fullRoster = useMemo(
    () => (pivot ? flattenSubordinates(pivot) : null),
    [pivot]
  );
  // With no indirect reports, direct reports == the whole team, so the
  // toggle could never change the roster — hide it (#1756).
  const canScopeToDirectReports = hasIndirectReports(fullRoster);
  // Scoping the roster scopes everything downstream — members, the heatmap,
  // and metric collections all derive from it.
  const roster = useMemo(
    () =>
      scopeRosterToDirectReports(
        fullRoster,
        canScopeToDirectReports && directReportsOnly
      ),
    [fullRoster, canScopeToDirectReports, directReportsOnly]
  );
  // Never fall back to the raw id (a UUID).
  const teamName = pivot ? (personName(pivot) ?? "") : "";

  // The roster IS the member list: identity owns who is on the team, and
  // every metric for them comes from `/v1/metric-results` below. There is no
  // second source to reconcile.
  const members = useMemo<TeamMember[]>(
    () =>
      (roster ?? []).map((entry) => ({
        person_id: entry.person_id,
        name: personDisplayName(entry),
      })),
    [roster]
  );
  const memberEntityIds = members.map((m) => normalizePersonId(m.person_id));
  const memberRefs: TeamMemberRef[] = members.map((m) => ({
    entityId: normalizePersonId(m.person_id),
    displayName: m.name,
  }));
  const heatmapQ = useMemberGridData(
    HEATMAP_COLLECTION,
    { type: "person", ids: memberEntityIds },
    dateRange,
    period
  );

  const metricGroupData = useMetricCollectionSet(
    TEAM_METRIC_COLLECTIONS,
    { type: "person", ids: memberEntityIds },
    dateRange
  );

  const metricBelowByMember = new Map<string, number>();
  for (const def of GROUPS) {
    const byKey = metricGroupData.get(def.id)?.byKey;
    if (!byKey) continue;
    for (const [memberId, count] of metricBelowCounts(
      def,
      byKey,
      memberEntityIds
    )) {
      metricBelowByMember.set(
        memberId,
        (metricBelowByMember.get(memberId) ?? 0) + count
      );
    }
  }

  // Per-person entries (git/ai) feed the heatmap's member details sheet.
  const metricEntriesByPerson = memberMetricEntries(
    GROUPS,
    (id) => metricGroupData.get(id)?.byKey,
    memberEntityIds
  );

  // The one loading gate: a single page spinner while ANY of the screen's
  // queries has no data. A period or scope change mints new query keys, so
  // the same gate re-trips — no per-widget loaders, no partial paints. The
  // roster query comes first: every other query derives its entity ids from it.
  const isLoading =
    visibleRoster.isPending ||
    heatmapQ.isPending ||
    collectionSetPending(metricGroupData);
  const hasMembers = members.length > 0;
  const isAllEmpty = !isLoading && !hasMembers;
  const pivotMissing =
    !visibleRoster.isPending && !visibleRoster.isError && pivot === null;

  const memberCountLabel = `${members.length} member${members.length === 1 ? "" : "s"}`;
  const scopeLabel = directReportsOnly
    ? `Direct reports of ${teamName}`
    : `${teamName}'s department`;

  return (
    <div className="flex flex-col">
      <DashboardHeader
        title={teamName ? `Team of ${teamName}` : ""}
        subtitle={
          canScopeToDirectReports
            ? `${scopeLabel} · ${memberCountLabel}`
            : memberCountLabel
        }
        person={teamId}
        hasReports
        actions={
          canScopeToDirectReports && fullRoster ? (
            <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground select-none">
              <Switch
                checked={directReportsOnly}
                onCheckedChange={setDirectReportsOnly}
              />
              <span>Direct reports only</span>
              <span className="text-xs text-muted-foreground">
                ({roster?.length ?? 0}/{fullRoster.length})
              </span>
            </label>
          ) : null
        }
      />
      <main className="flex flex-1 flex-col gap-8 p-4 md:p-6">
        {visibleRoster.isError || pivotMissing ? (
          <ComingSoon
            variant="card"
            state="error"
            label={
              pivotMissing
                ? "This person is not available"
                : "Unable to load this person"
            }
            onRetry={pivotMissing ? undefined : visibleRoster.retry}
          />
        ) : isLoading ? (
          <CenteredSpinner className="min-h-[70vh]" />
        ) : isAllEmpty ? (
          <DashboardEmptyState period={period} onSetPeriod={setPeriod} />
        ) : (
          <div className="flex flex-col gap-8">
            <TeamMembersAttention
              members={members}
              metricBelowByMember={metricBelowByMember}
              metricEntriesByPerson={metricEntriesByPerson}
            />

            {heatmapQ.isError ? (
              <ComingSoon
                state="error"
                label="Heatmap — unable to load"
                onRetry={() => heatmapQ.refetch()}
              />
            ) : (
              <MembersOverview
                members={members}
                heatmapByKey={heatmapQ.byKey}
                previousHeatmapByKey={heatmapQ.previousByKey}
                metricBelowByMember={metricBelowByMember}
                metricEntriesByPerson={metricEntriesByPerson}
              />
            )}

            <section className="flex flex-col gap-3">
              <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
                Sections
              </p>
              <div className="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-4">
                {GROUPS.map((def) => {
                  const result = metricGroupData.get(def.id);
                  if (!result) return null;
                  return (
                    <TeamMetricGroupCard
                      key={def.id}
                      def={def}
                      data={result}
                      memberIds={memberEntityIds}
                      onOpen={() => openDetails(def.id)}
                      subtitle="vs department peers"
                    />
                  );
                })}
              </div>
            </section>
          </div>
        )}
      </main>

      {GROUPS.map((def) => (
        <GroupDrilldownSheet
          key={def.id}
          open={detailsOpen && openGroup === def.id}
          onOpenChange={setDetailsOpen}
          onOpenChangeComplete={(open) => {
            if (!open && openGroup === def.id) setOpenGroup(null);
          }}
          def={def}
          metricTarget={{ kind: "team", members: memberRefs }}
          range={dateRange}
          period={period}
          cohortLabel="department"
        />
      ))}
    </div>
  );
}
