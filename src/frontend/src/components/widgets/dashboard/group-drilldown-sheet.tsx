import { Maximize2, Minimize2, XIcon } from "lucide-react";

import { ComingSoon } from "@/components/widgets/coming-soon";
import { CollectionDrilldown } from "@/components/widgets/metric-views/collection-drilldown";
import {
  TeamCollectionDrilldown,
  type TeamMemberRef,
} from "@/components/widgets/metric-views/team-collection-drilldown";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import type { DateRange } from "@/api/period-to-date-range";
import type { MetricGroup } from "@/lib/insight/groups";
import type { PeerCohortLabel } from "@/lib/peers";
import type { MetricCollectionResult } from "@/queries/metric-results";
import { cn } from "@/lib/utils";
import {
  parseLocalStorageBoolean,
  serializeLocalStorageBoolean,
  useLocalStorageState,
} from "@/hooks/use-local-storage-state";
import type { PeriodValue } from "@/types/insight";

/** Data target for a metrics-backed group's drilldown body. */
export type MetricDrilldownTarget =
  | {
      kind: "person";
      entityId: string;
      data: MetricCollectionResult;
    }
  | { kind: "team"; members: TeamMemberRef[] };

export interface GroupDrilldownSheetProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onOpenChangeComplete?: (open: boolean) => void;
  def: MetricGroup;
  metricTarget?: MetricDrilldownTarget;
  range?: DateRange;
  period?: PeriodValue;
  cohortLabel?: PeerCohortLabel;
}

export function GroupDrilldownSheet({
  open,
  onOpenChange,
  onOpenChangeComplete,
  def,
  metricTarget,
  range,
  period,
  cohortLabel = "department",
}: GroupDrilldownSheetProps) {
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      onOpenChangeComplete={onOpenChangeComplete}
    >
      <DialogContent
        showCloseButton={false}
        className="flex w-fit max-w-none! flex-col gap-0 overflow-hidden p-0"
      >
        <DrilldownPanel
          def={def}
          metricTarget={metricTarget}
          range={range}
          period={period}
          cohortLabel={cohortLabel}
        />
      </DialogContent>
    </Dialog>
  );
}

function DrilldownPanel({
  def,
  metricTarget,
  range,
  period,
  cohortLabel,
}: {
  def: MetricGroup;
  metricTarget?: MetricDrilldownTarget;
  range?: DateRange;
  period?: PeriodValue;
  cohortLabel: PeerCohortLabel;
}) {
  const [expanded, setExpanded] = useLocalStorageState<boolean>({
    key: "insight.drilldown.expanded",
    defaultValue: false,
    parse: parseLocalStorageBoolean,
    serialize: serializeLocalStorageBoolean,
  });

  return (
    <div
      className={cn(
        "flex min-h-0 flex-col overflow-hidden",
        expanded ? "h-[95vh] w-[95vw]" : "h-[70vh] w-[80vw]"
      )}
    >
      <DialogHeader className="flex shrink-0 flex-row items-center justify-between gap-2 border-b p-4">
        <DialogTitle>{def.title}</DialogTitle>
        <div className="flex items-center gap-0.5">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => setExpanded((v) => !v)}
            aria-label={expanded ? "Shrink" : "Expand"}
          >
            {expanded ? <Minimize2 /> : <Maximize2 />}
          </Button>
          <DialogClose
            render={
              <Button variant="ghost" size="icon-sm" aria-label="Close" />
            }
          >
            <XIcon />
          </DialogClose>
        </div>
      </DialogHeader>
      <div className="no-scrollbar min-h-0 flex-1 overflow-y-auto">
        {metricTarget?.kind === "person" ? (
          <CollectionDrilldown
            def={def}
            data={metricTarget.data}
            entityId={metricTarget.entityId}
            range={range}
            cohortLabel={cohortLabel}
          />
        ) : metricTarget?.kind === "team" && range && period ? (
          <TeamCollectionDrilldown
            def={def}
            members={metricTarget.members}
            range={range}
            period={period}
            cohortLabel={cohortLabel}
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center p-10">
            <ComingSoon state="error" label="Missing drilldown data" />
          </div>
        )}
      </div>
    </div>
  );
}
