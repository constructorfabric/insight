import type {
  ConnectorSchedule,
  InheritedSchedule,
  ScheduleRequestState,
} from "@/api/connector-health-client";
import { formatStarted } from "@/lib/portal/connector-health";
import { isScheduleOverride } from "@/lib/portal/connector-schedule";

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
  schedule: ConnectorSchedule | ScheduleRequestState | undefined
): string[] {
  if (!schedule) return ["The schedule has not been loaded yet."];

  const { applied, inherited, revision } = schedule;
  const older = applied && applied.revision < revision ? applied : null;
  switch (schedule.status) {
    case "inherited":
      return deploymentLines(inherited, "No schedule request. Runs");
    case "pending": {
      const requested = "crons" in schedule ? schedule : schedule.requested;
      const attribution = isScheduleOverride(requested)
        ? `, saved by ${requested.updated_by ?? "—"} at ${formatStarted(requested.updated_at)}`
        : "";
      return [
        `Request revision ${revision}${attribution}.`,
        ...(older
          ? [
              `Revision ${older.revision} is running now.`,
              ...older.crons.map((cron) => `${cron} UTC`),
            ]
          : []),
      ];
    }
    case "applied":
      return [
        ...(applied
          ? [
              `Request revision ${revision} is running, checked at ${formatStarted(applied.observed_at)}.`,
              ...applied.crons.map((cron) => `${cron} UTC`),
            ]
          : ["The applied schedule has not been recorded yet."]),
        ...deploymentLines(inherited, "Overrides"),
      ];
    case "error":
      return [
        schedule.error
          ? `Applying request revision ${schedule.error.attempted_revision} failed at ${formatStarted(schedule.error.attempted_at)}: ${schedule.error.message}`
          : "The failed schedule attempt has not been recorded yet.",
        ...(older ? [`Revision ${older.revision} is running.`] : []),
      ];
  }
}
