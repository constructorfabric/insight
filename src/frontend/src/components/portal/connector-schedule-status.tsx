import type { ConnectorSchedule } from "@/api/connector-health-client";
import { scheduleClock } from "@/lib/portal/connector-schedule-time";
import {
  conciseScheduleWords,
  scheduleIsOverdue,
} from "@/lib/portal/connector-schedule-display";

export function ConnectorScheduleStatus({
  schedule,
  reconcileIntervalSecs,
}: {
  schedule: ConnectorSchedule;
  reconcileIntervalSecs?: number | null;
}) {
  if (schedule.read_error)
    return <p className="text-sm text-warning">{schedule.read_error}</p>;
  if (schedule.status !== "pending") return null;
  const overdue = scheduleIsOverdue(schedule, reconcileIntervalSecs);
  const hint = schedule.next_reconcile_at
    ? `Next reconcile expected around ${scheduleClock(schedule.next_reconcile_at)} UTC`
    : "Takes effect at the next reconcile pass";
  const live = schedule.live;
  const running = live
    ? `until then runs ${live.timezone === "UTC" ? conciseScheduleWords(live.crons).toLowerCase() + " UTC" : live.crons.join("; ") + " (" + (live.timezone ?? "controller time") + ")"}${live.suspended ? " · suspended" : ""}.`
    : "No sync job runs until then.";
  return (
    <p
      data-slot="schedule-hint"
      className={`text-sm ${overdue ? "text-warning" : "text-muted-foreground"}`}
    >
      <span>{hint}</span>. {running}
      {overdue
        ? " Not applied after two reconcile passes. Ask an operator."
        : ""}
    </p>
  );
}
