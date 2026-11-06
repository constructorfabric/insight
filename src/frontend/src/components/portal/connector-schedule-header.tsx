import type { ConnectorSchedule } from "@/api/connector-health-client";
import { ConnectorScheduleTooltip } from "@/components/portal/connector-schedule-tooltip";
import { Badge } from "@gears-frontx/ui-kit";
import { scheduleTooltipLines } from "@/lib/portal/connector-schedule-tooltip";

export function ConnectorScheduleHeader({
  schedule,
}: {
  schedule: ConnectorSchedule | undefined;
}) {
  const status = schedule?.status;
  const label =
    status === "pending"
      ? "● Pending"
      : status === "error"
        ? "Error"
        : status === "applied"
          ? "Applied"
          : status === "inherited"
            ? "Inherited"
            : "Default schedule";
  return (
    <div className="flex min-w-0 items-center justify-between gap-2">
      <h3 className="shrink-0 text-sm font-medium">Schedule</h3>
      <div
        data-slot="schedule-header-status"
        className="flex min-w-0 items-center justify-end"
      >
        <ConnectorScheduleTooltip lines={scheduleTooltipLines(schedule)}>
          <Badge
            variant="secondary"
            title={label}
            className={`max-w-full min-w-0 text-right text-xs whitespace-nowrap ${status === "error" ? "bg-warning/15 text-warning" : status === "pending" ? "bg-primary/10 text-primary" : "text-muted-foreground"}`}
          >
            <span className="truncate">{label}</span>
          </Badge>
        </ConnectorScheduleTooltip>
      </div>
    </div>
  );
}
