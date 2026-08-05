import { useMemo } from "react";

import type { DateRange } from "@/api/period-to-date-range";
import { Card, CardContent } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import {
  MembersGrid,
  type MembersGridMember,
} from "@/components/widgets/dashboard/members-grid";
import type { MetricGroup } from "@/lib/insight/groups";
import type { PeerCohortLabel } from "@/lib/peers";
import { useMemberGridData } from "@/queries/member-grid";
import type { PeriodValue } from "@/types/insight";
import { cn } from "@/lib/utils";

export interface TeamMemberRef {
  entityId: string;
  displayName: string;
}

export interface TeamCollectionDrilldownProps {
  def: MetricGroup;
  members: TeamMemberRef[];
  range: DateRange;
  period: PeriodValue;
  cohortLabel?: PeerCohortLabel;
  className?: string;
}

/**
 * Drilldown for a metrics-backed group over a roster of people: the members
 * grid scoped to the group's full metric collection. Every cell is a
 * member's own value, trend, and standing vs their OWN department cohort —
 * nothing is pooled; the group card's judgment surfaces stay on the
 * dashboard.
 */
export function TeamCollectionDrilldown({
  def,
  members,
  range,
  period,
  cohortLabel = "department",
  className,
}: TeamCollectionDrilldownProps) {
  const gridMembers = useMemo<MembersGridMember[]>(
    () =>
      members.map((member) => ({
        entityId: member.entityId,
        displayName: member.displayName,
      })),
    [members]
  );
  const metricKeys = useMemo(
    () => def.collection.metrics.map((metric) => metric.key),
    [def]
  );
  const data = useMemberGridData(
    def.collection,
    { type: "person", ids: gridMembers.map((member) => member.entityId) },
    range,
    period
  );

  if (members.length === 0) {
    return (
      <p className="py-6 text-center text-sm text-muted-foreground">
        No team members to display.
      </p>
    );
  }
  if (data.isPending) {
    return (
      <div className="flex h-full min-h-96 w-full items-center justify-center p-10">
        <Spinner className="size-12 text-muted-foreground" />
      </div>
    );
  }
  if (data.isError) {
    return (
      <div className="flex h-full min-h-96 w-full items-center justify-center p-10">
        <ComingSoon
          state="error"
          label="Unable to load metrics"
          onRetry={data.refetch}
        />
      </div>
    );
  }

  return (
    <div
      className={cn(
        "p-4 transition-opacity sm:p-6",
        data.isFetching && "opacity-60",
        className
      )}
    >
      <Card>
        <CardContent>
          <MembersGrid
            members={gridMembers}
            metricKeys={metricKeys}
            byKey={data.byKey}
            previousByKey={data.previousByKey}
            showIssues
            caption={`${def.title} metrics for each team member vs their own ${cohortLabel} peers`}
            cohortLabel={cohortLabel}
          />
        </CardContent>
      </Card>
    </div>
  );
}
