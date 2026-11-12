import { useId, useState } from "react";

import type {
  ConnectorInstanceRef,
  SyncFact,
} from "@/api/connector-health-client";
import { Badge, Button } from "@gears-frontx/ui-kit";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { TONE_BAR_STYLE, TONE_STYLE } from "./connector-health-tone";
import { ConnectorScheduleTooltip } from "./connector-schedule-tooltip";
import {
  UNMEASURED,
  describeDuration,
  describeSync,
  formatRecords,
  formatStarted,
} from "@/lib/portal/connector-health";
import { cn } from "@/lib/utils";
import { useConnectorSyncs } from "@/queries/connector-health";

const INITIAL_ROWS = 10;
const MAX_RUNS = 50;

function statusTotals(syncs: readonly SyncFact[]) {
  const finished = syncs.filter((sync) =>
    ["succeeded", "failed", "cancelled", "incomplete"].includes(sync.status)
  );
  if (!finished.length) return null;

  const succeeded = finished.filter(
    (sync) => sync.status === "succeeded"
  ).length;
  const percent = Math.round((succeeded / finished.length) * 100);
  return {
    share: `${percent}% succeeded`,
    counts: `${succeeded} of ${finished.length} finished syncs succeeded, ${finished.length - succeeded} failed`,
  };
}

export function ConnectorSyncHistory({
  instance,
  asOf,
}: {
  instance: ConnectorInstanceRef;
  asOf: string;
}) {
  const { data, isPending, isError, refetch } = useConnectorSyncs(instance);
  const [showAll, setShowAll] = useState(false);
  const id = useId();

  if (isPending) return <CenteredSpinner className="min-h-24" />;
  if (isError || data === undefined) {
    return (
      <div className="max-w-md">
        <ComingSoon variant="card" state="error" onRetry={() => refetch()} />
      </div>
    );
  }

  const syncs = data.syncs.slice(0, MAX_RUNS);
  const totals = statusTotals(syncs);
  const visible = showAll ? syncs : syncs.slice(0, INITIAL_ROWS);
  const count =
    syncs.length > visible.length
      ? `Latest ${visible.length} of ${syncs.length} recorded`
      : `Latest ${syncs.length}`;

  return (
    <section
      aria-labelledby={`${id}-heading`}
      className="max-w-full min-w-0 space-y-3 [overflow-wrap:anywhere] whitespace-normal"
    >
      <div className="flex max-w-full min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        <h3 id={`${id}-heading`} className="shrink-0 text-sm font-semibold">
          Recent syncs
        </h3>
        {syncs.length > 0 && (
          <>
            <ol
              aria-label="Recorded sync statuses, newest on the right"
              className="flex max-w-full min-w-0 flex-wrap gap-[2px]"
            >
              {syncs
                .slice()
                .reverse()
                .map((sync, index) => {
                  const state = describeSync(sync);
                  const label = `Run ${syncs.length - index}: ${state.label}, started ${formatStarted(sync.started_at)}`;
                  return (
                    <li
                      key={sync.job_id}
                      aria-label={label}
                      title={label}
                      className={cn(
                        "h-[18px] w-[9px] shrink-0 rounded-[2px]",
                        TONE_BAR_STYLE[state.tone]
                      )}
                    />
                  );
                })}
            </ol>
            {totals && (
              <ConnectorScheduleTooltip lines={[totals.counts]}>
                <span className="text-xs text-muted-foreground">
                  {totals.share}
                </span>
              </ConnectorScheduleTooltip>
            )}
          </>
        )}
        <p className="ml-auto text-xs text-muted-foreground">{count}</p>
      </div>
      {syncs.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No sync has been recorded for this connector.
        </p>
      ) : (
        <>
          <table
            id={`${id}-table`}
            aria-label="Recent syncs"
            className="w-full max-w-full table-fixed text-xs sm:text-sm"
          >
            <colgroup>
              <col className="w-1/4" />
              <col className="w-[35%] sm:w-2/5" />
              <col className="w-1/5" />
              <col className="w-1/5 sm:w-[15%]" />
            </colgroup>
            <thead className="[overflow-wrap:anywhere] [word-break:normal] whitespace-normal">
              <tr className="border-b text-left text-muted-foreground">
                <th scope="col" className="px-1 py-2 font-medium">
                  Status
                </th>
                <th
                  scope="col"
                  className="px-1 py-2 font-medium whitespace-normal"
                >
                  Started (UTC)
                </th>
                <th scope="col" className="px-1 py-2 text-right font-medium">
                  Duration
                </th>
                <th scope="col" className="px-1 py-2 text-right font-medium">
                  Records
                </th>
              </tr>
            </thead>
            <tbody>
              {visible.map((sync) => (
                <SyncRow key={sync.job_id} sync={sync} asOf={asOf} />
              ))}
            </tbody>
          </table>
          {syncs.length > INITIAL_ROWS && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              aria-expanded={showAll}
              aria-controls={`${id}-table`}
              onClick={() => setShowAll(!showAll)}
            >
              {showAll ? "Show latest 10" : `Show all ${syncs.length}`}
            </Button>
          )}
        </>
      )}
    </section>
  );
}

function SyncRow({ sync, asOf }: { sync: SyncFact; asOf: string }) {
  const state = describeSync(sync);
  const duration = describeDuration(sync, asOf);
  return (
    <tr className="border-b last:border-0">
      <td className="px-1 py-2 align-top">
        <Badge
          className={cn(
            "max-w-full font-medium whitespace-normal",
            TONE_STYLE[state.tone]
          )}
        >
          {state.label}
        </Badge>
      </td>
      <td className="px-1 py-2 align-top text-muted-foreground tabular-nums">
        {formatStarted(sync.started_at)}
      </td>
      <td
        className={cn(
          "px-1 py-2 text-right align-top tabular-nums",
          duration.inFlight ? "text-foreground" : "text-muted-foreground"
        )}
      >
        {duration.text}
      </td>
      <td className="px-1 py-2 text-right align-top text-muted-foreground tabular-nums">
        {formatRecords(sync.records_reported)}
        {sync.records_reported == null && (
          <span className="sr-only">
            {UNMEASURED} means the mover reported no count, not a count of zero
          </span>
        )}
      </td>
    </tr>
  );
}
