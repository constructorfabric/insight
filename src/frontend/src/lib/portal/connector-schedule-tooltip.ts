import type {
  ConnectorSchedule,
  InheritedSchedule,
  SchedulePendingReason,
  ScheduleRequestState,
} from "@/api/connector-health-client";
import { isScheduleOverride } from "@/lib/portal/connector-schedule";
import { scheduleStatusView } from "@/lib/portal/connector-schedule-status";
import { scheduleTime } from "./connector-schedule-time";

export function schedulePendingWords(
  reason: SchedulePendingReason | null | undefined
): string | null {
  switch (reason) {
    case "no_cronworkflow":
      return "This instance has no sync schedule yet";
    case "awaiting_reconcile":
      return "The sync job is not yet prepared";
    case "different_schedule":
      return "The sync job runs a different schedule";
    case null:
    case undefined:
      return null;
  }
}

function deploymentLines(
  inherited: InheritedSchedule | null | undefined,
  prefix: string
): string[] {
  if (!inherited) return ["The deployment schedule has not been recorded yet."];

  const source =
    inherited.origin === "annotation"
      ? "the Secret annotation insight.cyberfabric.com/schedule"
      : inherited.origin === "descriptor"
        ? "the connector descriptor"
        : inherited.origin === "default"
          ? "the default"
          : inherited.origin;
  return [
    `${prefix} the deployment schedule from ${source}.`,
    ...inherited.crons.map((cron) => `${cron} UTC`),
  ];
}

export function scheduleTooltipLines(
  schedule: ConnectorSchedule | ScheduleRequestState | undefined,
  person = "another admin"
): string[] {
  if (!schedule) return ["The schedule has not been loaded yet."];

  if (schedule.read_error) return [schedule.read_error];

  if (!("status" in schedule)) {
    const observation = schedule.last_observation;
    if (!observation) return ["The schedule has not been loaded yet."];

    const { label } = scheduleStatusView(observation.status);
    return [`${label}. Schedule details are loading.`];
  }

  if (schedule.read_error) return [schedule.read_error];

  const { live, inherited, revision } = schedule;
  const running = live
    ? [
        `Runs now, checked at ${scheduleTime(live.observed_at)}.`,
        ...live.crons.map(
          (cron) => `${cron} ${live.timezone ?? "controller time"}`
        ),
      ]
    : [];
  switch (schedule.status) {
    case "paused":
      return ["The sync job is paused. No sync runs are scheduled."];
    case "inherited":
      return deploymentLines(inherited, "No schedule request. Runs");
    case "pending": {
      const requested = schedule.requested;
      const attribution = isScheduleOverride(requested)
        ? `, saved by ${person} at ${scheduleTime(requested.updated_at)}`
        : "";
      const reason = schedulePendingWords(schedule.pending_reason);
      return [
        `Request revision ${revision}${attribution}.`,
        ...(reason ? [`${reason}.`] : []),
        ...(schedule.next_reconcile_at
          ? [`Next reconcile: ${scheduleTime(schedule.next_reconcile_at)}`]
          : []),
        ...(requested && !isScheduleOverride(requested)
          ? deploymentLines(inherited, "Returns to")
          : []),
        ...running,
      ];
    }
    case "applied":
      return [
        ...(live
          ? [
              `Request revision ${revision} is running, checked at ${scheduleTime(live.observed_at)}.`,
              ...live.crons.map(
                (cron) => `${cron} ${live.timezone ?? "controller time"}`
              ),
            ]
          : ["The live schedule has not been recorded yet."]),
        ...deploymentLines(inherited, "Overrides"),
      ];
    case null:
      return ["The live sync job cannot be read."];
  }
}
