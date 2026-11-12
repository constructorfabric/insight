import type { ReactNode } from "react";

import type { ConnectorSchedule } from "@/api/connector-health-client";
import { ConnectorScheduleTooltip } from "@/components/portal/connector-schedule-tooltip";
import { Badge } from "@gears-frontx/ui-kit";
import { scheduleStatusView } from "@/lib/portal/connector-schedule-status";
import { scheduleTooltipLines } from "@/lib/portal/connector-schedule-tooltip";
import { useConnectorScheduleAuthor } from "@/queries/connector-schedule-author";

export function ConnectorScheduleHeader({
  schedule,
  loading = false,
  children,
}: {
  schedule: ConnectorSchedule | undefined;
  loading?: boolean;
  children?: ReactNode;
}) {
  const person = useConnectorScheduleAuthor(schedule?.requested?.updated_by);
  const { label, Icon, tone, iconClass } = scheduleStatusView(
    schedule?.status,
    schedule?.pending_reason
  );
  return (
    <header className="flex min-w-0 flex-wrap items-center gap-2">
      <h3 className="shrink-0 text-sm font-medium">Schedule</h3>
      <div
        data-slot="schedule-header-status"
        className="flex min-w-0 items-center"
      >
        {loading ? (
          <span className="text-xs text-muted-foreground">Loading…</span>
        ) : (
          <ConnectorScheduleTooltip
            lines={scheduleTooltipLines(schedule, person)}
          >
            <Badge
              variant="secondary"
              title={label}
              className={`max-w-full min-w-0 gap-1 text-xs whitespace-nowrap ${tone}`}
            >
              <Icon aria-hidden="true" className={iconClass} />
              <span className="truncate">{label}</span>
            </Badge>
          </ConnectorScheduleTooltip>
        )}
      </div>
      {children}
    </header>
  );
}
