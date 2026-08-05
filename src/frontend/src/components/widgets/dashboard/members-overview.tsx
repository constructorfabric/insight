import { useMemo } from "react";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  MembersGrid,
  type MembersGridMember,
} from "@/components/widgets/dashboard/members-grid";
import { TriageList, type TriageRow } from "@/components/widgets/dashboard/triage-list";
import { HEATMAP_METRIC_KEYS } from "@/lib/insight/groups";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import { worstEntry, type PeerStoryEntry } from "@/lib/metrics/peer-story";
import type { PeerCohortLabel } from "@/lib/peers";
import { rankCounts, type RankCounts } from "@/lib/scoring";
import type { TeamMember } from "@/types/insight";

export interface MembersOverviewProps {
  members: TeamMember[];
  /** Current-period value + peer standing per cross-family metric key. */
  heatmapByKey: Map<string, NormalizedMetricResult>;
  /** Previous-period value per cross-family metric key (period view only). */
  previousHeatmapByKey: Map<string, NormalizedMetricResult>;
  /**
   * Per-member below-peer counts across ALL group collections, keyed by
   * normalized person id — the full standing, not just the grid's columns.
   */
  metricBelowByMember: Map<string, number>;
  /**
   * Per-person peer-story entries across all groups, keyed by normalized
   * person id; each entry's standing is vs the person's own org unit.
   */
  metricEntriesByPerson: Map<string, PeerStoryEntry[]>;
  /** Names the peer cohort in the subtitle and cell popovers. */
  cohortLabel?: PeerCohortLabel;
}

/**
 * The dashboard's members × cross-family-metrics overview: the members grid
 * on larger screens, a compact triage list on mobile.
 */
export function MembersOverview({
  members,
  heatmapByKey,
  previousHeatmapByKey,
  metricBelowByMember,
  metricEntriesByPerson,
  cohortLabel = "department",
}: MembersOverviewProps) {
  const gridMembers = useMemo<MembersGridMember[]>(
    () =>
      members.map((member) => ({
        entityId: normalizePersonId(member.person_id),
        displayName: member.name,
      })),
    [members],
  );

  const worstByMember = useMemo(() => {
    const worst = new Map<string, string | null>();
    for (const member of gridMembers) {
      const entries = metricEntriesByPerson.get(member.entityId) ?? [];
      worst.set(member.entityId, worstEntry(entries)?.label ?? null);
    }
    return worst;
  }, [gridMembers, metricEntriesByPerson]);

  // Cross-group standing counts per member — the chip judges the FULL
  // standing (every group's metrics), not just the grid's columns.
  const countsByMember = useMemo(() => {
    const counts = new Map<string, RankCounts>();
    for (const member of gridMembers) {
      const entries = metricEntriesByPerson.get(member.entityId) ?? [];
      counts.set(
        member.entityId,
        rankCounts(entries.map((entry) => ({ row: entry, rank: entry.status }))),
      );
    }
    return counts;
  }, [gridMembers, metricEntriesByPerson]);

  const triageRows: TriageRow[] = useMemo(
    () =>
      members.map((member) => {
        const entityId = normalizePersonId(member.person_id);
        const entries = metricEntriesByPerson.get(entityId) ?? [];
        return {
          member,
          belowCount: metricBelowByMember.get(entityId) ?? 0,
          topCount: entries.filter((entry) => entry.status === "top").length,
          worstMetricLabel: worstEntry(entries)?.label ?? null,
        };
      }),
    [members, metricBelowByMember, metricEntriesByPerson],
  );

  return (
    <Card>
      <CardHeader className="gap-1">
        <CardTitle>Members × metrics</CardTitle>
        <p className="text-xs text-muted-foreground">
          {members.length} members · cell colour = position vs {cohortLabel}{" "}
          peers
        </p>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <div className="sm:hidden">
          <TriageList rows={triageRows} />
        </div>
        <div className="hidden sm:block">
          <MembersGrid
            members={gridMembers}
            metricKeys={HEATMAP_METRIC_KEYS}
            byKey={heatmapByKey}
            previousByKey={previousHeatmapByKey}
            countsByMember={countsByMember}
            worstByMember={worstByMember}
            caption={`Team members compared to their own ${cohortLabel} peers across cross-family metrics`}
            cohortLabel={cohortLabel}
          />
        </div>
      </CardContent>
    </Card>
  );
}
