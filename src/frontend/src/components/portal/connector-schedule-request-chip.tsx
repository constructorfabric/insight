import type { ConnectorHealth } from "@/api/connector-health-client";
import { ConnectorScheduleTooltip } from "@/components/portal/connector-schedule-tooltip";
import { Badge } from "@gears-frontx/ui-kit";
import { localTime } from "@/lib/portal/connector-schedule-editor";
import { scheduleTooltipLines } from "@/lib/portal/connector-schedule-tooltip";

export function ConnectorScheduleRequestChip({
  row,
}: {
  row: ConnectorHealth;
}) {
  const request = row.schedule_request;
  const visible = request?.status === "pending" || request?.status === "error";
  const chip = (
    <Badge
      variant="secondary"
      title={
        request?.status === "error" && request.error
          ? `${request.error.message} · Attempted at ${localTime(request.error.attempted_at)}`
          : undefined
      }
      aria-hidden={!visible}
      className={`block w-fit text-xs ${request?.status === "error" ? "bg-warning/15 text-warning" : "bg-primary/15 text-primary"} ${visible ? "cursor-help" : "invisible"}`}
    >
      {request?.status === "pending"
        ? "change pending"
        : request?.status === "error"
          ? "Error"
          : null}
    </Badge>
  );
  if (!visible) return chip;

  return (
    <ConnectorScheduleTooltip
      lines={scheduleTooltipLines(request ?? undefined)}
    >
      {chip}
    </ConnectorScheduleTooltip>
  );
}
