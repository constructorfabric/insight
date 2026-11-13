import { ConnectorScheduleTooltip } from "./connector-schedule-tooltip";
import {
  relativeScheduleTime,
  scheduleRuns,
  scheduleTime,
  shortScheduleTime,
} from "@/lib/portal/connector-schedule-time";

export function ConnectorScheduleRuns({
  crons,
  timezone = "UTC",
  suspended = false,
}: {
  crons: string[];
  timezone?: string;
  suspended?: boolean;
}) {
  const now = new Date();
  const runs = suspended ? [] : scheduleRuns(crons, timezone, now);
  if (suspended)
    return <span className="text-xs text-muted-foreground">Suspended</span>;
  if (!runs.length) return null;
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-2 text-xs">
      <span className="text-muted-foreground">Next (UTC)</span>
      <ol className="flex min-w-0 flex-wrap gap-2">
        {runs.map((run) => (
          <li key={run.toISOString()} className="max-w-full min-w-0">
            <ConnectorScheduleTooltip
              lines={[scheduleTime(run), relativeScheduleTime(run, now)]}
            >
              <time
                dateTime={run.toISOString()}
                className="rounded-md border px-2 py-1 tabular-nums"
              >
                {shortScheduleTime(run, now)}
              </time>
            </ConnectorScheduleTooltip>
          </li>
        ))}
      </ol>
    </div>
  );
}
