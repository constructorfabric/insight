import type {
  ConnectorSchedule,
  InheritedSchedule,
} from "@/api/connector-health-client";
import { scheduleWords } from "./connector-schedule-editor";
import { scheduleClock, scheduleRuns } from "./connector-schedule-time";

export function conciseScheduleWords(crons: string[]): string {
  const words = (scheduleWords(crons) ?? "Custom schedule")
    .replace(/^Syncs /, "")
    .replace(/ \(all UTC\)\.?$/, "")
    .replace(/ UTC\.?$/, "")
    .replace(/\.+$/, "");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function deploymentScheduleLines(
  inherited: InheritedSchedule | null | undefined
): string[] {
  if (!inherited) return ["The deployment schedule has not been recorded yet."];
  const source =
    inherited.origin === "annotation"
      ? "the Secret annotation"
      : inherited.origin === "descriptor"
        ? "the connector descriptor"
        : inherited.origin === "default"
          ? "the default"
          : inherited.origin;
  return [
    `${conciseScheduleWords(inherited.crons)} UTC`,
    ...inherited.crons.map((cron) => `${cron} UTC`),
    `From ${source}`,
  ];
}

export function scheduleIsOverdue(
  schedule: ConnectorSchedule,
  fallbackInterval?: number | null,
  now = new Date()
): boolean {
  const interval = schedule.reconcile_interval_secs ?? fallbackInterval;
  return (
    schedule.status === "pending" &&
    schedule.requested != null &&
    interval != null &&
    interval > 0 &&
    now.getTime() - new Date(schedule.requested.updated_at).getTime() >=
      interval * 2000
  );
}

export function scheduleTransitionMessage(
  schedule: ConnectorSchedule,
  now = new Date()
): string {
  if (schedule.status !== "applied") return "";
  const live = schedule.live;
  const next =
    live?.timezone && !live.suspended
      ? scheduleRuns(live.crons, live.timezone, now)[0]
      : null;
  return next
    ? `Schedule applied · next sync ${scheduleClock(next)} UTC`
    : "Schedule applied";
}
